//! One-time migration of legacy TypeScript-signer data into the canonical
//! `requests`/`accounts` schema.
//!
//! The TS signer stored PNP data in `accountsOnChain` / `requestsOnChain` with
//! addresses in whatever casing clients supplied (Celo's `isValidAddress`
//! accepts any casing). The Rust signer canonicalizes every address to its
//! EIP-55 checksummed form, so a naive "point at the same DB" swap would miss
//! any account stored in a different casing and silently reset its quota.
//!
//! This migration reads the legacy tables once and writes normalized rows into
//! the canonical tables, cleaning up as it goes:
//! - addresses are re-checksummed (unparseable ones are skipped and counted),
//! - request rows with a NULL signature are dropped (the canonical schema
//!   requires a signature),
//! - accounts whose casings collapse to the same checksummed address have their
//!   `num_lookups` summed (never under-counting billed usage),
//! - requests that collapse to the same `(caller_address, blinded_query)` keep
//!   the row with the most recent timestamp.
//!
//! The whole import runs in a single transaction alongside a marker row, so a
//! failure rolls everything back and a completed import never re-runs.

use alloy::primitives::Address;
use sqlx::{PgPool, Postgres, Transaction};

use crate::errors::OdisError;

/// Number of legacy rows read per page during the streaming import.
const PAGE_SIZE: i64 = 1000;

/// Stable key for the transaction-scoped advisory lock that serializes migration
/// attempts across concurrently-starting replicas.
const MIGRATION_LOCK_KEY: i64 = 0x0D15_EA5E_0000_0001;

/// Summary of what the legacy migration did, for logging and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegacyMigrationReport {
    /// A prior run already completed; nothing was done this time.
    pub already_migrated: bool,
    /// At least one legacy table was found and imported.
    pub legacy_tables_present: bool,
    pub accounts_imported: u64,
    pub accounts_skipped_unparseable: u64,
    pub requests_imported: u64,
    pub requests_skipped_null_signature: u64,
    pub requests_skipped_unparseable: u64,
}

/// Run the one-time legacy TS -> canonical data migration if it hasn't run yet.
///
/// The marker check, import, and marker write all happen inside a single
/// transaction that first takes a transaction-scoped advisory lock. This
/// serializes concurrently-starting replicas: the first to acquire the lock
/// runs the import, and any others block until it commits and then observe the
/// marker and return `already_migrated` instead of importing again. The lock is
/// released automatically on commit or rollback.
pub async fn run_legacy_migration(pool: &PgPool) -> Result<LegacyMigrationReport, OdisError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(db_err("begin migration transaction"))?;

    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(db_err("acquire advisory lock"))?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS legacy_migration ( \
            id                INT PRIMARY KEY, \
            completed_at      TIMESTAMPTZ NOT NULL, \
            accounts_imported BIGINT NOT NULL, \
            requests_imported BIGINT NOT NULL, \
            accounts_skipped  BIGINT NOT NULL, \
            requests_skipped  BIGINT NOT NULL \
        )",
    )
    .execute(&mut *tx)
    .await
    .map_err(db_err("create marker table"))?;

    let marker: Option<(i32,)> = sqlx::query_as("SELECT id FROM legacy_migration WHERE id = 1")
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err("check migration marker"))?;
    if marker.is_some() {
        tracing::info!("legacy data migration already completed; skipping");
        return Ok(LegacyMigrationReport {
            already_migrated: true,
            ..Default::default()
        });
    }

    let (accounts_present, requests_present) = detect_legacy_tables(pool).await?;
    if !accounts_present && !requests_present {
        tracing::info!(
            "no legacy tables (accountsOnChain/requestsOnChain) found; nothing to migrate"
        );
        return Ok(LegacyMigrationReport::default());
    }

    tracing::info!(
        accounts_present,
        requests_present,
        "starting one-time legacy data migration"
    );

    let mut report = LegacyMigrationReport {
        legacy_tables_present: true,
        ..Default::default()
    };

    if accounts_present {
        import_accounts(pool, &mut tx, &mut report).await?;
    }
    if requests_present {
        import_requests(pool, &mut tx, &mut report).await?;
    }

    sqlx::query(
        "INSERT INTO legacy_migration \
         (id, completed_at, accounts_imported, requests_imported, accounts_skipped, requests_skipped) \
         VALUES (1, now(), $1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
    )
    .bind(report.accounts_imported as i64)
    .bind(report.requests_imported as i64)
    .bind(report.accounts_skipped_unparseable as i64)
    .bind((report.requests_skipped_null_signature + report.requests_skipped_unparseable) as i64)
    .execute(&mut *tx)
    .await
    .map_err(db_err("write migration marker"))?;

    tx.commit()
        .await
        .map_err(db_err("commit migration transaction"))?;

    tracing::info!(
        accounts_imported = report.accounts_imported,
        accounts_skipped_unparseable = report.accounts_skipped_unparseable,
        requests_imported = report.requests_imported,
        requests_skipped_null_signature = report.requests_skipped_null_signature,
        requests_skipped_unparseable = report.requests_skipped_unparseable,
        "legacy data migration complete"
    );

    Ok(report)
}

