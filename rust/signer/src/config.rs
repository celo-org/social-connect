use alloy::primitives::Address;
use std::env;

#[derive(Debug, Clone, PartialEq)]
pub enum KeystoreType {
    Mock,
    GoogleSecretManager,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    pub pnp_api_enabled: bool,
    pub keystore_type: KeystoreType,
    pub pnp_key_name_base: String,
    pub pnp_latest_key_version: u32,
    pub db_path: String,
    pub blockchain_provider: Option<String>,
    pub chain_id: u64,
    pub accounts_contract_address: Option<Address>,
    pub odis_payments_contract_address: Option<Address>,
    pub full_node_retry_count: u32,
    pub full_node_retry_delay_ms: u64,
    pub timeout_ms: u64,
    pub query_price_per_cusd: f64,
    pub google_project_id: Option<String>,
    pub request_pruning_days: u64,
    pub request_pruning_interval_secs: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(String),
    #[error("invalid value for {name}: {source}")]
    InvalidValue {
        name: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Call `dotenvy::dotenv()` before this if you want `.env` file support.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Config {
            server_port: parse_env("SERVER_PORT", Some(8080))?,
            pnp_api_enabled: parse_env_bool("PHONE_NUMBER_PRIVACY_API_ENABLED", Some(false))?,
            keystore_type: parse_keystore_type()?,
            pnp_key_name_base: parse_env_string(
                "PHONE_NUMBER_PRIVACY_KEY_NAME_BASE",
                Some("phoneNumberPrivacy"),
            )?,
            pnp_latest_key_version: parse_env("PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION", Some(1))?,
            db_path: parse_env_string("DB_PATH", Some(":memory:"))?,
            blockchain_provider: env::var("BLOCKCHAIN_PROVIDER")
                .ok()
                .filter(|s| !s.is_empty()),
            chain_id: parse_env("CHAIN_ID", Some(44787))?,
            accounts_contract_address: parse_env_address("ACCOUNTS_CONTRACT_ADDRESS")?,
            odis_payments_contract_address: parse_env_address("ODIS_PAYMENTS_CONTRACT_ADDRESS")?,
            full_node_retry_count: parse_env("FULL_NODE_RETRY_COUNT", Some(5))?,
            full_node_retry_delay_ms: parse_env("FULL_NODE_RETRY_DELAY_MS", Some(100))?,
            timeout_ms: parse_env("ODIS_SIGNER_TIMEOUT", Some(5000))?,
            query_price_per_cusd: parse_env("QUERY_PRICE_PER_CUSD", Some(0.001))?,
            google_project_id: env::var("KEYSTORE_GOOGLE_PROJECT_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            request_pruning_days: parse_env("REQUEST_PRUNING_DAYS", Some(7))?,
            request_pruning_interval_secs: parse_env("REQUEST_PRUNING_INTERVAL_SECS", Some(86400))?,
        })
    }
}

fn parse_env_string(name: &str, default: Option<&str>) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(val) => Ok(val),
        Err(_) => default
            .map(String::from)
            .ok_or_else(|| ConfigError::Missing(name.to_string())),
    }
}

fn parse_env<T>(name: &str, default: Option<T>) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(name) {
        Ok(val) => val.parse::<T>().map_err(|e| ConfigError::InvalidValue {
            name: name.to_string(),
            source: Box::new(e),
        }),
        Err(_) => default.ok_or_else(|| ConfigError::Missing(name.to_string())),
    }
}

fn parse_env_address(name: &str) -> Result<Option<Address>, ConfigError> {
    match env::var(name) {
        Ok(val) => val
            .parse::<Address>()
            .map(Some)
            .map_err(|e| ConfigError::InvalidValue {
                name: name.to_string(),
                source: Box::new(e),
            }),
        Err(_) => Ok(None),
    }
}

fn parse_env_bool(name: &str, default: Option<bool>) -> Result<bool, ConfigError> {
    match env::var(name) {
        Ok(val) => match val.to_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                name: name.to_string(),
                source: format!("expected true/false/1/0, got '{val}'").into(),
            }),
        },
        Err(_) => default.ok_or_else(|| ConfigError::Missing(name.to_string())),
    }
}

