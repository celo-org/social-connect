//! The combiner's configuration, read from the environment. Names and
//! defaults are the TS combiner's; every setting that would make the combiner
//! run wrong stops the start with a message that says what to change.

use crate::signature::{KeyVersion, KeyVersionError};
use crate::signers::SignerEndpoint;
use odis_signer::config::{ConfigError, LogFormat, parse_env, parse_env_string, parse_log_format};
use serde::{Deserialize, de::DeserializeOwned};
use std::{env, time::Duration};

/// One signer the combiner calls.
#[derive(Debug, Clone, PartialEq)]
pub struct SignerEntry {
    pub url: String,
    /// The share index every partial signature from this signer must carry.
    /// When unset, any index is accepted.
    pub share_index: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server_port: u16,
    /// Value of the `app` label on every metric.
    pub service_name: String,
    pub signers: Vec<SignerEntry>,
    pub signer_timeout: Duration,
    pub key_versions: Vec<KeyVersion>,
    pub current_key_version: u32,
    pub blockchain_provider: String,
    pub blockchain_api_key: Option<String>,
    pub chain_id: u64,
    /// Limit for one full node lookup, retries included.
    pub full_node_timeout: Duration,
    /// Retries after the first try.
    pub full_node_retry_count: u32,
    pub full_node_retry_delay: Duration,
    pub log_format: LogFormat,
}

/// A signer list entry as written in `PNP_ODIS_SERVICES_SIGNERS`. Other fields,
/// such as the TS combiner's `fallbackUrl`, are ignored.
#[derive(Deserialize)]
struct RawSigner {
    url: String,
    #[serde(rename = "shareIndex")]
    share_index: Option<u32>,
}

/// A key version list entry as written in `PNP_KEYS_VERSIONS`.
#[derive(Deserialize)]
struct RawKeyVersion {
    #[serde(rename = "keyVersion")]
    key_version: u32,
    threshold: usize,
    polynomial: String,
    #[serde(rename = "pubKey")]
    pub_key: String,
}

fn invalid(name: &str, message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        name: name.to_string(),
        source: message.into().into(),
    }
}

fn parse_json_env<T: DeserializeOwned>(name: &str, expected: &str) -> Result<T, ConfigError> {
    let text = parse_env_string(name, None)?;
    serde_json::from_str(&text).map_err(|e| invalid(name, format!("{expected}: {e}")))
}

fn parse_duration_ms(name: &str, default: u64) -> Result<Duration, ConfigError> {
    let millis: u64 = parse_env(name, Some(default))?;
    if millis == 0 {
        return Err(invalid(name, "must be greater than 0"));
    }
    Ok(Duration::from_millis(millis))
}

fn parse_signers() -> Result<Vec<SignerEntry>, ConfigError> {
    const NAME: &str = "PNP_ODIS_SERVICES_SIGNERS";
    let raw: Vec<RawSigner> = parse_json_env(
        NAME,
        "must be a JSON list of objects with a string url, such as [{\"url\":\"http://signer\"}]",
    )?;
    let signers: Vec<SignerEntry> = raw
        .into_iter()
        .map(|s| SignerEntry {
            url: s.url,
            share_index: s.share_index,
        })
        .collect();

    // Only checked: the call appends the endpoint to the URL as written, as TS does. The check
    // parses that string, because the parser drops trailing spaces from the URL alone and the
    // call would then fail on a space inside the host.
    for (i, signer) in signers.iter().enumerate() {
        let is_http =
            reqwest::Url::parse(&format!("{}{}", signer.url, SignerEndpoint::Sign.path()))
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        if !is_http {
            return Err(invalid(
                NAME,
                format!(
                    "signer {} has url {:?}, which is not an http:// or https:// URL; \
                     set the signer's full URL, such as http://signer:8080",
                    i + 1,
                    signer.url
                ),
            ));
        }
    }

    for (i, signer) in signers.iter().enumerate() {
        let Some(index) = signer.share_index else {
            continue;
        };
        if let Some(first) = signers[..i]
            .iter()
            .position(|s| s.share_index == Some(index))
        {
            return Err(invalid(
                NAME,
                format!(
                    "signers {} and {} both have shareIndex {index}; \
                     give each signer its own shareIndex",
                    first + 1,
                    i + 1
                ),
            ));
        }
    }
    Ok(signers)
}

