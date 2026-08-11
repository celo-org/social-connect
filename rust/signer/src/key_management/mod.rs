mod google;
mod mock;

pub use google::GoogleSecretManagerKeyProvider;
pub use mock::MockKeyProvider;

use async_trait::async_trait;

use crate::errors::OdisError;

/// Provides private key shares for BLS signing.
#[async_trait]
pub trait KeyProvider: Send + Sync {
    /// Get a private key by name and version.
    /// Returns hex-encoded key bytes on success.
    async fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError>;
}
