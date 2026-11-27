//! One-time migration of legacy TypeScript-signer data into the canonical
//! `requests`/`accounts` schema.
//!
//! The TS signer stored PNP data in `accountsOnChain` / `requestsOnChain` with
//! addresses in whatever casing clients supplied (Celo's `isValidAddress`
//! accepts any casing). The Rust signer canonicalizes every address to its
//! EIP-55 checksummed form, so a naive "point at the same DB" swap would miss
//! any account stored in a different casing and silently reset its quota.
//!
//! This migration runs once and writes normalized rows from the legacy tables
//! into the canonical tables, by these rules:
//! - addresses are re-checksummed (unparseable ones are skipped and counted),
//! - request rows with a NULL signature are dropped (the canonical schema
//!   requires a signature),
//! - accounts whose casings collapse to the same checksummed address have their
//!   `num_lookups` summed (never under-counting billed usage),
//! - requests that collapse to the same `(caller_address, blinded_query)` keep
//!   the row with the most recent timestamp,
//! - every request is imported with an unknown key version (`key_version` NULL):
//!   the TS signer did not store which key made a signature.
//!
//! The import first locks the legacy tables against writes, so that all it
//! reads sees the same rows. The checksum is the only step Postgres cannot
//! take, since it has no keccak, and it depends on the address alone. So the
//! import reads the distinct legacy addresses, checksums each one here, and
//! writes the pairs to a temporary table, `legacy_address`. Then one
//! `INSERT ... SELECT` per canonical table joins the legacy rows through that
//! mapping and applies the rules above. The number of round trips to the
//! database does not grow with the number of legacy rows.
//!
//! The whole import runs in a single transaction alongside a marker row, so a
//! failure rolls everything back and a completed import never re-runs.

use std::time::Instant;

use alloy::primitives::Address;
use sqlx::{PgPool, Postgres, Transaction};

use crate::errors::OdisError;

/// Stable key for the transaction-scoped advisory lock that serializes migration
/// attempts across concurrently-starting replicas.
const MIGRATION_LOCK_KEY: i64 = 0x0D15_EA5E_0000_0001;

/// The TS signer's tables as quoted identifiers, for the `LOCK TABLE`
/// statement, which names only the tables that exist.
const LEGACY_ACCOUNTS_TABLE: &str = "\"accountsOnChain\"";
const LEGACY_REQUESTS_TABLE: &str = "\"requestsOnChain\"";

/// Summary of what the legacy migration did, for logging and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegacyMigrationReport {
    /// A prior run already completed; nothing was done this time.
    pub already_migrated: bool,
    /// At least one legacy table was found and imported.
    pub legacy_tables_present: bool,
    /// Legacy account rows taken, before casings of one address are merged.
    pub accounts_imported: u64,
    pub accounts_skipped_unparseable: u64,
    /// Rows of `accounts` inserted or added to.
    pub accounts_written: u64,
    /// Legacy request rows taken, before duplicates are merged.
    pub requests_imported: u64,
    pub requests_skipped_null_signature: u64,
    pub requests_skipped_unparseable: u64,
    /// Rows of `requests` inserted or replaced.
    pub requests_written: u64,
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

    // A TS signer that still runs must wait for the import rather than write a
    // row past it: a caller that first appeared after the address mapping was
    // built would count as unparseable and lose its requests, and the marker
    // would prevent a retry. SHARE lets others read and blocks every insert,
    // update and delete until the transaction ends. The TS signer writes the
    // request before the account in one transaction, so the tables are locked
    // in that order; a TS transaction holding the one and waiting for the other
    // cannot then deadlock with the import.
    let present_tables: Vec<&str> = [
        (requests_present, LEGACY_REQUESTS_TABLE),
        (accounts_present, LEGACY_ACCOUNTS_TABLE),
    ]
    .into_iter()
    .filter_map(|(present, table)| present.then_some(table))
    .collect();
    let lock = format!("LOCK TABLE {} IN SHARE MODE", present_tables.join(", "));
    tracing::info!(
        "waiting for a lock on the legacy tables; open TS signer transactions on them delay the \
         import"
    );
    sqlx::query(&lock)
        .execute(&mut *tx)
        .await
        .map_err(db_err("lock legacy tables"))?;

    map_legacy_addresses(&mut tx, accounts_present, requests_present).await?;

    let mut report = LegacyMigrationReport {
        legacy_tables_present: true,
        ..Default::default()
    };

    if accounts_present {
        import_accounts(&mut tx, &mut report).await?;
    }
    if requests_present {
        import_requests(&mut tx, &mut report).await?;
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
        accounts_written = report.accounts_written,
        requests_imported = report.requests_imported,
        requests_skipped_null_signature = report.requests_skipped_null_signature,
        requests_skipped_unparseable = report.requests_skipped_unparseable,
        requests_written = report.requests_written,
        "legacy data migration complete"
    );

    Ok(report)
}