fn key_version_error(name: &str, key_version: u32, error: KeyVersionError) -> ConfigError {
    let hint = match error {
        KeyVersionError::PolynomialEncoding => "; check the hex string",
        KeyVersionError::PublicKeyEncoding => "; check the base64 string",
        KeyVersionError::ThresholdMismatch {
            threshold,
            coefficients,
        } => {
            return invalid(
                name,
                format!(
                    "key version {key_version} has threshold {threshold} but its polynomial \
                     has {coefficients} coefficients; set threshold to {coefficients}"
                ),
            );
        }
        KeyVersionError::PublicKeyIsIdentity => "; use the polynomial's constant term",
        KeyVersionError::PublicKeyNotConstantTerm => "; use the key generated with this polynomial",
    };
    invalid(name, format!("key version {key_version}: {error}{hint}"))
}

/// Decodes the key version list and checks it against the number of signers.
/// The threshold range is checked before the decoding, so a threshold above
/// the signers is reported as that and not as a polynomial that does not fit.
fn parse_key_versions(signer_count: usize) -> Result<Vec<KeyVersion>, ConfigError> {
    const NAME: &str = "PNP_KEYS_VERSIONS";
    let raw: Vec<RawKeyVersion> = parse_json_env(
        NAME,
        "must be a JSON list of objects with keyVersion, threshold, polynomial and pubKey",
    )?;

    let mut key_versions: Vec<KeyVersion> = Vec::with_capacity(raw.len());
    for entry in raw {
        if entry.threshold == 0 || entry.threshold > signer_count {
            return Err(invalid(
                NAME,
                format!(
                    "key version {} has threshold {}; \
                     set a threshold from 1 to {signer_count}, the number of signers",
                    entry.key_version, entry.threshold
                ),
            ));
        }
        let key_version = KeyVersion::decode(
            entry.key_version,
            entry.threshold,
            &entry.polynomial,
            &entry.pub_key,
        )
        .map_err(|e| key_version_error(NAME, entry.key_version, e))?;
        if key_versions
            .iter()
            .any(|k| k.key_version == key_version.key_version)
        {
            return Err(invalid(
                NAME,
                format!(
                    "key version {} is listed twice; keep one entry for each key version",
                    key_version.key_version
                ),
            ));
        }
        key_versions.push(key_version);
    }
    Ok(key_versions)
}

