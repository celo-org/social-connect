use std::future::Future;
use std::time::Duration;

use alloy::network::Ethereum;
use alloy::primitives::{Address, U256, address};
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::sol;
use alloy::transports::http::reqwest;
use async_trait::async_trait;
use tracing::warn;

use super::{AccountService, PnpAccount};
use crate::config::Config;
use crate::errors::OdisError;

sol! {
    #[sol(rpc)]
    interface IAccounts {
        function getDataEncryptionKey(address account) external view returns (bytes);
    }

    #[sol(rpc)]
    interface IOdisPayments {
        function totalPaidCUSD(address account) external view returns (uint256);
    }
}

/// Default contract addresses per chain ID.
struct ContractAddresses {
    /// Celo Accounts contract, used to look up data encryption keys (DEKs).
    accounts: Address,
    /// OdisPayments contract, used to look up total cUSD paid for quota.
    odis_payments: Address,
}

/// Returns default contract addresses for known chains.
/// Only mainnet and Celo Sepolia are supported.
fn default_contract_addresses(chain_id: u64) -> Option<ContractAddresses> {
    match chain_id {
        // Celo mainnet
        42220 => Some(ContractAddresses {
            accounts: address!("0x7d21685C17607338b313a7174bAb6620baD0aaB7"),
            odis_payments: address!("0xae6b29f31b96e61dddc792f45fda4e4f0356d0cb"),
        }),
        // Celo Sepolia
        11142220 => Some(ContractAddresses {
            accounts: address!("0x44957232699ca060B607E77083bDACD350d6b6d1"),
            odis_payments: address!("0x96AfaE75F12A759c1dFB364ce93548c3Bd242D58"),
        }),
        _ => None,
    }
}

/// Name of the header that carries the full node API key, as the TS signer sends it.
const API_KEY_HEADER: &str = "apiKey";

/// Builds the HTTP client for full node requests. With an API key, every
/// request carries it in the `apiKey` header.
fn build_http_client(api_key: Option<&str>) -> Result<reqwest::Client, OdisError> {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(api_key) = api_key {
        let mut value = reqwest::header::HeaderValue::from_str(api_key).map_err(|_| {
            warn!("BLOCKCHAIN_API_KEY is not a valid HTTP header value");
            OdisError::FullNodeError
        })?;
        // Keeps the key out of debug output of the header map.
        value.set_sensitive(true);
        headers.insert(API_KEY_HEADER, value);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| {
            warn!(error = %e, "failed to build full node HTTP client");
            OdisError::FullNodeError
        })
}

pub struct ClientAccountService {
    contracts: ContractAddresses,
    provider: RootProvider<Ethereum>,
    retry_count: u32,
    retry_delay: Duration,
    query_price_per_cusd: f64,
}

impl ClientAccountService {
    pub fn new(config: &Config) -> Result<Self, OdisError> {
        let provider_url = config.blockchain_provider.as_deref().ok_or_else(|| {
            warn!("BLOCKCHAIN_PROVIDER is required for on-chain account service");
            OdisError::FullNodeError
        })?;

        let defaults = default_contract_addresses(config.chain_id);
        let accounts = config
            .accounts_contract_address
            .or(defaults.as_ref().map(|d| d.accounts))
            .ok_or_else(|| {
                warn!(
                    chain_id = config.chain_id,
                    "no Accounts contract address configured"
                );
                OdisError::FullNodeError
            })?;
        let odis_payments = config
            .odis_payments_contract_address
            .or(defaults.as_ref().map(|d| d.odis_payments))
            .ok_or_else(|| {
                warn!(
                    chain_id = config.chain_id,
                    "no OdisPayments contract address configured"
                );
                OdisError::FullNodeError
            })?;

        let url = provider_url.parse().map_err(|_| {
            warn!("invalid BLOCKCHAIN_PROVIDER URL");
            OdisError::FullNodeError
        })?;
        let http_client = build_http_client(config.blockchain_api_key.as_deref())?;
        let provider = ProviderBuilder::default().connect_reqwest(http_client, url);

        Ok(Self {
            contracts: ContractAddresses {
                accounts,
                odis_payments,
            },
            provider,
            retry_count: config.full_node_retry_count,
            retry_delay: Duration::from_millis(config.full_node_retry_delay_ms),
            query_price_per_cusd: config.query_price_per_cusd,
        })
    }

