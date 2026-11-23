//! Reads an account's data encryption key (DEK) from the Accounts contract on
//! the full node.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::network::Ethereum;
use alloy::primitives::{Address, address};
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use alloy::sol;
use alloy::transports::http::reqwest;
use async_trait::async_trait;
use metrics::{counter, histogram};
use moka::future::Cache;
use tracing::warn;

use crate::config::Config;
use crate::errors::{FAILURE_TO_GET_DEK, FULL_NODE_ERROR};
use crate::metrics::{BLOCKCHAIN_ERRORS, ERRORS, FULL_NODE_LATENCY};

sol! {
    #[sol(rpc)]
    interface IAccounts {
        function getDataEncryptionKey(address account) external view returns (bytes);
    }
}

/// Celo Accounts contract on Celo mainnet.
const MAINNET_ACCOUNTS: Address = address!("0x7d21685C17607338b313a7174bAb6620baD0aaB7");
/// Celo Accounts contract on Celo Sepolia.
const SEPOLIA_ACCOUNTS: Address = address!("0x44957232699ca060B607E77083bDACD350d6b6d1");

/// The Accounts contract of a chain. The TS combiner has no setting for the
/// address, so only the chains it ships with are served.
fn accounts_address(chain_id: u64) -> Option<Address> {
    match chain_id {
        42220 => Some(MAINNET_ACCOUNTS),
        11142220 => Some(SEPOLIA_ACCOUNTS),
        _ => None,
    }
}

/// Name of the header that carries the full node API key, as the TS combiner sends it.
const API_KEY_HEADER: &str = "apiKey";

/// Value of the `code_segment` label of the full node latency histogram.
const CODE_SEGMENT: &str = "getDataEncryptionKey";

const CACHE_CAPACITY: u64 = 500;
const CACHE_TIME_TO_LIVE: Duration = Duration::from_secs(5);

#[async_trait]
pub trait DekLookup: Send + Sync {
    /// The account's DEK as lower-case hex without `0x`; empty when it has none.
    async fn dek(&self, account: Address) -> Result<String, DekLookupError>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DekLookupError {
    #[error("the full node could not answer the DEK lookup: {0}")]
    FullNode(String),
    #[error("the DEK lookup ran past the full node timeout")]
    Timeout,
}

/// Why the full node client cannot be built or does not fit the configuration.
#[derive(Debug, thiserror::Error)]
pub enum FullNodeError {
    #[error(
        "CHAIN_ID {0} is not supported: the Accounts contract address is known only for 42220 (Celo mainnet) and 11142220 (Celo Sepolia)"
    )]
    UnknownChain(u64),
    #[error("BLOCKCHAIN_PROVIDER is not a valid URL")]
    InvalidUrl,
    #[error("BLOCKCHAIN_API_KEY is not a valid HTTP header value")]
    InvalidApiKey,
    #[error("the HTTP client for full node calls could not be built: {0}")]
    Client(reqwest::Error),
    #[error("the full node at BLOCKCHAIN_PROVIDER did not report its chain ID")]
    NoChainId,
    #[error(
        "CHAIN_ID is {configured}, but the full node at BLOCKCHAIN_PROVIDER is on chain {full_node}"
    )]
    ChainIdMismatch { configured: u64, full_node: u64 },
}

/// Builds the HTTP client for full node requests. With an API key, every
/// request carries it in the `apiKey` header.
fn build_http_client(api_key: Option<&str>) -> Result<reqwest::Client, FullNodeError> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(api_key) = api_key {
        let mut value = reqwest::header::HeaderValue::from_str(api_key)
            .map_err(|_| FullNodeError::InvalidApiKey)?;
        // Keeps the key out of debug output of the header map.
        value.set_sensitive(true);
        headers.insert(API_KEY_HEADER, value);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(FullNodeError::Client)
}