impl Config {
    /// Loads the configuration from environment variables. Call
    /// `dotenvy::dotenv()` first for `.env` support.
    ///
    /// The TS combiner's switches `PNP_SHOULD_AUTHENTICATE`,
    /// `PNP_SHOULD_CHECK_QUOTA`, `PHONE_NUMBER_PRIVACY_API_ENABLED`,
    /// `PNP_MOCK_DEK` and the `DOMAIN*` variables are not read.
    /// `PNP_SHOULD_MOCK_ACCOUNT_SERVICE=true` stops the start.
    pub fn from_env() -> Result<Config, ConfigError> {
        // The TS combiner answers every DEK lookup with `PNP_MOCK_DEK` on this switch. This
        // combiner has no such switch, and would query the full node where the operator expects
        // mock answers.
        // TODO: remove once the migration from the TS combiner is finished; only TS manifests
        // set this variable.
        if env::var("PNP_SHOULD_MOCK_ACCOUNT_SERVICE").is_ok_and(|v| v.eq_ignore_ascii_case("true"))
        {
            return Err(invalid(
                "PNP_SHOULD_MOCK_ACCOUNT_SERVICE",
                "this combiner has no mock account service to switch to; remove the variable",
            ));
        }

        let server_port = parse_env("SERVER_PORT", Some(8081))?;
        let service_name = parse_env_string("SERVICE_NAME", Some("odis-combiner"))?;
        let signers = parse_signers()?;
        let signer_timeout = parse_duration_ms("PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS", 5000)?;
        let key_versions = parse_key_versions(signers.len())?;

        let current_key_version = parse_env("PNP_KEYS_CURRENT_VERSION", Some(1))?;
        if !key_versions
            .iter()
            .any(|k| k.key_version == current_key_version)
        {
            let listed: Vec<String> = key_versions
                .iter()
                .map(|k| k.key_version.to_string())
                .collect();
            return Err(invalid(
                "PNP_KEYS_CURRENT_VERSION",
                format!(
                    "key version {current_key_version} is not in PNP_KEYS_VERSIONS, \
                     which lists {}",
                    listed.join(", ")
                ),
            ));
        }

        let blockchain_provider = parse_env_string("BLOCKCHAIN_PROVIDER", None)?;
        if blockchain_provider.is_empty() {
            return Err(invalid(
                "BLOCKCHAIN_PROVIDER",
                "must not be empty; set the URL of a full node",
            ));
        }

        Ok(Config {
            server_port,
            service_name,
            signers,
            signer_timeout,
            key_versions,
            current_key_version,
            blockchain_provider,
            blockchain_api_key: env::var("BLOCKCHAIN_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            chain_id: parse_env("CHAIN_ID", Some(11142220))?,
            full_node_timeout: parse_duration_ms("PNP_FULL_NODE_TIMEOUT_MS", 1000)?,
            full_node_retry_count: parse_env("PNP_FULL_NODE_RETRY_COUNT", Some(5))?,
            full_node_retry_delay: Duration::from_millis(parse_env(
                "PNP_FULL_NODE_DELAY_MS",
                Some(100),
            )?),
            log_format: parse_log_format()?,
        })
    }

    pub fn key_version(&self, version: u32) -> Option<&KeyVersion> {
        self.key_versions.iter().find(|k| k.key_version == version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::PublicKey;
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
    use std::{env, sync::Mutex, time::Duration};
    use threshold_bls::group::Element;

    // Env vars are process-global, so tests that modify them must not run in parallel.
    // SAFETY: All env-modifying tests hold ENV_LOCK, ensuring single-threaded access.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const ALL_VARIABLES: [&str; 18] = [
        "SERVER_PORT",
        "SERVICE_NAME",
        "PNP_ODIS_SERVICES_SIGNERS",
        "PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS",
        "PNP_KEYS_VERSIONS",
        "PNP_KEYS_CURRENT_VERSION",
        "BLOCKCHAIN_PROVIDER",
        "BLOCKCHAIN_API_KEY",
        "CHAIN_ID",
        "PNP_FULL_NODE_TIMEOUT_MS",
        "PNP_FULL_NODE_RETRY_COUNT",
        "PNP_FULL_NODE_DELAY_MS",
        "LOG_FORMAT",
        "PNP_SHOULD_AUTHENTICATE",
        "PNP_SHOULD_CHECK_QUOTA",
        "PHONE_NUMBER_PRIVACY_API_ENABLED",
        "PNP_SHOULD_MOCK_ACCOUNT_SERVICE",
        "PNP_MOCK_DEK",
    ];

    const SIGNERS: &str = r#"[{"url":"http://s0"},{"url":"http://s1"},{"url":"http://s2"}]"#;

    struct DevKey {
        key_version: u32,
        polynomial: String,
        pub_key: String,
    }

    /// The dev key versions 1 to 3 (threshold 2) from the combine vectors.
    fn dev_keys() -> Vec<DevKey> {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/combine_vectors.json")).unwrap();
        vectors["keys"]
            .as_array()
            .unwrap()
            .iter()
            .take(3)
            .map(|key| DevKey {
                key_version: key["keyVersion"].as_u64().unwrap() as u32,
                polynomial: key["polynomial"].as_str().unwrap().to_string(),
                pub_key: key["pubKey"].as_str().unwrap().to_string(),
            })
            .collect()
    }

    /// A key version list entry; the unknown field `extra` must be ignored.
    fn key_entry(key_version: u32, threshold: usize, polynomial: &str, pub_key: &str) -> String {
        format!(
            r#"{{"keyVersion":{key_version},"threshold":{threshold},"polynomial":"{polynomial}","pubKey":"{pub_key}","extra":1}}"#
        )
    }

    fn valid_key_versions() -> String {
        let entries: Vec<String> = dev_keys()
            .iter()
            .map(|k| key_entry(k.key_version, 2, &k.polynomial, &k.pub_key))
            .collect();
        format!("[{}]", entries.join(","))
    }

    unsafe fn clear_env() {
        for key in ALL_VARIABLES {
            unsafe { env::remove_var(key) };
        }
    }

    /// Runs `Config::from_env` with the three required variables set to valid
    /// values, then `overrides` applied: `None` removes a variable.
    fn from_env_with(overrides: &[(&str, Option<&str>)]) -> Result<Config, ConfigError> {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            clear_env();
            env::set_var("PNP_ODIS_SERVICES_SIGNERS", SIGNERS);
            env::set_var("PNP_KEYS_VERSIONS", valid_key_versions());
            env::set_var("BLOCKCHAIN_PROVIDER", "https://rpc.example.com");
            for (name, value) in overrides {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
        }
        let result = Config::from_env();
        unsafe { clear_env() };
        result
    }

    #[test]
    fn defaults_are_the_ts_combiners() {
        let config = from_env_with(&[]).unwrap();

        assert_eq!(config.server_port, 8081);
        assert_eq!(config.service_name, "odis-combiner");
        assert_eq!(config.signers.len(), 3);
        assert_eq!(config.signer_timeout, Duration::from_millis(5000));
        assert_eq!(config.key_versions.len(), 3);
        assert_eq!(config.current_key_version, 1);
        assert_eq!(config.blockchain_provider, "https://rpc.example.com");
        assert_eq!(config.blockchain_api_key, None);
        assert_eq!(config.chain_id, 11142220);
        assert_eq!(config.full_node_timeout, Duration::from_millis(1000));
        assert_eq!(config.full_node_retry_count, 5);
        assert_eq!(config.full_node_retry_delay, Duration::from_millis(100));
        assert_eq!(config.log_format, LogFormat::Text);
        assert_eq!(config.key_version(2).unwrap().key_version, 2);
        assert!(config.key_version(9).is_none());
    }

    #[test]
    fn set_values_replace_the_defaults() {
        let config = from_env_with(&[
            ("SERVER_PORT", Some("9090")),
            ("SERVICE_NAME", Some("combiner-2")),
            ("PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS", Some("750")),
            ("PNP_KEYS_CURRENT_VERSION", Some("3")),
            ("BLOCKCHAIN_API_KEY", Some("forno-key")),
            ("CHAIN_ID", Some("42220")),
            ("PNP_FULL_NODE_TIMEOUT_MS", Some("2500")),
            ("PNP_FULL_NODE_RETRY_COUNT", Some("2")),
            ("PNP_FULL_NODE_DELAY_MS", Some("20")),
            ("LOG_FORMAT", Some("JSON")),
        ])
        .unwrap();

        assert_eq!(config.server_port, 9090);
        assert_eq!(config.service_name, "combiner-2");
        assert_eq!(config.signer_timeout, Duration::from_millis(750));
        assert_eq!(config.current_key_version, 3);
        assert_eq!(config.blockchain_api_key.as_deref(), Some("forno-key"));
        assert_eq!(config.chain_id, 42220);
        assert_eq!(config.full_node_timeout, Duration::from_millis(2500));
        assert_eq!(config.full_node_retry_count, 2);
        assert_eq!(config.full_node_retry_delay, Duration::from_millis(20));
        assert_eq!(config.log_format, LogFormat::Json);
    }

    #[test]
    fn signer_entries_take_an_optional_share_index() {
        let config = from_env_with(&[(
            "PNP_ODIS_SERVICES_SIGNERS",
            Some(
                r#"[{"url":"http://a","fallbackUrl":"http://b","shareIndex":2,"other":1},{"url":"https://c/"}]"#,
            ),
        )])
        .unwrap();

        assert_eq!(
            config.signers,
            [
                SignerEntry {
                    url: "http://a".to_string(),
                    share_index: Some(2)
                },
                // Kept as written, trailing slash included.
                SignerEntry {
                    url: "https://c/".to_string(),
                    share_index: None
                },
            ]
        );
    }

    /// A case name, the variables it overrides (`None` removes one), and the
    /// exact error text.
    type RefusalCase = (&'static str, Vec<(&'static str, Option<String>)>, String);

    #[test]
    fn startup_refusals() {
        let keys = dev_keys();
        let (k1, k2) = (&keys[0], &keys[1]);
        let entry = |version, threshold, poly: &str, pub_key: &str| {
            format!("[{}]", key_entry(version, threshold, poly, pub_key))
        };
        let identity = BASE64.encode(bincode::serialize(&PublicKey::zero()).unwrap());

        let cases: Vec<RefusalCase> =
            vec![
            (
                "signers missing",
                vec![("PNP_ODIS_SERVICES_SIGNERS", None)],
                "missing required env var: PNP_ODIS_SERVICES_SIGNERS".into(),
            ),
            (
                "signers not JSON",
                vec![("PNP_ODIS_SERVICES_SIGNERS", Some("http://a".into()))],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: must be a JSON list of objects \
                 with a string url, such as [{\"url\":\"http://signer\"}]: \
                 expected value at line 1 column 1"
                    .into(),
            ),
            (
                "signers without a string url",
                vec![("PNP_ODIS_SERVICES_SIGNERS", Some(r#"[{"url":1}]"#.into()))],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: must be a JSON list of objects \
                 with a string url, such as [{\"url\":\"http://signer\"}]: \
                 invalid type: integer `1`, expected a string at line 1 column 9"
                    .into(),
            ),
            (
                "signers not a list",
                vec![("PNP_ODIS_SERVICES_SIGNERS", Some(r#"{"url":"a"}"#.into()))],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: must be a JSON list of objects \
                 with a string url, such as [{\"url\":\"http://signer\"}]: \
                 invalid type: map, expected a sequence at line 1 column 0"
                    .into(),
            ),
            (
                "signer url empty",
                vec![(
                    "PNP_ODIS_SERVICES_SIGNERS",
                    Some(r#"[{"url":"http://a"},{"url":""}]"#.into()),
                )],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: signer 2 has url \"\", which is \
                 not an http:// or https:// URL; set the signer's full URL, such as \
                 http://signer:8080"
                    .into(),
            ),
            (
                "signer url without a scheme",
                vec![(
                    "PNP_ODIS_SERVICES_SIGNERS",
                    Some(r#"[{"url":"signer.example.com"}]"#.into()),
                )],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: signer 1 has url \
                 \"signer.example.com\", which is not an http:// or https:// URL; set the \
                 signer's full URL, such as http://signer:8080"
                    .into(),
            ),
            (
                "signer url with a trailing space",
                vec![(
                    "PNP_ODIS_SERVICES_SIGNERS",
                    Some(r#"[{"url":"http://a"},{"url":"http://signer:8080 "}]"#.into()),
                )],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: signer 2 has url \
                 \"http://signer:8080 \", which is not an http:// or https:// URL; set the \
                 signer's full URL, such as http://signer:8080"
                    .into(),
            ),
            (
                "signer url with another scheme",
                vec![(
                    "PNP_ODIS_SERVICES_SIGNERS",
                    Some(r#"[{"url":"ftp://signer"}]"#.into()),
                )],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: signer 1 has url \
                 \"ftp://signer\", which is not an http:// or https:// URL; set the signer's \
                 full URL, such as http://signer:8080"
                    .into(),
            ),
            (
                "share index used twice",
                vec![(
                    "PNP_ODIS_SERVICES_SIGNERS",
                    Some(
                        r#"[{"url":"http://a","shareIndex":1},{"url":"http://b"},{"url":"http://c","shareIndex":1}]"#
                            .into(),
                    ),
                )],
                "invalid value for PNP_ODIS_SERVICES_SIGNERS: signers 1 and 3 both have \
                 shareIndex 1; give each signer its own shareIndex"
                    .into(),
            ),
            (
                "timeout zero",
                vec![("PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS", Some("0".into()))],
                "invalid value for PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS: must be greater than 0"
                    .into(),
            ),
            (
                "timeout not a number",
                vec![("PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS", Some("abc".into()))],
                "invalid value for PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS: \
                 invalid digit found in string"
                    .into(),
            ),
            (
                "full node timeout zero",
                vec![("PNP_FULL_NODE_TIMEOUT_MS", Some("0".into()))],
                "invalid value for PNP_FULL_NODE_TIMEOUT_MS: must be greater than 0".into(),
            ),
            (
                "key versions missing",
                vec![("PNP_KEYS_VERSIONS", None)],
                "missing required env var: PNP_KEYS_VERSIONS".into(),
            ),
            (
                "key versions not JSON",
                vec![("PNP_KEYS_VERSIONS", Some("keys".into()))],
                "invalid value for PNP_KEYS_VERSIONS: must be a JSON list of objects with \
                 keyVersion, threshold, polynomial and pubKey: expected value at line 1 column 1"
                    .into(),
            ),
            (
                "threshold zero",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 0, &k1.polynomial, &k1.pub_key)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1 has threshold 0; \
                 set a threshold from 1 to 3, the number of signers"
                    .into(),
            ),
            (
                "threshold above the signers",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 4, &k1.polynomial, &k1.pub_key)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1 has threshold 4; \
                 set a threshold from 1 to 3, the number of signers"
                    .into(),
            ),
            (
                "polynomial not decodable",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 2, "zz", &k1.pub_key)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1: the polynomial is not the \
                 exact encoding of a polynomial; check the hex string"
                    .into(),
            ),
            (
                "public key not decodable",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 2, &k1.polynomial, "AAAA")))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1: the public key is not the \
                 exact encoding of a public key; check the base64 string"
                    .into(),
            ),
            (
                "threshold does not fit the polynomial",
                vec![("PNP_KEYS_VERSIONS", Some(entry(2, 3, &k2.polynomial, &k2.pub_key)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 2 has threshold 3 but its \
                 polynomial has 2 coefficients; set threshold to 2"
                    .into(),
            ),
            (
                "public key is the identity",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 2, &k1.polynomial, &identity)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1: the public key is the \
                 identity point; use the polynomial's constant term"
                    .into(),
            ),
            (
                "public key is not the constant term",
                vec![("PNP_KEYS_VERSIONS", Some(entry(1, 2, &k1.polynomial, &k2.pub_key)))],
                "invalid value for PNP_KEYS_VERSIONS: key version 1: the public key is not the \
                 polynomial's constant term; use the key generated with this polynomial"
                    .into(),
            ),
            (
                "key version listed twice",
                vec![(
                    "PNP_KEYS_VERSIONS",
                    Some(format!(
                        "[{},{}]",
                        key_entry(1, 2, &k1.polynomial, &k1.pub_key),
                        key_entry(1, 2, &k2.polynomial, &k2.pub_key)
                    )),
                )],
                "invalid value for PNP_KEYS_VERSIONS: key version 1 is listed twice; \
                 keep one entry for each key version"
                    .into(),
            ),
            (
                "current version not listed",
                vec![("PNP_KEYS_CURRENT_VERSION", Some("9".into()))],
                "invalid value for PNP_KEYS_CURRENT_VERSION: key version 9 is not in \
                 PNP_KEYS_VERSIONS, which lists 1, 2, 3"
                    .into(),
            ),
            (
                "provider unset",
                vec![("BLOCKCHAIN_PROVIDER", None)],
                "missing required env var: BLOCKCHAIN_PROVIDER".into(),
            ),
            (
                "provider empty",
                vec![("BLOCKCHAIN_PROVIDER", Some("".into()))],
                "invalid value for BLOCKCHAIN_PROVIDER: must not be empty; set the URL of a \
                 full node"
                    .into(),
            ),
            (
                "log format stackdriver",
                vec![("LOG_FORMAT", Some("stackdriver".into()))],
                "invalid value for LOG_FORMAT: unknown log format: 'stackdriver', \
                 expected 'text' or 'json'"
                    .into(),
            ),
            (
                "log format human",
                vec![("LOG_FORMAT", Some("human".into()))],
                "invalid value for LOG_FORMAT: unknown log format: 'human', \
                 expected 'text' or 'json'"
                    .into(),
            ),
            (
                "port not a number",
                vec![("SERVER_PORT", Some("abc".into()))],
                "invalid value for SERVER_PORT: invalid digit found in string".into(),
            ),
            (
                "mock account service on",
                vec![("PNP_SHOULD_MOCK_ACCOUNT_SERVICE", Some("true".into()))],
                "invalid value for PNP_SHOULD_MOCK_ACCOUNT_SERVICE: this combiner has no mock \
                 account service to switch to; remove the variable"
                    .into(),
            ),
            (
                "mock account service on in upper case",
                vec![("PNP_SHOULD_MOCK_ACCOUNT_SERVICE", Some("TRUE".into()))],
                "invalid value for PNP_SHOULD_MOCK_ACCOUNT_SERVICE: this combiner has no mock \
                 account service to switch to; remove the variable"
                    .into(),
            ),
        ];

        for (name, overrides, expected) in cases {
            let overrides: Vec<(&str, Option<&str>)> = overrides
                .iter()
                .map(|(variable, value)| (*variable, value.as_deref()))
                .collect();
            let err = from_env_with(&overrides).unwrap_err();
            assert_eq!(err.to_string(), expected, "{name}");
        }
    }

    #[test]
    fn variables_without_effect_change_nothing() {
        let without = from_env_with(&[]).unwrap();
        let with = from_env_with(&[
            ("PNP_SHOULD_AUTHENTICATE", Some("false")),
            ("PNP_SHOULD_CHECK_QUOTA", Some("true")),
            ("PHONE_NUMBER_PRIVACY_API_ENABLED", Some("false")),
            ("PNP_SHOULD_MOCK_ACCOUNT_SERVICE", Some("false")),
            ("PNP_MOCK_DEK", Some("0x01")),
            ("DOMAINS_API_ENABLED", Some("true")),
        ])
        .unwrap();

        assert_eq!(format!("{with:?}"), format!("{without:?}"));
    }
}
