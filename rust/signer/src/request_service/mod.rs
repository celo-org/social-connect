mod metered;
mod sqlite;

pub use metered::MeteredPnpRequestService;
pub use sqlite::SqlitePnpRequestService;

use alloy::primitives::Address;
use async_trait::async_trait;

use crate::errors::OdisError;

/// Tracks PNP sign requests and quota usage.
#[async_trait]
pub trait PnpRequestService: Send + Sync {
    async fn get_used_quota(&self, address: Address) -> Result<u32, OdisError>;
    async fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError>;
    async fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
    ) -> Result<(), OdisError>;

    /// Delete requests older than the given number of days.
    /// Returns the number of deleted rows.
    async fn delete_old_requests(&self, older_than_days: u64) -> Result<u64, OdisError>;
}