    async fn get_dek(&self, address: Address) -> Result<String, OdisError> {
        let accounts = IAccounts::new(self.contracts.accounts, &self.provider);

        let dek_bytes = retry_with_backoff(self.retry_count, self.retry_delay, || async {
            let result = accounts.getDataEncryptionKey(address).call().await;
            result.map(|r| r.0).map_err(|e| {
                warn!(error = %e, %address, "failed to fetch DEK");
                OdisError::FailureToGetDek
            })
        })
        .await?;

        Ok(hex::encode(&dek_bytes))
    }

    async fn get_total_quota(&self, address: Address) -> Result<u32, OdisError> {
        let payments = IOdisPayments::new(self.contracts.odis_payments, &self.provider);

        let total_paid = retry_with_backoff(self.retry_count, self.retry_delay, || async {
            let result = payments.totalPaidCUSD(address).call().await;
            result.map_err(|e| {
                warn!(error = %e, %address, "failed to fetch total paid");
                OdisError::FailureToGetTotalQuota
            })
        })
        .await?;

        Ok(calculate_quota(total_paid, self.query_price_per_cusd))
    }
}

#[async_trait]
impl AccountService for ClientAccountService {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
        // Both lookups run at once. The TS signer reads the DEK first, so when
        // both fail, the DEK error is the one to report.
        let (dek, total_quota) = tokio::join!(self.get_dek(address), self.get_total_quota(address));
        let (dek, total_quota) = (dek?, total_quota?);

        Ok(PnpAccount {
            address,
            dek,
            pnp_total_quota: total_quota,
        })
    }
}

/// Calculate quota from total paid amount (in wei) and price per query (in cUSD).
///
/// Formula: `floor(total_paid_wei / (query_price_per_cusd * 1e18))`
fn calculate_quota(total_paid_wei: U256, query_price_per_cusd: f64) -> u32 {
    let divisor_f64 = query_price_per_cusd * 1e18;
    if divisor_f64 <= 0.0 {
        return 0;
    }
    let divisor = U256::from(divisor_f64 as u128);
    if divisor.is_zero() {
        return 0;
    }
    let quota = total_paid_wei / divisor;
    // Saturate to u32::MAX if the quota exceeds u32 range
    quota.try_into().unwrap_or(u32::MAX)
}

