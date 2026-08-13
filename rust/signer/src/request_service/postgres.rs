use alloy::primitives::Address;
use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::legacy_migration::{
    LegacyMigrationReport, ensure_legacy_data_imported, run_legacy_migration,
};
use super::{PnpRequestService, RecordOutcome};
use crate::errors::OdisError;

/// Read an account's used quota inside an open transaction. Absent accounts
/// have used nothing.
async fn read_used_quota(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    address: &str,
) -> Result<u32, OdisError> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT num_lookups FROM accounts WHERE address = $1")
        .bind(address)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!("used quota lookup failed: {e}");
            OdisError::DatabaseError
        })?;
    Ok(row
        .map(|(n,)| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0))
}

/// PostgreSQL-backed [`PnpRequestService`], using the same canonical
/// `requests`/`accounts` schema as the SQLite backend.
pub struct PostgresPnpRequestService {
    pool: PgPool,
}

impl PostgresPnpRequestService {
    pub async fn new(url: &str) -> Result<Self, OdisError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .map_err(|e| {
                tracing::error!("failed to connect to Postgres: {e}");
                OdisError::DatabaseError
            })?;

        sqlx::migrate!("./migrations/postgres")
            .run(&pool)
            .await
            .map_err(|e| {
                tracing::error!("failed to run Postgres migrations: {e}");
                OdisError::DatabaseError
            })?;

        Ok(Self { pool })
    }

    /// Run the one-time legacy TS -> canonical data migration. Idempotent:
    /// guarded by a marker table so a completed import never re-runs.
    pub async fn run_legacy_migration(&self) -> Result<LegacyMigrationReport, OdisError> {
        run_legacy_migration(&self.pool).await
    }

    /// Fails when the database holds TS signer tables that were never imported.
    pub async fn ensure_legacy_data_imported(&self) -> Result<(), OdisError> {
        ensure_legacy_data_imported(&self.pool).await
    }
}

#[async_trait]
impl PnpRequestService for PostgresPnpRequestService {
    async fn get_used_quota(&self, address: Address) -> Result<u32, OdisError> {
        let addr = address.to_string();
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT num_lookups FROM accounts WHERE address = $1")
                .bind(&addr)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    tracing::error!("get_used_quota query failed: {e}");
                    OdisError::DatabaseError
                })?;
        Ok(row
            .map(|(n,)| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0))
    }

    async fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError> {
        let addr = address.to_string();
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT signature FROM requests WHERE caller_address = $1 AND blinded_query = $2 LIMIT 1",
        )
        .bind(&addr)
        .bind(blinded_query)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("get_duplicate_request query failed: {e}");
            OdisError::DatabaseError
        })?;
        Ok(row.map(|(sig,)| sig))
    }

    async fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
        total_quota: u32,
    ) -> Result<RecordOutcome, OdisError> {
        let addr = address.to_string();
        let quota = i32::try_from(total_quota).unwrap_or(i32::MAX);
        let mut tx = self.pool.begin().await.map_err(|e| {
            tracing::error!("failed to begin transaction: {e}");
            OdisError::DatabaseError
        })?;

        // Claim the (address, query) slot. A racing identical request blocks
        // here until this transaction resolves, then sees the conflict.
        let claimed = sqlx::query(
            "INSERT INTO requests (caller_address, blinded_query, signature) VALUES ($1, $2, $3) \
             ON CONFLICT (caller_address, blinded_query) DO NOTHING",
        )
        .bind(&addr)
        .bind(blinded_query)
        .bind(signature)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("insert request failed: {e}");
            OdisError::DatabaseError
        })?
        .rows_affected()
            == 1;

        if !claimed {
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT signature FROM requests WHERE caller_address = $1 AND blinded_query = $2",
            )
            .bind(&addr)
            .bind(blinded_query)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!("duplicate signature lookup failed: {e}");
                OdisError::DatabaseError
            })?;
            let used = read_used_quota(&mut tx, &addr).await?;
            tx.commit().await.map_err(|e| {
                tracing::error!("failed to commit transaction: {e}");
                OdisError::DatabaseError
            })?;

            let Some((existing_sig,)) = existing else {
                tracing::error!("request insert conflicted but no row was found");
                return Err(OdisError::DatabaseError);
            };
            return Ok(RecordOutcome::Duplicate {
                signature: existing_sig,
                performed_query_count: used,
            });
        }

        // Charge quota. `ON CONFLICT DO UPDATE ... WHERE` re-checks the bound
        // against the locked row, so concurrent requests serialize here instead
        // of both slipping past a stale read. No row comes back when the
        // account is already at its limit, or when the quota is zero and the
        // account has no row yet.
        let charged: Option<(i32,)> = sqlx::query_as(
            "INSERT INTO accounts (address, num_lookups) SELECT $1, 1 WHERE $2::int > 0 \
             ON CONFLICT (address) DO UPDATE SET num_lookups = accounts.num_lookups + 1 \
             WHERE accounts.num_lookups < $2::int \
             RETURNING num_lookups",
        )
        .bind(&addr)
        .bind(quota)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("upsert account failed: {e}");
            OdisError::DatabaseError
        })?;

        match charged {
            Some((num_lookups,)) => {
                tx.commit().await.map_err(|e| {
                    tracing::error!("failed to commit transaction: {e}");
                    OdisError::DatabaseError
                })?;
                Ok(RecordOutcome::Recorded {
                    performed_query_count: u32::try_from(num_lookups).unwrap_or(u32::MAX),
                })
            }
            None => {
                let used = read_used_quota(&mut tx, &addr).await?;
                // Discards the request row claimed above.
                tx.rollback().await.map_err(|e| {
                    tracing::error!("failed to roll back transaction: {e}");
                    OdisError::DatabaseError
                })?;
                Ok(RecordOutcome::QuotaExceeded {
                    performed_query_count: used,
                })
            }
        }
    }

    async fn delete_old_requests(&self, older_than_days: u64) -> Result<u64, OdisError> {
        let days = i32::try_from(older_than_days).unwrap_or(i32::MAX);
        let result = sqlx::query(
            "DELETE FROM requests WHERE \"timestamp\" <= now() - make_interval(days => $1::int)",
        )
        .bind(days)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("delete_old_requests failed: {e}");
            OdisError::DatabaseError
        })?;
        Ok(result.rows_affected())
    }
}