/// Fails when the database holds TS signer tables that were never imported.
///
/// Serving requests from such a database would start every account at a used
/// quota of zero, so the signer must not start until the import has run.
pub async fn ensure_legacy_data_imported(pool: &PgPool) -> Result<(), OdisError> {
    let (accounts_present, requests_present) = detect_legacy_tables(pool).await?;
    if !accounts_present && !requests_present {
        return Ok(());
    }

    let (marker_table,): (Option<String>,) =
        sqlx::query_as("SELECT to_regclass('legacy_migration')::text")
            .fetch_one(pool)
            .await
            .map_err(db_err("detect marker table"))?;
    if marker_table.is_some() {
        let marker: Option<(i32,)> = sqlx::query_as("SELECT id FROM legacy_migration WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(db_err("check migration marker"))?;
        if marker.is_some() {
            return Ok(());
        }
    }

    tracing::error!(
        "the database holds TS signer tables (accountsOnChain/requestsOnChain) that were never \
         imported; set MIGRATE_LEGACY_DATA=true to import them. Without the import every \
         account's used quota would start at zero. To start from zero on purpose, drop or \
         rename those tables."
    );
    Err(OdisError::DatabaseError)
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

/// Creates the temporary table `legacy_address`, which maps each distinct
/// address of the legacy tables to its checksummed form, and fills it.
///
/// An address that does not parse is logged and left out, so the imports do
/// not take its rows.
async fn map_legacy_addresses(
    tx: &mut Transaction<'_, Postgres>,
    accounts_present: bool,
    requests_present: bool,
) -> Result<(), OdisError> {
    sqlx::query(
        "CREATE TEMP TABLE legacy_address ( \
            raw         text PRIMARY KEY, \
            checksummed text NOT NULL \
        ) ON COMMIT DROP",
    )
    .execute(&mut **tx)
    .await
    .map_err(db_err("create address mapping"))?;

    // Requests without a signature are not imported, so their callers need no
    // mapping. UNION removes the duplicates of two tables; one table alone
    // needs DISTINCT, or a caller with two requests would break the primary key.
    let distinct_addresses = match (accounts_present, requests_present) {
        (true, true) => {
            "SELECT address FROM \"accountsOnChain\" \
             UNION \
             SELECT caller_address FROM \"requestsOnChain\" WHERE signature IS NOT NULL"
        }
        (true, false) => "SELECT DISTINCT address FROM \"accountsOnChain\"",
        (false, true) => {
            "SELECT DISTINCT caller_address FROM \"requestsOnChain\" WHERE signature IS NOT NULL"
        }
        (false, false) => return Ok(()),
    };
    let addresses: Vec<String> = sqlx::query_scalar(distinct_addresses)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err("read legacy addresses"))?;

    let mut raw = Vec::with_capacity(addresses.len());
    let mut checksummed = Vec::with_capacity(addresses.len());
    for address in addresses {
        match checksum(&address) {
            Some(canonical) => {
                raw.push(address);
                checksummed.push(canonical);
            }
            None => {
                tracing::warn!(address = %address, "skipping legacy rows with unparseable address")
            }
        }
    }
    if raw.is_empty() {
        return Ok(());
    }

    // Each array is one parameter, so the number of addresses is not bound by
    // Postgres' limit on the number of parameters.
    sqlx::query(
        "INSERT INTO legacy_address (raw, checksummed) \
         SELECT * FROM UNNEST($1::text[], $2::text[])",
    )
    .bind(raw)
    .bind(checksummed)
    .execute(&mut **tx)
    .await
    .map_err(db_err("write address mapping"))?;
    Ok(())
}

/// Imports `accountsOnChain` through the address mapping, in one statement.
async fn import_accounts(
    tx: &mut Transaction<'_, Postgres>,
    report: &mut LegacyMigrationReport,
) -> Result<(), OdisError> {
    let (taken, skipped_unparseable): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE m.raw IS NOT NULL), \
                count(*) FILTER (WHERE m.raw IS NULL) \
         FROM \"accountsOnChain\" a \
         LEFT JOIN legacy_address m ON m.raw = a.address",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err("count legacy accounts"))?;
    report.accounts_imported = taken as u64;
    report.accounts_skipped_unparseable = skipped_unparseable as u64;
    tracing::info!(legacy_rows = taken, "importing legacy accounts");

    // The casings of one address are summed, and the sum is added to a row the
    // Rust signer already wrote. A NULL count is 0 and a NULL creation time is
    // now. A sum beyond the `integer` range fails the import.
    let started = Instant::now();
    let written = sqlx::query(
        "INSERT INTO accounts (address, num_lookups, created_at) \
         SELECT m.checksummed, SUM(COALESCE(a.num_lookups, 0))::integer, \
                MIN(COALESCE(a.created_at, now())) \
         FROM \"accountsOnChain\" a \
         JOIN legacy_address m ON m.raw = a.address \
         GROUP BY m.checksummed \
         ON CONFLICT (address) DO UPDATE SET \
             num_lookups = accounts.num_lookups + EXCLUDED.num_lookups, \
             created_at = LEAST(accounts.created_at, EXCLUDED.created_at)",
    )
    .execute(&mut **tx)
    .await
    .map_err(db_err("import legacy accounts"))?
    .rows_affected();
    report.accounts_written = written;
    tracing::info!(
        rows_written = written,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "legacy accounts imported"
    );

    Ok(())
}