/// Retry an async operation with exponential backoff (1.5x multiplier).
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

    // Initial attempt
    let mut last_err = match f().await {
        Ok(val) => return Ok(val),
        Err(e) => e,
    };

    // Retries with backoff
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
    use crate::config::{DatabaseConfig, KeystoreType, LogFormat};
    use alloy::primitives::U256;
    use axum::http::HeaderMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    /// Starts a JSON-RPC server on a free local port that answers every call
    /// with 1 cUSD (1e18) and records the headers of each request it receives.
    async fn spawn_full_node() -> (String, Arc<Mutex<Vec<HeaderMap>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let app = axum::Router::new().fallback(
            move |headers: HeaderMap, axum::Json(request): axum::Json<serde_json::Value>| {
                let recorder = recorder.clone();
                async move {
                    recorder.lock().unwrap().push(headers);
                    axum::Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": format!("0x{:064x}", 1_000_000_000_000_000_000u128),
                    }))
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    fn config_for(provider_url: String, api_key: Option<&str>) -> Config {
        Config {
            server_port: 0,
            pnp_api_enabled: true,
            keystore_type: KeystoreType::Mock,
            pnp_key_name_base: "phoneNumberPrivacy".to_string(),
            pnp_latest_key_version: 1,
            database: DatabaseConfig::Sqlite {
                path: ":memory:".to_string(),
            },
            migrate_legacy_data: false,
            blockchain_provider: Some(provider_url),
            blockchain_api_key: api_key.map(String::from),
            chain_id: 11142220,
            accounts_contract_address: None,
            odis_payments_contract_address: None,
            full_node_retry_count: 0,
            full_node_retry_delay_ms: 1,
            timeout_ms: 5000,
            query_price_per_cusd: 0.001,
            google_project_id: None,
            request_pruning_days: 7,
            request_pruning_interval_secs: 86400,
            log_format: LogFormat::Text,
        }
    }

    #[tokio::test]
    async fn api_key_is_sent_to_the_full_node_only_when_configured() {
        let (url, seen) = spawn_full_node().await;

        let with_key = ClientAccountService::new(&config_for(url.clone(), Some("forno-key")))
            .expect("client with API key");
        assert_eq!(with_key.get_total_quota(Address::ZERO).await, Ok(1000));

        let without_key =
            ClientAccountService::new(&config_for(url, None)).expect("client without API key");
        assert_eq!(without_key.get_total_quota(Address::ZERO).await, Ok(1000));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].get("apiKey").unwrap(), "forno-key");
        assert!(seen[1].get("apiKey").is_none());
    }

    #[test]
    fn api_key_that_is_not_a_valid_header_value_is_rejected() {
        let config = config_for("http://localhost:8545".to_string(), Some("bad\nkey"));
        assert!(matches!(
            ClientAccountService::new(&config),
            Err(OdisError::FullNodeError)
        ));
    }

    #[test]
    fn default_contract_addresses_by_chain() {
        // Mainnet
        let mainnet = default_contract_addresses(42220).unwrap();
        assert_eq!(
            mainnet.accounts,
            address!("0x7d21685C17607338b313a7174bAb6620baD0aaB7")
        );
        assert_eq!(
            mainnet.odis_payments,
            address!("0xae6b29f31b96e61dddc792f45fda4e4f0356d0cb")
        );

        // Celo Sepolia
        let sepolia = default_contract_addresses(11142220).unwrap();
        assert_eq!(
            sepolia.accounts,
            address!("0x44957232699ca060B607E77083bDACD350d6b6d1")
        );
        assert_eq!(
            sepolia.odis_payments,
            address!("0x96AfaE75F12A759c1dFB364ce93548c3Bd242D58")
        );

        // Unknown chain
        assert!(default_contract_addresses(99999).is_none());
    }

    #[test]
    fn calculate_quota_from_payments() {
        // Zero payment
        assert_eq!(calculate_quota(U256::ZERO, 0.001), 0);

        // Exactly 1 query: 0.001 cUSD = 1e15 wei
        assert_eq!(
            calculate_quota(U256::from(1_000_000_000_000_000u128), 0.001),
            1
        );

        // 1.5 queries floors to 1
        assert_eq!(
            calculate_quota(U256::from(1_500_000_000_000_000u128), 0.001),
            1
        );

        // 1 cUSD = 1e18 wei at 0.001 cUSD/query = 1000 queries
        assert_eq!(
            calculate_quota(U256::from(1_000_000_000_000_000_000u128), 0.001),
            1000
        );

        // The TS signer's other test vectors: 1 wei and 1.56 cUSD.
        assert_eq!(calculate_quota(U256::from(1u64), 0.001), 0);
        assert_eq!(
            calculate_quota(U256::from(1_560_000_000_000_000_000u128), 0.001),
            1560
        );

        // More than fits in 32 bits stops at the maximum. That needs one
        // account to pay over 4.29 million cUSD.
        assert_eq!(
            calculate_quota(U256::from(123_456_789u128 * 10u128.pow(18)), 0.001),
            u32::MAX
        );

        // Edge cases: zero and negative price
        assert_eq!(calculate_quota(U256::from(1_000u64), 0.0), 0);
        assert_eq!(calculate_quota(U256::from(1_000u64), -1.0), 0);
    }

    #[tokio::test]
    async fn retry_succeeds_first_try() {
        let result: Result<u32, &str> =
            retry_with_backoff(3, Duration::from_millis(1), || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retry_succeeds_after_failures() {
        let attempts = AtomicU32::new(0);
        let result: Result<u32, &str> = retry_with_backoff(3, Duration::from_millis(1), || async {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < 2 { Err("not yet") } else { Ok(99) }
        })
        .await;
        assert_eq!(result.unwrap(), 99);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_exhausts_all_attempts() {
        let attempts = AtomicU32::new(0);
        let result: Result<u32, &str> = retry_with_backoff(2, Duration::from_millis(1), || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("always fails")
        })
        .await;
        assert_eq!(result.unwrap_err(), "always fails");
        // 1 initial + 2 retries = 3 attempts
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
