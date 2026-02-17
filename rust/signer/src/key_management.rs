use std::collections::HashMap;

use crate::errors::OdisError;

/// Provides private key shares for BLS signing.
pub trait KeyProvider: Send + Sync {
    /// Get a private key by name and version.
    /// Returns hex-encoded key bytes on success.
    fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError>;
}

/// Mock key provider with hardcoded dev key shares from values.ts.
/// Keys are stored as `"{name}-{version}" -> hex_key`.
pub struct MockKeyProvider {
    keys: HashMap<String, String>,
}

impl Default for MockKeyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockKeyProvider {
    pub fn new() -> Self {
        let keys = HashMap::from([
            (
                "phoneNumberPrivacy-1".to_string(),
                "000000000e7e1a2fad3b54deb2b1b32cf4c7b084842d50bbb5c6143b9d9577d16e050f03"
                    .to_string(),
            ),
            (
                "phoneNumberPrivacy-2".to_string(),
                "0000000087c722e1338395b942d8332328795a46c718baeb8fef9e5c63111d495469c50e"
                    .to_string(),
            ),
            (
                "phoneNumberPrivacy-3".to_string(),
                "000000005b2c8089ead28a08233b6b16b2341542453523445950cfbd9bd2f1d09c8eee0c"
                    .to_string(),
            ),
        ]);
        Self { keys }
    }
}

impl KeyProvider for MockKeyProvider {
    fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError> {
        let key_id = format!("{name}-{version}");
        self.keys
            .get(&key_id)
            .cloned()
            .ok_or(OdisError::KeyFetchError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_provider_returns_keys_for_all_versions() {
        let provider = MockKeyProvider::new();

        for version in 1..=3 {
            let key = provider.get_key("phoneNumberPrivacy", version);
            assert!(key.is_ok(), "version {version} should exist");
            // All dev keys are 72 hex chars (4-byte index + 32-byte scalar)
            assert_eq!(key.unwrap().len(), 72);
        }
    }

    #[test]
    fn mock_provider_returns_error_for_unknown_key() {
        let provider = MockKeyProvider::new();

        assert!(provider.get_key("phoneNumberPrivacy", 99).is_err());
        assert!(provider.get_key("unknown", 1).is_err());
    }
}
