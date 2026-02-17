use alloy::primitives::Address;
use async_trait::async_trait;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use super::PnpRequestService;
use crate::errors::OdisError;

pub struct SqlitePnpRequestService {
    pool: SqlitePool,
}

impl SqlitePnpRequestService {
    pub async fn new(db_path: &str) -> Result<Self, OdisError> {
        let is_memory = db_path == ":memory:";
        let base = if is_memory {
            SqliteConnectOptions::new()
                .in_memory(true)
                .shared_cache(true)
        } else {
            SqliteConnectOptions::new()
                .filename(db_path)
                .create_if_missing(true)
        };
        let options = base
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(if is_memory { 1 } else { 5 })
            .connect_with(options)
            .await
            .map_err(|e| {
                tracing::error!("failed to connect to database: {e}");
                OdisError::DatabaseError
            })?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| {
                tracing::error!("failed to run migrations: {e}");
                OdisError::DatabaseError
            })?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl PnpRequestService for SqlitePnpRequestService {
    async fn get_used_quota(&self, address: Address) -> Result<u32, OdisError> {
        let addr = address.to_string();
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT num_lookups FROM accounts WHERE address = ?")
                .bind(&addr)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    tracing::error!("get_used_quota query failed: {e}");
                    OdisError::DatabaseError
                })?;
        Ok(row.map(|(n,)| n as u32).unwrap_or(0))
    }

    async fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError> {
        let addr = address.to_string();
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT signature FROM requests WHERE caller_address = ? AND blinded_query = ?",
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
            "INSERT INTO requests (caller_address, blinded_query, signature) VALUES (?, ?, ?)",
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
            "INSERT INTO accounts (address, num_lookups) VALUES (?, 1) \
             ON CONFLICT(address) DO UPDATE SET num_lookups = num_lookups + 1",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const ADDR: Address = address!("0x0000000000000000000000000000000000007E57");
    const OTHER_ADDR: Address = address!("0x0000000000000000000000000000000000000001");

    async fn test_service() -> SqlitePnpRequestService {
        SqlitePnpRequestService::new(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn initially_zero_quota() {
        let svc = test_service().await;
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn no_duplicate_for_unknown_request() {
        let svc = test_service().await;
        assert!(
            svc.get_duplicate_request(ADDR, "query1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn record_increments_quota_and_stores_signature() {
        let svc = test_service().await;

        svc.record_request(ADDR, "query1", "sig1").await.unwrap();
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(
            svc.get_duplicate_request(ADDR, "query1").await.unwrap(),
            Some("sig1".to_string())
        );

        // Different query increments quota again
        svc.record_request(ADDR, "query2", "sig2").await.unwrap();
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn addresses_are_independent() {
        let svc = test_service().await;

        svc.record_request(ADDR, "query1", "sig1").await.unwrap();
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(svc.get_used_quota(OTHER_ADDR).await.unwrap(), 0);
    }
}