fn parse_keystore_type() -> Result<KeystoreType, ConfigError> {
    let val = parse_env_string("KEYSTORE_TYPE", None)?;
    match val.to_lowercase().as_str() {
        "mock" | "mocksecretmanager" => Ok(KeystoreType::Mock),
        "googlesecretmanager" => Ok(KeystoreType::GoogleSecretManager),
        _ => Err(ConfigError::InvalidValue {
            name: "KEYSTORE_TYPE".to_string(),
            source: format!("unknown keystore type: '{val}'").into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;
    use std::sync::Mutex;

    // Env vars are process-global, so tests that modify them must not run in parallel.
    // SAFETY: All env-modifying tests hold ENV_LOCK, ensuring single-threaded access.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    unsafe fn set(key: &str, val: &str) {
        unsafe { env::set_var(key, val) };
    }

    unsafe fn clear_env() {
        for key in [
            "SERVER_PORT",
            "PHONE_NUMBER_PRIVACY_API_ENABLED",
            "KEYSTORE_TYPE",
            "PHONE_NUMBER_PRIVACY_KEY_NAME_BASE",
            "PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION",
            "DB_PATH",
            "BLOCKCHAIN_PROVIDER",
            "CHAIN_ID",
            "ACCOUNTS_CONTRACT_ADDRESS",
            "ODIS_PAYMENTS_CONTRACT_ADDRESS",
            "FULL_NODE_RETRY_COUNT",
            "FULL_NODE_RETRY_DELAY_MS",
            "ODIS_SIGNER_TIMEOUT",
            "QUERY_PRICE_PER_CUSD",
            "KEYSTORE_GOOGLE_PROJECT_ID",
            "REQUEST_PRUNING_DAYS",
            "REQUEST_PRUNING_INTERVAL_SECS",
        ] {
            unsafe { env::remove_var(key) };
        }
    }

    #[test]
    fn defaults_with_mock_keystore() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
        }

        let config = Config::from_env().unwrap();

        assert_eq!(config.server_port, 8080);
        assert!(!config.pnp_api_enabled);
        assert_eq!(config.keystore_type, KeystoreType::Mock);
        assert_eq!(config.pnp_key_name_base, "phoneNumberPrivacy");
        assert_eq!(config.pnp_latest_key_version, 1);
        assert_eq!(config.db_path, ":memory:");
        assert!(config.blockchain_provider.is_none());
        assert_eq!(config.chain_id, 44787);
        assert_eq!(config.accounts_contract_address, None);
        assert_eq!(config.odis_payments_contract_address, None);
        assert_eq!(config.full_node_retry_count, 5);
        assert_eq!(config.full_node_retry_delay_ms, 100);
        assert_eq!(config.timeout_ms, 5000);
        assert!((config.query_price_per_cusd - 0.001).abs() < f64::EPSILON);
        assert!(config.google_project_id.is_none());
        assert_eq!(config.request_pruning_days, 7);
        assert_eq!(config.request_pruning_interval_secs, 86400);
    }

    #[test]
    fn custom_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("SERVER_PORT", "9090");
            set("PHONE_NUMBER_PRIVACY_API_ENABLED", "true");
            set("KEYSTORE_TYPE", "GoogleSecretManager");
            set("KEYSTORE_GOOGLE_PROJECT_ID", "my-gcp-project");
            set("PHONE_NUMBER_PRIVACY_KEY_NAME_BASE", "mykey");
            set("PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION", "3");
            set("DB_PATH", "/tmp/test.db");
            set("BLOCKCHAIN_PROVIDER", "https://rpc.example.com");
            set("CHAIN_ID", "42220");
            set(
                "ACCOUNTS_CONTRACT_ADDRESS",
                "0x1234567890abcdef1234567890abcdef12345678",
            );
            set(
                "ODIS_PAYMENTS_CONTRACT_ADDRESS",
                "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
            );
            set("FULL_NODE_RETRY_COUNT", "3");
            set("FULL_NODE_RETRY_DELAY_MS", "200");
            set("ODIS_SIGNER_TIMEOUT", "10000");
            set("QUERY_PRICE_PER_CUSD", "0.01");
        }

        let config = Config::from_env().unwrap();

        assert_eq!(config.server_port, 9090);
        assert!(config.pnp_api_enabled);
        assert_eq!(config.keystore_type, KeystoreType::GoogleSecretManager);
        assert_eq!(config.google_project_id.as_deref(), Some("my-gcp-project"));
        assert_eq!(config.pnp_key_name_base, "mykey");
        assert_eq!(config.pnp_latest_key_version, 3);
        assert_eq!(config.db_path, "/tmp/test.db");
        assert_eq!(
            config.blockchain_provider.as_deref(),
            Some("https://rpc.example.com")
        );
        assert_eq!(config.chain_id, 42220);
        assert_eq!(
            config.accounts_contract_address,
            Some(address!("0x1234567890abcdef1234567890abcdef12345678"))
        );
        assert_eq!(
            config.odis_payments_contract_address,
            Some(address!("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"))
        );
        assert_eq!(config.full_node_retry_count, 3);
        assert_eq!(config.full_node_retry_delay_ms, 200);
        assert_eq!(config.timeout_ms, 10000);
        assert!((config.query_price_per_cusd - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn missing_keystore_type_is_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe { clear_env() };

        let result = Config::from_env();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::Missing(ref name) if name == "KEYSTORE_TYPE"));
    }

    #[test]
    fn invalid_keystore_type_is_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "nonsense");
        }

        let result = Config::from_env();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidValue { .. }
        ));
    }

    #[test]
    fn invalid_port_is_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("SERVER_PORT", "not_a_number");
        }

        let result = Config::from_env();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidValue { .. }
        ));
    }

    #[test]
    fn invalid_bool_is_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("PHONE_NUMBER_PRIVACY_API_ENABLED", "yes");
        }

        let result = Config::from_env();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidValue { .. }
        ));
    }

    #[test]
    fn empty_string_env_vars_are_treated_as_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("BLOCKCHAIN_PROVIDER", "");
            set("KEYSTORE_GOOGLE_PROJECT_ID", "");
        }

        let config = Config::from_env().unwrap();
        assert!(config.blockchain_provider.is_none());
        assert!(config.google_project_id.is_none());
    }

    #[test]
    fn mocksecretmanager_maps_to_mock() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "MockSecretManager");
        }

        let config = Config::from_env().unwrap();
        assert_eq!(config.keystore_type, KeystoreType::Mock);
    }
}
