use alloy::primitives::Address;
use std::env;

#[derive(Debug, Clone, PartialEq)]
pub enum KeystoreType {
    Mock,
    GoogleSecretManager,
}

/// How log lines are written to stdout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogFormat {
    /// Human-readable lines, for local use.
    Text,
    /// One JSON object per line, for log collectors.
    Json,
}

/// Which persistence backend to use for PNP request/quota tracking.
///
/// Selected from `DATABASE_URL` by scheme; falls back to SQLite via `DB_PATH`.
#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseConfig {
    /// SQLite file path, or `:memory:` for an in-memory database.
    Sqlite { path: String },
    /// PostgreSQL connection URL (`postgres://...`).
    Postgres { url: String },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    pub pnp_api_enabled: bool,
    pub keystore_type: KeystoreType,
    pub pnp_key_name_base: String,
    pub pnp_latest_key_version: u32,
    pub database: DatabaseConfig,
    /// When true (and using Postgres), run the one-time legacy TS -> canonical
    /// data migration on startup. Guarded by a marker so it never re-runs.
    pub migrate_legacy_data: bool,
    pub blockchain_provider: Option<String>,
    /// Sent as the `apiKey` header on every request to `blockchain_provider`,
    /// which is how forno identifies clients with a raised rate limit.
    pub blockchain_api_key: Option<String>,
    pub chain_id: u64,
    pub accounts_contract_address: Option<Address>,
    pub odis_payments_contract_address: Option<Address>,
    pub full_node_retry_count: u32,
    pub full_node_retry_delay_ms: u64,
    /// Limit in milliseconds for one full node lookup, retries included.
    pub full_node_timeout_ms: u64,
    pub timeout_ms: u64,
    pub query_price_per_cusd: f64,
    pub google_project_id: Option<String>,
    pub request_pruning_days: u64,
    pub request_pruning_interval_secs: u64,
    pub log_format: LogFormat,
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
        // The TS signer swaps in mock services on these. This signer has no
        // such switch, and would read the chain and charge quota where the
        // operator expects mock answers.
        // TODO: remove once the migration from the TS signer is finished; only TS manifests
        // set these variables.
        for ts_variable in ["SHOULD_MOCK_ACCOUNT_SERVICE", "SHOULD_MOCK_REQUEST_SERVICE"] {
            if env::var(ts_variable).is_ok_and(|v| v.eq_ignore_ascii_case("true")) {
                return Err(ConfigError::InvalidValue {
                    name: ts_variable.to_string(),
                    source: "this signer has no mock services to switch to; remove the variable"
                        .into(),
                });
            }
        }

        let config = Config {
            server_port: parse_env("SERVER_PORT", Some(8080))?,
            pnp_api_enabled: parse_env_bool("PHONE_NUMBER_PRIVACY_API_ENABLED", Some(false))?,
            keystore_type: parse_keystore_type()?,
            pnp_key_name_base: parse_env_string(
                "PHONE_NUMBER_PRIVACY_KEY_NAME_BASE",
                Some("phoneNumberPrivacy"),
            )?,
            pnp_latest_key_version: parse_env("PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION", Some(1))?,
            database: parse_database_config()?,
            migrate_legacy_data: parse_env_bool("MIGRATE_LEGACY_DATA", Some(false))?,
            blockchain_provider: env::var("BLOCKCHAIN_PROVIDER")
                .ok()
                .filter(|s| !s.is_empty()),
            blockchain_api_key: env::var("BLOCKCHAIN_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            chain_id: parse_env("CHAIN_ID", Some(11142220))?,
            accounts_contract_address: parse_env_address("ACCOUNTS_CONTRACT_ADDRESS")?,
            odis_payments_contract_address: parse_env_address("ODIS_PAYMENTS_CONTRACT_ADDRESS")?,
            full_node_retry_count: parse_env("FULL_NODE_RETRY_COUNT", Some(5))?,
            full_node_retry_delay_ms: parse_env("FULL_NODE_RETRY_DELAY_MS", Some(100))?,
            full_node_timeout_ms: parse_env("FULL_NODE_TIMEOUT_MS", Some(1000))?,
            timeout_ms: parse_env("ODIS_SIGNER_TIMEOUT", Some(5000))?,
            query_price_per_cusd: parse_env("QUERY_PRICE_PER_CUSD", Some(0.001))?,
            google_project_id: env::var("KEYSTORE_GOOGLE_PROJECT_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            request_pruning_days: parse_env("REQUEST_PRUNING_DAYS", Some(7))?,
            request_pruning_interval_secs: parse_env("REQUEST_PRUNING_INTERVAL_SECS", Some(86400))?,
            log_format: parse_log_format()?,
        };

        if config.timeout_ms == 0 {
            return Err(ConfigError::InvalidValue {
                name: "ODIS_SIGNER_TIMEOUT".to_string(),
                source: "must be greater than 0".into(),
            });
        }

        if config.full_node_timeout_ms == 0 {
            return Err(ConfigError::InvalidValue {
                name: "FULL_NODE_TIMEOUT_MS".to_string(),
                source: "must be greater than 0".into(),
            });
        }

        if !(config.query_price_per_cusd > 0.0 && config.query_price_per_cusd.is_finite()) {
            return Err(ConfigError::InvalidValue {
                name: "QUERY_PRICE_PER_CUSD".to_string(),
                source: "must be a positive finite number".into(),
            });
        }

        if config.pnp_latest_key_version == 0 {
            return Err(ConfigError::InvalidValue {
                name: "PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION".to_string(),
                source: "must be >= 1".into(),
            });
        }

        Ok(config)
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
            "true" => Ok(true),
            "false" => Ok(false),
            // The TS signer reads every value but `true` as false. A value such
            // as `1` is refused, because reading it as either could switch an
            // API on or off against the operator's intent.
            _ => Err(ConfigError::InvalidValue {
                name: name.to_string(),
                source: format!("expected true or false, got '{val}'").into(),
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

fn parse_log_format() -> Result<LogFormat, ConfigError> {
    let val = parse_env_string("LOG_FORMAT", Some("text"))?;
    match val.to_lowercase().as_str() {
        "text" => Ok(LogFormat::Text),
        "json" => Ok(LogFormat::Json),
        _ => Err(ConfigError::InvalidValue {
            name: "LOG_FORMAT".to_string(),
            source: format!("unknown log format: '{val}', expected 'text' or 'json'").into(),
        }),
    }
}

/// Selects the persistence backend.
///
/// A non-empty `DATABASE_URL` takes precedence and is dispatched by scheme:
/// `postgres://`/`postgresql://` -> Postgres, `sqlite:`/`sqlite://` -> SQLite.
/// When `DATABASE_URL` is unset, falls back to SQLite via `DB_PATH`
/// (default `:memory:`), preserving the original single-backend behavior.
fn parse_database_config() -> Result<DatabaseConfig, ConfigError> {
    match env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
        Some(url) => {
            // Match the scheme case-insensitively (RFC 3986), but slice the
            // original URL so the path/DSN keeps its original casing.
            let lower = url.to_lowercase();
            if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
                Ok(DatabaseConfig::Postgres { url })
            } else if lower.starts_with("sqlite://") {
                Ok(DatabaseConfig::Sqlite {
                    path: url["sqlite://".len()..].to_string(),
                })
            } else if lower.starts_with("sqlite:") {
                // Covers `sqlite::memory:` (-> `:memory:`) and `sqlite:relative/path.db`.
                Ok(DatabaseConfig::Sqlite {
                    path: url["sqlite:".len()..].to_string(),
                })
            } else {
                // The URL can hold a password, so the error names only the
                // scheme. Without a scheme, any part of the value could be the
                // secret, and nothing of it is shown.
                let scheme = url
                    .split_once(':')
                    .map(|(scheme, _)| scheme)
                    .filter(|scheme| {
                        !scheme.is_empty()
                            && scheme
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
                    });
                let problem = match scheme {
                    Some(scheme) => format!("unsupported scheme '{scheme}'"),
                    None => "missing scheme".to_string(),
                };
                Err(ConfigError::InvalidValue {
                    name: "DATABASE_URL".to_string(),
                    source: format!("{problem}; expected postgres:// or sqlite:").into(),
                })
            }
        }
        None => {
            // The TS signer is configured through DB_HOST, DB_TYPE and friends. Starting
            // on the SQLite default with those set would silently use an empty database.
            for ts_variable in ["DB_HOST", "DB_TYPE"] {
                if env::var(ts_variable).is_ok_and(|v| !v.is_empty()) {
                    return Err(ConfigError::InvalidValue {
                        name: "DATABASE_URL".to_string(),
                        source: format!(
                            "{ts_variable} is set but DATABASE_URL is not; this signer reads the \
                             database location only from DATABASE_URL, \
                             e.g. postgres://user:password@host:5432/database"
                        )
                        .into(),
                    });
                }
            }
            Ok(DatabaseConfig::Sqlite {
                path: parse_env_string("DB_PATH", Some(":memory:"))?,
            })
        }
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
            "DATABASE_URL",
            "DB_HOST",
            "DB_TYPE",
            "MIGRATE_LEGACY_DATA",
            "BLOCKCHAIN_PROVIDER",
            "BLOCKCHAIN_API_KEY",
            "CHAIN_ID",
            "ACCOUNTS_CONTRACT_ADDRESS",
            "ODIS_PAYMENTS_CONTRACT_ADDRESS",
            "FULL_NODE_RETRY_COUNT",
            "FULL_NODE_RETRY_DELAY_MS",
            "FULL_NODE_TIMEOUT_MS",
            "ODIS_SIGNER_TIMEOUT",
            "QUERY_PRICE_PER_CUSD",
            "KEYSTORE_GOOGLE_PROJECT_ID",
            "REQUEST_PRUNING_DAYS",
            "REQUEST_PRUNING_INTERVAL_SECS",
            "LOG_FORMAT",
            "SHOULD_MOCK_ACCOUNT_SERVICE",
            "SHOULD_MOCK_REQUEST_SERVICE",
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
        assert_eq!(
            config.database,
            DatabaseConfig::Sqlite {
                path: ":memory:".to_string()
            }
        );
        assert!(!config.migrate_legacy_data);
        assert!(config.blockchain_provider.is_none());
        assert!(config.blockchain_api_key.is_none());
        assert_eq!(config.chain_id, 11142220);
        assert_eq!(config.accounts_contract_address, None);
        assert_eq!(config.odis_payments_contract_address, None);
        assert_eq!(config.full_node_retry_count, 5);
        assert_eq!(config.full_node_retry_delay_ms, 100);
        assert_eq!(config.full_node_timeout_ms, 1000);
        assert_eq!(config.timeout_ms, 5000);
        assert!((config.query_price_per_cusd - 0.001).abs() < f64::EPSILON);
        assert!(config.google_project_id.is_none());
        assert_eq!(config.request_pruning_days, 7);
        assert_eq!(config.request_pruning_interval_secs, 86400);
        assert_eq!(config.log_format, LogFormat::Text);
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
            set("BLOCKCHAIN_API_KEY", "forno-key");
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
            set("FULL_NODE_TIMEOUT_MS", "2500");
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
        assert_eq!(
            config.database,
            DatabaseConfig::Sqlite {
                path: "/tmp/test.db".to_string()
            }
        );
        assert_eq!(
            config.blockchain_provider.as_deref(),
            Some("https://rpc.example.com")
        );
        assert_eq!(config.blockchain_api_key.as_deref(), Some("forno-key"));
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
        assert_eq!(config.full_node_timeout_ms, 2500);
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
    fn log_format_is_parsed_case_insensitively_and_rejects_unknown_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("LOG_FORMAT", "JSON");
        }
        assert_eq!(Config::from_env().unwrap().log_format, LogFormat::Json);

        unsafe { set("LOG_FORMAT", "stackdriver") };
        assert!(matches!(
            Config::from_env().unwrap_err(),
            ConfigError::InvalidValue { name, .. } if name == "LOG_FORMAT"
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
        // The TS signer reads `1` as false; reading it as true would switch
        // the API on with the same manifest.
        for value in ["yes", "1", "0"] {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set("PHONE_NUMBER_PRIVACY_API_ENABLED", value);
            }

            assert!(
                matches!(
                    Config::from_env().unwrap_err(),
                    ConfigError::InvalidValue { ref name, .. }
                        if name == "PHONE_NUMBER_PRIVACY_API_ENABLED"
                ),
                "{value}"
            );
        }
        for (value, expected) in [("true", true), ("TRUE", true), ("false", false)] {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set("PHONE_NUMBER_PRIVACY_API_ENABLED", value);
            }
            assert_eq!(Config::from_env().unwrap().pnp_api_enabled, expected);
        }
    }

    #[test]
    fn ts_signer_mock_switches_are_an_error_when_on() {
        let _lock = ENV_LOCK.lock().unwrap();
        for ts_variable in ["SHOULD_MOCK_ACCOUNT_SERVICE", "SHOULD_MOCK_REQUEST_SERVICE"] {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set(ts_variable, "true");
            }
            assert!(
                matches!(
                    Config::from_env().unwrap_err(),
                    ConfigError::InvalidValue { ref name, .. } if name == ts_variable
                ),
                "{ts_variable}"
            );

            // A manifest that switches the mock off says what this signer does anyway.
            unsafe { set(ts_variable, "false") };
            assert!(Config::from_env().is_ok(), "{ts_variable}=false");
        }
    }

    /// A manifest written for the TS signer configures the database through
    /// DB_HOST and friends, which this signer does not read.
    #[test]
    fn ts_signer_database_variables_without_database_url_are_an_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        for name in ["DB_HOST", "DB_TYPE"] {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set(name, "postgres");
            }
            let err = Config::from_env().unwrap_err();
            assert!(
                matches!(&err, ConfigError::InvalidValue { name, .. } if name == "DATABASE_URL"),
                "{name}: {err}"
            );

            unsafe { set("DATABASE_URL", "postgres://localhost/odis") };
            assert!(Config::from_env().is_ok(), "{name} with DATABASE_URL");
        }
    }

    #[test]
    fn empty_string_env_vars_are_treated_as_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("BLOCKCHAIN_PROVIDER", "");
            set("BLOCKCHAIN_API_KEY", "");
            set("KEYSTORE_GOOGLE_PROJECT_ID", "");
        }

        let config = Config::from_env().unwrap();
        assert!(config.blockchain_provider.is_none());
        assert!(config.blockchain_api_key.is_none());
        assert!(config.google_project_id.is_none());
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("ODIS_SIGNER_TIMEOUT", "0");
        }

        let result = Config::from_env();
        assert!(
            matches!(result.unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "ODIS_SIGNER_TIMEOUT")
        );

        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("ODIS_SIGNER_TIMEOUT", "-1");
        }

        let result = Config::from_env();
        assert!(
            matches!(result.unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "ODIS_SIGNER_TIMEOUT")
        );

        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("FULL_NODE_TIMEOUT_MS", "0");
        }

        let result = Config::from_env();
        assert!(
            matches!(result.unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "FULL_NODE_TIMEOUT_MS")
        );
    }

    #[test]
    fn invalid_query_price_is_rejected() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("QUERY_PRICE_PER_CUSD", "-1.0");
        }
        assert!(
            matches!(Config::from_env().unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "QUERY_PRICE_PER_CUSD")
        );

        unsafe { set("QUERY_PRICE_PER_CUSD", "nan") };
        assert!(
            matches!(Config::from_env().unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "QUERY_PRICE_PER_CUSD")
        );

        unsafe { set("QUERY_PRICE_PER_CUSD", "inf") };
        assert!(
            matches!(Config::from_env().unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "QUERY_PRICE_PER_CUSD")
        );
    }

    #[test]
    fn zero_key_version_is_rejected() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION", "0");
        }

        let result = Config::from_env();
        assert!(
            matches!(result.unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION")
        );
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

    #[test]
    fn database_url_selects_postgres() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set(
                "DATABASE_URL",
                "postgres://user:pw@db.example.com:5432/odis",
            );
        }

        let config = Config::from_env().unwrap();
        assert_eq!(
            config.database,
            DatabaseConfig::Postgres {
                url: "postgres://user:pw@db.example.com:5432/odis".to_string()
            }
        );
    }

    #[test]
    fn database_url_accepts_postgresql_scheme() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("DATABASE_URL", "postgresql://localhost/odis");
        }

        let config = Config::from_env().unwrap();
        assert!(matches!(config.database, DatabaseConfig::Postgres { .. }));
    }

    #[test]
    fn database_url_sqlite_dsn_forms() {
        let _lock = ENV_LOCK.lock().unwrap();

        for (dsn, expected_path) in [
            ("sqlite://data/signer.db", "data/signer.db"),
            ("sqlite:relative.db", "relative.db"),
            ("sqlite::memory:", ":memory:"),
            // Scheme is case-insensitive; the path keeps its original casing.
            ("SQLite://Data/Signer.DB", "Data/Signer.DB"),
            ("SQLITE::memory:", ":memory:"),
        ] {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set("DATABASE_URL", dsn);
            }
            let config = Config::from_env().unwrap();
            assert_eq!(
                config.database,
                DatabaseConfig::Sqlite {
                    path: expected_path.to_string()
                },
                "dsn {dsn} should map to path {expected_path}"
            );
        }
    }

    #[test]
    fn database_url_takes_precedence_over_db_path() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("DB_PATH", "/tmp/ignored.db");
            set("DATABASE_URL", "postgres://localhost/odis");
        }

        let config = Config::from_env().unwrap();
        assert!(matches!(config.database, DatabaseConfig::Postgres { .. }));
    }

    #[test]
    fn empty_database_url_falls_back_to_db_path() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("DATABASE_URL", "");
            set("DB_PATH", "/tmp/fallback.db");
        }

        let config = Config::from_env().unwrap();
        assert_eq!(
            config.database,
            DatabaseConfig::Sqlite {
                path: "/tmp/fallback.db".to_string()
            }
        );
    }

    #[test]
    fn unsupported_database_url_scheme_is_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("DATABASE_URL", "mysql://localhost/odis");
        }

        assert!(
            matches!(Config::from_env().unwrap_err(), ConfigError::InvalidValue { ref name, .. } if name == "DATABASE_URL")
        );
    }

    /// The error is written to the logs at startup, so it must not repeat a
    /// URL that can hold a password.
    #[test]
    fn unsupported_database_url_error_does_not_show_the_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        let cases = [
            (
                "postgre://odis:hunter2@db-host/odis",
                "unsupported scheme 'postgre'",
            ),
            ("hunter2@db-host/odis", "missing scheme"),
            ("hunter2@db-host:5432/odis", "missing scheme"),
        ];
        for (url, problem) in cases {
            unsafe {
                clear_env();
                set("KEYSTORE_TYPE", "Mock");
                set("DATABASE_URL", url);
            }
            let err = Config::from_env().unwrap_err();
            // `main` prints the Debug form; the Display form goes to callers.
            for text in [format!("{err}"), format!("{err:?}")] {
                assert!(text.contains(problem), "{url}: {text}");
                assert!(!text.contains("hunter2"), "{url}: {text}");
                assert!(!text.contains("db-host"), "{url}: {text}");
            }
        }
    }

    #[test]
    fn migrate_legacy_data_flag() {
        let _lock = ENV_LOCK.lock().unwrap();
        unsafe {
            clear_env();
            set("KEYSTORE_TYPE", "Mock");
            set("MIGRATE_LEGACY_DATA", "true");
        }

        assert!(Config::from_env().unwrap().migrate_legacy_data);
    }
}
