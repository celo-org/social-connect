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

    async fn delete_old_requests(&self, older_than_days: u64) -> Result<u64, OdisError> {
        let modifier = format!("-{older_than_days} days");
        let result = sqlx::query("DELETE FROM requests WHERE timestamp <= datetime('now', ?)")
            .bind(&modifier)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!("delete_old_requests failed: {e}");
                OdisError::DatabaseError
            })?;
        Ok(result.rows_affected())
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

    /// Insert a request with a specific timestamp (YYYY-MM-DD HH:MM:SS format).
    async fn insert_with_timestamp(
        svc: &SqlitePnpRequestService,
        addr: &str,
        query: &str,
        ts: &str,
    ) {
        sqlx::query(
            "INSERT INTO requests (caller_address, blinded_query, signature, timestamp) \
             VALUES (?, ?, 'sig', ?)",
        )
        .bind(addr)
        .bind(query)
        .bind(ts)
        .execute(&svc.pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn delete_old_requests_removes_expired() {
        let svc = test_service().await;
        let addr = ADDR.to_string();

        // Insert a request from 10 days ago
        insert_with_timestamp(&svc, &addr, "old_query", "2000-01-01 00:00:00").await;
        // Insert a fresh request (uses default datetime('now'))
        svc.record_request(ADDR, "new_query", "sig").await.unwrap();

        let deleted = svc.delete_old_requests(7).await.unwrap();
        assert_eq!(deleted, 1);

        // Old request gone, new one survives
        assert!(
            svc.get_duplicate_request(ADDR, "old_query")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            svc.get_duplicate_request(ADDR, "new_query")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn delete_old_requests_with_zero_days_deletes_all() {
        let svc = test_service().await;

        svc.record_request(ADDR, "q1", "sig1").await.unwrap();
        svc.record_request(ADDR, "q2", "sig2").await.unwrap();

        let deleted = svc.delete_old_requests(0).await.unwrap();
        assert_eq!(deleted, 2);
    }

    #[tokio::test]
    async fn delete_old_requests_returns_zero_when_nothing_to_prune() {
        let svc = test_service().await;
        let deleted = svc.delete_old_requests(7).await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn addresses_are_independent() {
        let svc = test_service().await;

        svc.record_request(ADDR, "query1", "sig1").await.unwrap();
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(svc.get_used_quota(OTHER_ADDR).await.unwrap(), 0);
    }
}
