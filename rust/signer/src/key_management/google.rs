use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::KeyProvider;
use crate::errors::OdisError;

/// Expected length of a hex-encoded BLS key share (4-byte u32 index + 32-byte scalar).
const PRIVATE_KEY_HEX_SIZE: usize = 72;

/// Key provider that fetches BLS key shares from Google Secret Manager.
///
/// Keys are cached in memory after the first fetch since they don't change.
/// Uses Application Default Credentials (ADC) for authentication.
pub struct GoogleSecretManagerKeyProvider {
    client: google_cloud_secretmanager_v1::client::SecretManagerService,
    project_id: String,
    cache: RwLock<HashMap<String, String>>,
}

impl GoogleSecretManagerKeyProvider {
    /// Create a new provider, connecting to Google Secret Manager via ADC.
    pub async fn new(project_id: &str) -> Result<Self, OdisError> {
        let client = google_cloud_secretmanager_v1::client::SecretManagerService::builder()
            .build()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "failed to create Secret Manager client");
                OdisError::KeyFetchError
            })?;

        Ok(Self {
            client,
            project_id: project_id.to_string(),
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Prefetch a key at startup so the signer fails fast if GCP is misconfigured.
    pub async fn prefetch(&self, name: &str, version: u32) -> Result<(), OdisError> {
        self.get_key(name, version).await?;
        Ok(())
    }

    /// Fetch a secret version from Google Secret Manager.
    async fn fetch_from_gcp(&self, name: &str, version: u32) -> Result<String, OdisError> {
        let secret_id = format!(
            "projects/{}/secrets/{}-{}/versions/latest",
            self.project_id, name, version
        );

        tracing::debug!(secret_id, "fetching key from Secret Manager");

        let response = self
            .client
            .access_secret_version()
            .set_name(&secret_id)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, secret_id, "failed to access secret version");
                OdisError::KeyFetchError
            })?;

        let payload = response.payload.ok_or_else(|| {
            tracing::error!(secret_id, "secret version has no payload");
            OdisError::KeyFetchError
        })?;

        let raw = String::from_utf8(payload.data.to_vec()).map_err(|e| {
            tracing::error!(error = %e, secret_id, "secret payload is not valid UTF-8");
            OdisError::KeyFetchError
        })?;

        let key = raw.trim().to_string();

        if key.len() != PRIVATE_KEY_HEX_SIZE {
            tracing::error!(
                secret_id,
                expected = PRIVATE_KEY_HEX_SIZE,
                actual = key.len(),
                "invalid private key length"
            );
            return Err(OdisError::KeyFetchError);
        }

        Ok(key)
    }
}

#[async_trait]
impl KeyProvider for GoogleSecretManagerKeyProvider {
    async fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError> {
        let cache_key = format!("{name}-{version}");

        // Fast path: check read lock
        {
            let cache = self.cache.read().await;
            if let Some(key) = cache.get(&cache_key) {
                return Ok(key.clone());
            }
        }

        // Slow path: fetch and cache
        let key = self.fetch_from_gcp(name, version).await?;

        {
            let mut cache = self.cache.write().await;
            cache.insert(cache_key, key.clone());
        }

        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_key_length_is_accepted() {
        // 72 hex chars = valid
        let key = "000000000e7e1a2fad3b54deb2b1b32cf4c7b084842d50bbb5c6143b9d9577d16e050f03";
        assert_eq!(key.len(), PRIVATE_KEY_HEX_SIZE);
    }

    #[test]
    fn invalid_key_lengths_are_rejected() {
        assert_ne!("too_short".len(), PRIVATE_KEY_HEX_SIZE);
        assert_ne!("a".repeat(100).len(), PRIVATE_KEY_HEX_SIZE);
    }
}
