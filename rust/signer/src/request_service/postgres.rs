use alloy::primitives::Address;
use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::PnpRequestService;
use super::legacy_migration::{LegacyMigrationReport, run_legacy_migration};
use crate::errors::OdisError;

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
    ) -> Result<(), OdisError> {
        let addr = address.to_string();
        let mut tx = self.pool.begin().await.map_err(|e| {
            tracing::error!("failed to begin transaction: {e}");
            OdisError::DatabaseError
        })?;

        sqlx::query(
            "INSERT INTO requests (caller_address, blinded_query, signature) VALUES ($1, $2, $3)",
        )
        .bind(&addr)
        .bind(blinded_query)
        .bind(signature)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("insert request failed: {e}");
            OdisError::DatabaseError
        })?;

        sqlx::query(
            "INSERT INTO accounts (address, num_lookups) VALUES ($1, 1) \
             ON CONFLICT (address) DO UPDATE SET num_lookups = accounts.num_lookups + 1",
        )
        .bind(&addr)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("upsert account failed: {e}");
            OdisError::DatabaseError
        })?;

        tx.commit().await.map_err(|e| {
            tracing::error!("failed to commit transaction: {e}");
            OdisError::DatabaseError
        })?;

        Ok(())
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