/// Returns `(accountsOnChain_present, requestsOnChain_present)`.
async fn detect_legacy_tables(pool: &PgPool) -> Result<(bool, bool), OdisError> {
    let (accounts, requests): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT to_regclass('\"accountsOnChain\"')::text, to_regclass('\"requestsOnChain\"')::text",
    )
    .fetch_one(pool)
    .await
    .map_err(db_err("detect legacy tables"))?;
    Ok((accounts.is_some(), requests.is_some()))
}

async fn import_accounts(
    pool: &PgPool,
    tx: &mut Transaction<'_, Postgres>,
    report: &mut LegacyMigrationReport,
) -> Result<(), OdisError> {
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM \"accountsOnChain\"")
        .fetch_one(pool)
        .await
        .map_err(db_err("count legacy accounts"))?;
    tracing::info!(total, "importing legacy accounts");

    let mut processed: i64 = 0;
    let mut last_decile: i64 = 0;
    let mut cursor: Option<String> = None;

    loop {
        let page: Vec<(String, i32, Option<String>)> = match &cursor {
            Some(last) => {
                sqlx::query_as(
                    "SELECT address, COALESCE(num_lookups, 0), created_at::text \
                 FROM \"accountsOnChain\" WHERE address > $1 ORDER BY address LIMIT $2",
                )
                .bind(last)
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as(
                    "SELECT address, COALESCE(num_lookups, 0), created_at::text \
                 FROM \"accountsOnChain\" ORDER BY address LIMIT $1",
                )
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await
            }
        }
        .map_err(db_err("read legacy accounts page"))?;

        let Some(last_row) = page.last() else { break };
        cursor = Some(last_row.0.clone());

        for (raw_address, num_lookups, created_at) in page {
            processed += 1;
            let Some(checksummed) = checksum(&raw_address) else {
                report.accounts_skipped_unparseable += 1;
                tracing::warn!(address = %raw_address, "skipping account with unparseable address");
                continue;
            };

            sqlx::query(
                "INSERT INTO accounts (address, num_lookups, created_at) \
                 VALUES ($1, $2, COALESCE($3::timestamptz, now())) \
                 ON CONFLICT (address) DO UPDATE SET \
                     num_lookups = accounts.num_lookups + EXCLUDED.num_lookups, \
                     created_at = LEAST(accounts.created_at, EXCLUDED.created_at)",
            )
            .bind(&checksummed)
            .bind(num_lookups)
            .bind(&created_at)
            .execute(&mut **tx)
            .await
            .map_err(db_err("upsert canonical account"))?;
            report.accounts_imported += 1;
        }

        maybe_log_progress("accounts", processed, total, &mut last_decile);
    }

    Ok(())
}