pub struct FullNodeDekLookup {
    chain_id: u64,
    accounts: Address,
    provider: RootProvider<Ethereum>,
    retry_count: u32,
    retry_delay: Duration,
    /// Limit for one lookup, all of its retries included.
    timeout: Duration,
    /// Forms of `BLOCKCHAIN_PROVIDER` that error text must not carry: some
    /// providers put the API key in the URL, and transport errors name the URL.
    secrets: Vec<String>,
}

impl FullNodeDekLookup {
    pub fn new(config: &Config) -> Result<Self, FullNodeError> {
        let accounts = accounts_address(config.chain_id)
            .ok_or(FullNodeError::UnknownChain(config.chain_id))?;
        let url: reqwest::Url = config
            .blockchain_provider
            .parse()
            .map_err(|_| FullNodeError::InvalidUrl)?;
        // Longest first, so a full URL is replaced before the path inside it.
        let mut secrets = vec![config.blockchain_provider.clone(), url.to_string()];
        if url.path().len() > 1 {
            secrets.push(url.path().to_string());
        }
        // A URL that ends in `?` has an empty query, and replacing an empty string would put
        // the marker between every character.
        if let Some(query) = url.query().filter(|query| !query.is_empty()) {
            secrets.push(query.to_string());
        }
        let http_client = build_http_client(config.blockchain_api_key.as_deref())?;
        let provider = ProviderBuilder::default().connect_reqwest(http_client, url);
        Ok(Self {
            chain_id: config.chain_id,
            accounts,
            provider,
            retry_count: config.full_node_retry_count,
            retry_delay: config.full_node_retry_delay,
            timeout: config.full_node_timeout,
            secrets,
        })
    }

    /// The text of a full node error with the provider URL removed.
    fn redact(&self, error: &impl std::fmt::Display) -> String {
        let mut text = error.to_string();
        for secret in &self.secrets {
            text = text.replace(secret.as_str(), "<BLOCKCHAIN_PROVIDER>");
        }
        text
    }

    /// Fails unless the full node reports the configured chain ID. The chain ID
    /// selects the Accounts address, which holds no contract on another chain.
    pub async fn verify_chain_id(&self) -> Result<(), FullNodeError> {
        let lookup = retry_with_backoff(self.retry_count, self.retry_delay, || async {
            self.provider.get_chain_id().await.map_err(|e| {
                warn!(error = %self.redact(&e), "failed to fetch the full node's chain ID");
            })
        });
        let full_node = tokio::time::timeout(self.timeout, lookup)
            .await
            .map_err(|_| warn!("timed out fetching the full node's chain ID"))
            .and_then(|result| result)
            .map_err(|()| FullNodeError::NoChainId)?;
        if full_node != self.chain_id {
            return Err(FullNodeError::ChainIdMismatch {
                configured: self.chain_id,
                full_node,
            });
        }
        Ok(())
    }

    async fn read_dek(&self, account: Address) -> Result<String, DekLookupError> {
        let accounts = IAccounts::new(self.accounts, &self.provider);
        let lookup = retry_with_backoff(self.retry_count, self.retry_delay, || async {
            accounts
                .getDataEncryptionKey(account)
                .call()
                .await
                .map_err(|e| {
                    let error = self.redact(&e);
                    warn!(%error, %account, "failed to fetch DEK");
                    DekLookupError::FullNode(error)
                })
        });
        let dek = tokio::time::timeout(self.timeout, lookup)
            .await
            .map_err(|_| {
                warn!(%account, "timed out fetching DEK");
                DekLookupError::Timeout
            })??;
        Ok(hex::encode(&dek))
    }
}

#[async_trait]
impl DekLookup for FullNodeDekLookup {
    async fn dek(&self, account: Address) -> Result<String, DekLookupError> {
        let started = Instant::now();
        let result = self.read_dek(account).await;
        histogram!(FULL_NODE_LATENCY, "code_segment" => CODE_SEGMENT)
            .record(started.elapsed().as_secs_f64());
        if result.is_err() {
            counter!(ERRORS, "endpoint" => "NA", "error_type" => FULL_NODE_ERROR).increment(1);
            counter!(BLOCKCHAIN_ERRORS, "endpoint" => "NA", "error_type" => FAILURE_TO_GET_DEK)
                .increment(1);
        }
        result
    }
}

