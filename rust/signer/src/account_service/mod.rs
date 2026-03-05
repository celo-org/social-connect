mod caching;
mod client;
mod metered;
mod mock;

pub use caching::CachingAccountService;
pub use client::ClientAccountService;
pub use metered::MeteredAccountService;
pub use mock::MockAccountService;

use alloy::primitives::Address;
use async_trait::async_trait;

use crate::errors::OdisError;

/// On-chain account data needed for PNP request processing.
#[derive(Clone)]
pub struct PnpAccount {
    pub address: Address,
    /// Hex-encoded SEC1 public key (data encryption key).
    pub dek: String,
    pub pnp_total_quota: u32,
}

#[async_trait]
pub trait AccountService: Send + Sync {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError>;
}
