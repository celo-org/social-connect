use std::time::Duration;

use alloy::primitives::Address;
use async_trait::async_trait;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use super::{PnpRequestService, RecordOutcome, RecordedRequest};
use crate::errors::OdisError;

/// Read an account's used quota inside an open transaction. Absent accounts
/// have used nothing.
async fn read_used_quota(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    address: &str,
) -> Result<u32, OdisError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT num_lookups FROM accounts WHERE address = ?")
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

pub struct SqlitePnpRequestService {
    pool: SqlitePool,
}

impl SqlitePnpRequestService {
    /// Opens the database and runs the schema migrations. `timeout` is how long
    /// a statement waits for a lock held by another connection.
    pub async fn new(db_path: &str, timeout: Duration) -> Result<Self, OdisError> {
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
            .busy_timeout(timeout);

        let pool_options = SqlitePoolOptions::new().max_connections(if is_memory { 1 } else { 5 });
        // An in-memory database lives in its one connection, so the pool must
        // never close it: closing destroys the data and the schema. By default
        // the pool closes idle connections and those older than 30 minutes.
        let pool_options = if is_memory {
            pool_options.idle_timeout(None).max_lifetime(None)
        } else {
            pool_options
        };
        let pool = pool_options.connect_with(options).await.map_err(|e| {
            tracing::error!("failed to connect to database: {e}");
            OdisError::DatabaseError
        })?;

        sqlx::migrate!("./migrations/sqlite")
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
    ) -> Result<Option<RecordedRequest>, OdisError> {
        let addr = address.to_string();
        let row: Option<(String, Option<i64>)> = sqlx::query_as(
            "SELECT signature, key_version FROM requests WHERE caller_address = ? AND blinded_query = ?",
        )
        .bind(&addr)
        .bind(blinded_query)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!("get_duplicate_request query failed: {e}");
            OdisError::DatabaseError
        })?;
        Ok(row.map(|(signature, key_version)| RecordedRequest {
            signature,
            key_version: key_version.and_then(|version| u32::try_from(version).ok()),
        }))
    }

    async fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
        key_version: u32,
        total_quota: u32,
    ) -> Result<RecordOutcome, OdisError> {
        let addr = address.to_string();
        let quota = i64::from(total_quota);
        let mut tx = self.pool.begin().await.map_err(|e| {
            tracing::error!("failed to begin transaction: {e}");
            OdisError::DatabaseError
        })?;

        // Claim the (address, query) slot. This is the first statement in the
        // transaction, so it takes the write lock before anything is read.
        let claimed = sqlx::query(
            "INSERT INTO requests (caller_address, blinded_query, signature, key_version) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (caller_address, blinded_query) DO NOTHING",
        )
        .bind(&addr)
        .bind(blinded_query)
        .bind(signature)
        .bind(i64::from(key_version))
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("insert request failed: {e}");
            OdisError::DatabaseError
        })?
        .rows_affected()
            == 1;

        // The query is already on record and charged, under this or another
        // key version. The record stays as it is.
        if !claimed {
            let used = read_used_quota(&mut tx, &addr).await?;
            tx.commit().await.map_err(|e| {
                tracing::error!("failed to commit transaction: {e}");
                OdisError::DatabaseError
            })?;

            return Ok(RecordOutcome::Duplicate {
                performed_query_count: used,
            });
        }

        // Charge quota. The bound is re-checked against the stored row inside
        // the same transaction, so it holds under concurrency. No row comes
        // back when the account is already at its limit, or when the quota is
        // zero and the account has no row yet.
        let charged: Option<(i64,)> = sqlx::query_as(
            "INSERT INTO accounts (address, num_lookups) SELECT ?, 1 WHERE ? > 0 \
             ON CONFLICT (address) DO UPDATE SET num_lookups = num_lookups + 1 \
             WHERE num_lookups < ? \
             RETURNING num_lookups",
        )
        .bind(&addr)
        .bind(quota)
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
    use std::sync::Arc;

    const ADDR: Address = address!("0x0000000000000000000000000000000000007E57");
    const OTHER_ADDR: Address = address!("0x0000000000000000000000000000000000000001");
    /// The default of `DB_TIMEOUT`.
    const TIMEOUT: Duration = Duration::from_millis(1000);

    async fn test_service() -> SqlitePnpRequestService {
        SqlitePnpRequestService::new(":memory:", TIMEOUT)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn in_memory_pool_never_closes_its_connection() {
        let svc = test_service().await;
        let options = svc.pool.options();
        assert_eq!(options.get_max_connections(), 1);
        assert_eq!(options.get_idle_timeout(), None);
        assert_eq!(options.get_max_lifetime(), None);
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

        assert_eq!(
            svc.record_request(ADDR, "query1", "sig1", 1, 10)
                .await
                .unwrap(),
            RecordOutcome::Recorded {
                performed_query_count: 1
            }
        );
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(
            svc.get_duplicate_request(ADDR, "query1").await.unwrap(),
            Some(RecordedRequest {
                signature: "sig1".to_string(),
                key_version: Some(1),
            })
        );

        // Different query increments quota again
        assert_eq!(
            svc.record_request(ADDR, "query2", "sig2", 1, 10)
                .await
                .unwrap(),
            RecordOutcome::Recorded {
                performed_query_count: 2
            }
        );
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn re_recording_a_query_is_not_charged_and_keeps_the_record() {
        let svc = test_service().await;

        svc.record_request(ADDR, "query1", "sig1", 1, 10)
            .await
            .unwrap();

        // Neither a second attempt under the same key version nor one under
        // another version is charged or replaces what is on record.
        for key_version in [1, 2] {
            assert_eq!(
                svc.record_request(ADDR, "query1", "sig2", key_version, 10)
                    .await
                    .unwrap(),
                RecordOutcome::Duplicate {
                    performed_query_count: 1,
                }
            );
        }
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(
            svc.get_duplicate_request(ADDR, "query1").await.unwrap(),
            Some(RecordedRequest {
                signature: "sig1".to_string(),
                key_version: Some(1),
            })
        );
    }

    /// A row may lack the key version, as rows imported from the TS signer do.
    #[tokio::test]
    async fn a_request_stored_without_a_key_version_has_an_unknown_one() {
        let svc = test_service().await;
        insert_with_timestamp(&svc, &ADDR.to_string(), "query1", "2000-01-01 00:00:00").await;

        assert_eq!(
            svc.get_duplicate_request(ADDR, "query1").await.unwrap(),
            Some(RecordedRequest {
                signature: "sig".to_string(),
                key_version: None,
            })
        );
        // It still marks the query as charged.
        assert_eq!(
            svc.record_request(ADDR, "query1", "sig2", 1, 10)
                .await
                .unwrap(),
            RecordOutcome::Duplicate {
                performed_query_count: 0,
            }
        );
    }

    #[tokio::test]
    async fn record_refuses_to_exceed_quota_and_writes_nothing() {
        let svc = test_service().await;

        svc.record_request(ADDR, "query1", "sig1", 1, 2)
            .await
            .unwrap();
        svc.record_request(ADDR, "query2", "sig2", 1, 2)
            .await
            .unwrap();

        assert_eq!(
            svc.record_request(ADDR, "query3", "sig3", 1, 2)
                .await
                .unwrap(),
            RecordOutcome::QuotaExceeded {
                performed_query_count: 2
            }
        );
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 2);
        // The rejected request must not linger in the table.
        assert!(
            svc.get_duplicate_request(ADDR, "query3")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn zero_quota_rejects_a_brand_new_account() {
        let svc = test_service().await;

        assert_eq!(
            svc.record_request(ADDR, "query1", "sig1", 1, 0)
                .await
                .unwrap(),
            RecordOutcome::QuotaExceeded {
                performed_query_count: 0
            }
        );
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 0);
        assert!(
            svc.get_duplicate_request(ADDR, "query1")
                .await
                .unwrap()
                .is_none()
        );
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
        svc.record_request(ADDR, "new_query", "sig", 1, 10)
            .await
            .unwrap();

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

        svc.record_request(ADDR, "q1", "sig1", 1, 10).await.unwrap();
        svc.record_request(ADDR, "q2", "sig2", 1, 10).await.unwrap();

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

        svc.record_request(ADDR, "query1", "sig1", 1, 10)
            .await
            .unwrap();
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
        assert_eq!(svc.get_used_quota(OTHER_ADDR).await.unwrap(), 0);
    }

    /// Concurrency needs a file-backed database: the in-memory service is
    /// pinned to a single pooled connection.
    async fn concurrent_test_service() -> (Arc<SqlitePnpRequestService>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.db");
        let svc = SqlitePnpRequestService::new(path.to_str().unwrap(), TIMEOUT)
            .await
            .unwrap();
        (Arc::new(svc), dir)
    }

    /// Records `queries` all at once, alternating between key versions 1 and 2.
    async fn record_concurrently(
        svc: &Arc<SqlitePnpRequestService>,
        queries: Vec<String>,
        total_quota: u32,
    ) -> Vec<RecordOutcome> {
        let handles: Vec<_> = queries
            .into_iter()
            .enumerate()
            .map(|(i, query)| {
                let svc = Arc::clone(svc);
                let key_version = 1 + (i % 2) as u32;
                tokio::spawn(async move {
                    let signature = format!("sig-{query}-v{key_version}");
                    svc.record_request(ADDR, &query, &signature, key_version, total_quota)
                        .await
                        .unwrap()
                })
            })
            .collect();

        let mut outcomes = Vec::new();
        for handle in handles {
            outcomes.push(handle.await.unwrap());
        }
        outcomes
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_identical_requests_charge_quota_once() {
        let (svc, _dir) = concurrent_test_service().await;

        let outcomes = record_concurrently(&svc, vec!["same".to_string(); 8], 100).await;

        let recorded = outcomes
            .iter()
            .filter(|o| matches!(o, RecordOutcome::Recorded { .. }))
            .count();
        assert_eq!(recorded, 1, "exactly one racer should win the slot");

        // Every loser learns that the query is on record, not an error.
        let duplicates = outcomes
            .iter()
            .filter(|o| matches!(o, RecordOutcome::Duplicate { .. }))
            .count();
        assert_eq!(duplicates, 7);

        // The record holds the winner's signature with the winner's key version.
        let recorded_request = svc
            .get_duplicate_request(ADDR, "same")
            .await
            .unwrap()
            .unwrap();
        let key_version = recorded_request.key_version.unwrap();
        assert_eq!(
            recorded_request.signature,
            format!("sig-same-v{key_version}")
        );
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_distinct_requests_cannot_overrun_quota() {
        let (svc, _dir) = concurrent_test_service().await;

        let queries: Vec<String> = (0..12).map(|i| format!("query{i}")).collect();
        let outcomes = record_concurrently(&svc, queries, 3).await;

        let recorded = outcomes
            .iter()
            .filter(|o| matches!(o, RecordOutcome::Recorded { .. }))
            .count();
        let rejected = outcomes
            .iter()
            .filter(|o| matches!(o, RecordOutcome::QuotaExceeded { .. }))
            .count();

        assert_eq!(recorded, 3, "quota bound must hold under concurrency");
        assert_eq!(rejected, 9);
        assert_eq!(svc.get_used_quota(ADDR).await.unwrap(), 3);

        // Rejected requests leave no rows behind.
        let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM requests")
            .fetch_one(&svc.pool)
            .await
            .unwrap();
        assert_eq!(rows, 3);
    }
}