/// Keeps a lookup's answers for 5 seconds. Failures are not kept.
pub struct CachingDekLookup {
    inner: Arc<dyn DekLookup>,
    cache: Cache<Address, String>,
}

impl CachingDekLookup {
    pub fn new(inner: Arc<dyn DekLookup>) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_CAPACITY)
            .time_to_live(CACHE_TIME_TO_LIVE)
            .build();
        Self { inner, cache }
    }
}

#[async_trait]
impl DekLookup for CachingDekLookup {
    async fn dek(&self, account: Address) -> Result<String, DekLookupError> {
        let inner = self.inner.clone();
        self.cache
            .try_get_with(account, async move { inner.dek(account).await })
            .await
            .map_err(|e| (*e).clone())
    }
}

/// Retries an async operation with exponential backoff (1.5x multiplier).
async fn retry_with_backoff<F, Fut, T, E>(
    max_retries: u32,
    initial_delay: Duration,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delay = initial_delay;
    let mut last_err = match f().await {
        Ok(val) => return Ok(val),
        Err(e) => e,
    };
    for _ in 0..max_retries {
        tokio::time::sleep(delay).await;
        delay = delay.mul_f64(1.5);
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::config::Config;
    use alloy::primitives::{Address, Bytes};
    use alloy::sol_types::SolCall;
    use async_trait::async_trait;
    use axum::http::HeaderMap;
    use odis_signer::config::LogFormat;

    const SEPOLIA: u64 = 11142220;
    const MAINNET: u64 = 42220;

    /// What the stub full node answers to `eth_call`.
    #[derive(Clone)]
    enum CallReply {
        Dek(Vec<u8>),
        RpcError,
        Silent,
    }

    struct StubFullNode {
        url: String,
        eth_calls: Arc<AtomicU32>,
        headers: Arc<Mutex<Vec<HeaderMap>>>,
    }

    /// Starts a JSON-RPC server on a free local port. `chain_id` is the answer to
    /// `eth_chainId`; `None` leaves it unanswered.
    async fn spawn_full_node(chain_id: Option<u64>, reply: CallReply) -> StubFullNode {
        let eth_calls = Arc::new(AtomicU32::new(0));
        let headers = Arc::new(Mutex::new(Vec::new()));
        let (calls, seen) = (eth_calls.clone(), headers.clone());
        let app = axum::Router::new().fallback(
            move |request_headers: HeaderMap,
                  axum::Json(request): axum::Json<serde_json::Value>| {
                let (calls, seen, reply) = (calls.clone(), seen.clone(), reply.clone());
                async move {
                    seen.lock().unwrap().push(request_headers);
                    let id = request["id"].clone();
                    let result = match request["method"].as_str().unwrap() {
                        "eth_chainId" => match chain_id {
                            Some(chain_id) => format!("0x{chain_id:x}"),
                            None => std::future::pending().await,
                        },
                        "eth_call" => {
                            calls.fetch_add(1, Ordering::SeqCst);
                            match reply {
                                CallReply::Dek(dek) => format!(
                                    "0x{}",
                                    hex::encode(
                                        IAccounts::getDataEncryptionKeyCall::abi_encode_returns(
                                            &Bytes::from(dek)
                                        )
                                    )
                                ),
                                CallReply::RpcError => {
                                    return axum::Json(serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "error": {"code": -32000, "message": "execution reverted"},
                                    }));
                                }
                                CallReply::Silent => std::future::pending().await,
                            }
                        }
                        other => panic!("unexpected method {other}"),
                    };
                    axum::Json(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}))
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        StubFullNode {
            url,
            eth_calls,
            headers,
        }
    }

    fn config_for(url: &str) -> Config {
        Config {
            server_port: 0,
            service_name: "combiner-test".to_string(),
            signers: vec![],
            signer_timeout: Duration::from_secs(1),
            key_versions: vec![],
            current_key_version: 1,
            blockchain_provider: url.to_string(),
            blockchain_api_key: None,
            chain_id: SEPOLIA,
            full_node_timeout: Duration::from_secs(5),
            full_node_retry_count: 0,
            full_node_retry_delay: Duration::from_millis(1),
            log_format: LogFormat::Text,
        }
    }

    #[tokio::test]
    async fn dek_is_read_from_the_accounts_contract() {
        let dek: Vec<u8> = (0..33).collect();
        let node = spawn_full_node(Some(SEPOLIA), CallReply::Dek(dek.clone())).await;
        let lookup = FullNodeDekLookup::new(&config_for(&node.url)).unwrap();

        assert_eq!(lookup.dek(Address::ZERO).await, Ok(hex::encode(dek)));
    }

    #[tokio::test]
    async fn account_without_a_dek_reads_empty() {
        let node = spawn_full_node(Some(SEPOLIA), CallReply::Dek(vec![])).await;
        let lookup = FullNodeDekLookup::new(&config_for(&node.url)).unwrap();

        assert_eq!(lookup.dek(Address::ZERO).await, Ok(String::new()));
    }

    #[tokio::test]
    async fn lookup_makes_one_try_plus_the_retry_count() {
        let node = spawn_full_node(Some(SEPOLIA), CallReply::RpcError).await;
        let config = Config {
            full_node_retry_count: 2,
            ..config_for(&node.url)
        };
        let lookup = FullNodeDekLookup::new(&config).unwrap();

        let result = lookup.dek(Address::ZERO).await;

        assert!(
            matches!(result, Err(DekLookupError::FullNode(_))),
            "{result:?}"
        );
        assert_eq!(node.eth_calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn lookup_gives_up_at_the_full_node_timeout() {
        let node = spawn_full_node(Some(SEPOLIA), CallReply::Silent).await;
        let config = Config {
            full_node_timeout: Duration::from_millis(200),
            full_node_retry_count: 5,
            ..config_for(&node.url)
        };
        let lookup = FullNodeDekLookup::new(&config).unwrap();

        let started = Instant::now();
        let result = lookup.dek(Address::ZERO).await;

        assert_eq!(result, Err(DekLookupError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn chain_id_must_match() {
        let node = spawn_full_node(Some(MAINNET), CallReply::Silent).await;
        let lookup = FullNodeDekLookup::new(&config_for(&node.url)).unwrap();
        assert!(matches!(
            lookup.verify_chain_id().await,
            Err(FullNodeError::ChainIdMismatch {
                configured: SEPOLIA,
                full_node: MAINNET
            })
        ));

        let matching = spawn_full_node(Some(SEPOLIA), CallReply::Silent).await;
        let lookup = FullNodeDekLookup::new(&config_for(&matching.url)).unwrap();
        assert!(lookup.verify_chain_id().await.is_ok());

        let silent = spawn_full_node(None, CallReply::Silent).await;
        let config = Config {
            full_node_timeout: Duration::from_millis(200),
            ..config_for(&silent.url)
        };
        let lookup = FullNodeDekLookup::new(&config).unwrap();
        assert!(matches!(
            lookup.verify_chain_id().await,
            Err(FullNodeError::NoChainId)
        ));
    }

    #[tokio::test]
    async fn only_known_chains_are_served() {
        let config = Config {
            chain_id: 1,
            ..config_for("http://127.0.0.1:1")
        };
        assert!(matches!(
            FullNodeDekLookup::new(&config),
            Err(FullNodeError::UnknownChain(1))
        ));
    }

    /// Collects what `tracing` writes while a test runs.
    #[derive(Clone, Default)]
    struct LogCapture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCapture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test]
    async fn provider_url_stays_out_of_errors_and_logs() {
        // A closed port makes the failure a transport error, which names the URL.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let url = format!("http://127.0.0.1:{port}/v3/SECRET-KEY-123?key=SECRET-QUERY-456");
        let lookup = FullNodeDekLookup::new(&config_for(&url)).unwrap();

        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let dek_error = lookup.dek(Address::ZERO).await.unwrap_err();
        let chain_error = lookup.verify_chain_id().await.unwrap_err();

        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("failed to fetch DEK"), "{logs}");
        assert!(
            logs.contains("failed to fetch the full node's chain ID"),
            "{logs}"
        );
        for text in [dek_error.to_string(), chain_error.to_string(), logs] {
            assert!(!text.contains("SECRET-KEY-123"), "{text}");
            assert!(!text.contains("SECRET-QUERY-456"), "{text}");
        }
        assert!(matches!(dek_error, DekLookupError::FullNode(_)));
    }

    #[test]
    fn redaction_skips_an_empty_query() {
        let url = "http://127.0.0.1:1/?";
        let lookup = FullNodeDekLookup::new(&config_for(url)).unwrap();

        assert_eq!(lookup.redact(&"connection refused"), "connection refused");
        assert_eq!(
            lookup.redact(&format!("error sending request for url ({url})")),
            "error sending request for url (<BLOCKCHAIN_PROVIDER>)"
        );
    }

    #[tokio::test]
    async fn bad_url_or_api_key_is_refused() {
        let config = config_for("not a url");
        assert!(matches!(
            FullNodeDekLookup::new(&config),
            Err(FullNodeError::InvalidUrl)
        ));
        let config = Config {
            blockchain_api_key: Some("bad\nkey".to_string()),
            ..config_for("http://127.0.0.1:1")
        };
        assert!(matches!(
            FullNodeDekLookup::new(&config),
            Err(FullNodeError::InvalidApiKey)
        ));
    }

    #[tokio::test]
    async fn api_key_is_sent_in_the_apikey_header() {
        let node = spawn_full_node(Some(SEPOLIA), CallReply::Dek(vec![1])).await;

        let with_key = Config {
            blockchain_api_key: Some("forno-key".to_string()),
            ..config_for(&node.url)
        };
        FullNodeDekLookup::new(&with_key)
            .unwrap()
            .dek(Address::ZERO)
            .await
            .unwrap();
        FullNodeDekLookup::new(&config_for(&node.url))
            .unwrap()
            .dek(Address::ZERO)
            .await
            .unwrap();

        let seen = node.headers.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].get("apiKey").unwrap(), "forno-key");
        assert!(seen[1].get("apiKey").is_none());
    }

    /// Counts calls and answers from a script: `true` for success.
    struct ScriptedLookup {
        calls: AtomicU32,
        fail_first: bool,
    }

    #[async_trait]
    impl DekLookup for ScriptedLookup {
        async fn dek(&self, _account: Address) -> Result<String, DekLookupError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_first && call == 0 {
                Err(DekLookupError::Timeout)
            } else {
                Ok("abcd".to_string())
            }
        }
    }

    #[tokio::test]
    async fn cache_serves_repeats_and_forgets_failures() {
        let inner = Arc::new(ScriptedLookup {
            calls: AtomicU32::new(0),
            fail_first: false,
        });
        let cache = CachingDekLookup::new(inner.clone());
        assert_eq!(cache.dek(Address::ZERO).await, Ok("abcd".to_string()));
        assert_eq!(cache.dek(Address::ZERO).await, Ok("abcd".to_string()));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);

        let inner = Arc::new(ScriptedLookup {
            calls: AtomicU32::new(0),
            fail_first: true,
        });
        let cache = CachingDekLookup::new(inner.clone());
        assert_eq!(cache.dek(Address::ZERO).await, Err(DekLookupError::Timeout));
        assert_eq!(cache.dek(Address::ZERO).await, Ok("abcd".to_string()));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }
}