async fn import_requests(
    pool: &PgPool,
    tx: &mut Transaction<'_, Postgres>,
    report: &mut LegacyMigrationReport,
) -> Result<(), OdisError> {
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM \"requestsOnChain\" WHERE signature IS NOT NULL")
            .fetch_one(pool)
            .await
            .map_err(db_err("count legacy requests"))?;
    let null_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM \"requestsOnChain\" WHERE signature IS NULL")
            .fetch_one(pool)
            .await
            .map_err(db_err("count null-signature requests"))?;
    report.requests_skipped_null_signature = null_count as u64;
    tracing::info!(
        total,
        skipped_null = null_count,
        "importing legacy requests"
    );

    let mut processed: i64 = 0;
    let mut last_decile: i64 = 0;
    // Keyset cursor over the full legacy PK (caller_address, blinded_query, timestamp)
    // so pages never skip rows that share a (caller_address, blinded_query).
    let mut cursor: Option<(String, String, String)> = None;

    loop {
        let page: Vec<(String, String, String, String)> = match &cursor {
            Some((ca, bq, ts)) => {
                sqlx::query_as(
                    "SELECT caller_address, blinded_query, signature, \"timestamp\"::text \
                 FROM \"requestsOnChain\" \
                 WHERE signature IS NOT NULL \
                   AND (caller_address, blinded_query, \"timestamp\") > ($1, $2, $3::timestamptz) \
                 ORDER BY caller_address, blinded_query, \"timestamp\" LIMIT $4",
                )
                .bind(ca)
                .bind(bq)
                .bind(ts)
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await
            }
            None => {
                sqlx::query_as(
                    "SELECT caller_address, blinded_query, signature, \"timestamp\"::text \
                 FROM \"requestsOnChain\" \
                 WHERE signature IS NOT NULL \
                 ORDER BY caller_address, blinded_query, \"timestamp\" LIMIT $1",
                )
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await
            }
        }
        .map_err(db_err("read legacy requests page"))?;

        let Some(last_row) = page.last() else { break };
        cursor = Some((last_row.0.clone(), last_row.1.clone(), last_row.3.clone()));

        for (raw_address, blinded_query, signature, timestamp) in page {
            processed += 1;
            let Some(checksummed) = checksum(&raw_address) else {
                report.requests_skipped_unparseable += 1;
                tracing::warn!(address = %raw_address, "skipping request with unparseable address");
                continue;
            };

            sqlx::query(
                "INSERT INTO requests (caller_address, blinded_query, signature, \"timestamp\") \
                 VALUES ($1, $2, $3, COALESCE($4::timestamptz, now())) \
                 ON CONFLICT (caller_address, blinded_query) DO UPDATE SET \
                     signature = EXCLUDED.signature, \"timestamp\" = EXCLUDED.\"timestamp\" \
                 WHERE EXCLUDED.\"timestamp\" > requests.\"timestamp\"",
            )
            .bind(&checksummed)
            .bind(&blinded_query)
            .bind(&signature)
            .bind(&timestamp)
            .execute(&mut **tx)
            .await
            .map_err(db_err("upsert canonical request"))?;
            report.requests_imported += 1;
        }

        maybe_log_progress("requests", processed, total, &mut last_decile);
    }

    Ok(())
}

/// Parse an address in any casing and return its EIP-55 checksummed form.
fn checksum(raw: &str) -> Option<String> {
    raw.parse::<Address>().ok().map(|a| a.to_string())
}

/// Log progress once per completed decile so large imports report smoothly
/// without a line per row.
fn maybe_log_progress(kind: &str, processed: i64, total: i64, last_decile: &mut i64) {
    if total <= 0 {
        return;
    }
    let decile = (processed * 10 / total).min(10);
    if decile > *last_decile {
        *last_decile = decile;
        tracing::info!(
            processed,
            total,
            percent = decile * 10,
            "legacy {kind} import progress"
        );
    }
}

fn db_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> OdisError {
    move |e| {
        tracing::error!("legacy migration failed at {ctx}: {e}");
        OdisError::DatabaseError
    }
}
