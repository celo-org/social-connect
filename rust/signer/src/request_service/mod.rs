mod legacy_migration;
mod metered;
mod postgres;
mod sqlite;

pub use legacy_migration::LegacyMigrationReport;
pub use metered::MeteredPnpRequestService;
pub use postgres::PostgresPnpRequestService;
pub use sqlite::SqlitePnpRequestService;

use alloy::primitives::Address;
use async_trait::async_trait;

use crate::errors::OdisError;

/// What happened when a request was recorded.
///
/// Recording resolves the duplicate and quota questions atomically, so the
/// caller learns the authoritative outcome even when concurrent requests for
/// the same account raced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// The request was new and within quota. Quota was charged.
    Recorded { performed_query_count: u32 },
    /// The query was already on record for this account, under any key
    /// version. Quota was not charged again and the record was left as it was.
    Duplicate { performed_query_count: u32 },
    /// The account was out of quota. Nothing was written.
    QuotaExceeded { performed_query_count: u32 },
}

/// A charged query as it is on record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub signature: String,
    /// The key version `signature` was made with. `None` when it is unknown:
    /// the row was imported from the TS signer, which never stored it. Such a
    /// signature must not be served, because it cannot be matched to the
    /// version a request asks for.
    pub key_version: Option<u32>,
}

/// Tracks PNP sign requests and quota usage.
///
/// A query is charged once per account, whatever key version signs it, so
/// there is one record per `(address, blinded_query)`. The record keeps the
/// signature of the request that was charged, and its key version.
#[async_trait]
pub trait PnpRequestService: Send + Sync {
    async fn get_used_quota(&self, address: Address) -> Result<u32, OdisError>;
    async fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<RecordedRequest>, OdisError>;

    /// Atomically record a request and charge one unit of quota against
    /// `total_quota`. `signature` was made with key version `key_version`.
    ///
    /// Duplicate detection and the quota bound are both enforced inside the
    /// write transaction, so concurrent requests for the same account can
    /// neither exceed `total_quota` nor collide on the `requests` primary key.
    async fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
        key_version: u32,
        total_quota: u32,
    ) -> Result<RecordOutcome, OdisError>;

    /// Delete requests older than the given number of days.
    /// Returns the number of deleted rows.
    async fn delete_old_requests(&self, older_than_days: u64) -> Result<u64, OdisError>;
}
