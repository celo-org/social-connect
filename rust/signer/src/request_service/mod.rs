mod memory;
mod sqlite;

pub use memory::InMemoryPnpRequestService;
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
}