/// Imports `requestsOnChain` through the address mapping, in one statement.
async fn import_requests(
    tx: &mut Transaction<'_, Postgres>,
    report: &mut LegacyMigrationReport,
) -> Result<(), OdisError> {
    let (null_signature, taken, skipped_unparseable): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE r.signature IS NULL), \
                count(*) FILTER (WHERE r.signature IS NOT NULL AND m.raw IS NOT NULL), \
                count(*) FILTER (WHERE r.signature IS NOT NULL AND m.raw IS NULL) \
         FROM \"requestsOnChain\" r \
         LEFT JOIN legacy_address m ON m.raw = r.caller_address",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err("count legacy requests"))?;
    report.requests_skipped_null_signature = null_signature as u64;
    report.requests_imported = taken as u64;
    report.requests_skipped_unparseable = skipped_unparseable as u64;
    tracing::info!(legacy_rows = taken, "importing legacy requests");

    // Of the legacy rows for one account and query the latest is kept, and of
    // two with the same timestamp the one whose raw address sorts first. A row
    // the Rust signer already wrote is replaced only by a later one, and loses
    // its key version with its signature.
    let started = Instant::now();
    let written = sqlx::query(
        "INSERT INTO requests (caller_address, blinded_query, signature, \"timestamp\") \
         SELECT DISTINCT ON (m.checksummed, r.blinded_query) \
                m.checksummed, r.blinded_query, r.signature, r.\"timestamp\" \
         FROM \"requestsOnChain\" r \
         JOIN legacy_address m ON m.raw = r.caller_address \
         WHERE r.signature IS NOT NULL \
         ORDER BY m.checksummed, r.blinded_query, r.\"timestamp\" DESC, r.caller_address \
         ON CONFLICT (caller_address, blinded_query) DO UPDATE SET \
             signature = EXCLUDED.signature, \
             key_version = NULL, \
             \"timestamp\" = EXCLUDED.\"timestamp\" \
         WHERE EXCLUDED.\"timestamp\" > requests.\"timestamp\"",
    )
    .execute(&mut **tx)
    .await
    .map_err(db_err("import legacy requests"))?
    .rows_affected();
    report.requests_written = written;
    tracing::info!(
        rows_written = written,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "legacy requests imported"
    );

    Ok(())
}

/// Parse an address in any casing and return its EIP-55 checksummed form.
fn checksum(raw: &str) -> Option<String> {
    raw.parse::<Address>().ok().map(|a| a.to_string())
}

fn db_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> OdisError {
    move |e| {
        tracing::error!("legacy migration failed at {ctx}: {e}");
        OdisError::DatabaseError
    }
}
