use std::future::Future;
use std::time::Duration;

use alloy::network::Ethereum;
use alloy::primitives::{Address, U256, address};
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::sol;
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
        let provider = ProviderBuilder::default().connect_http(url);

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
        let (dek, total_quota) =
            tokio::try_join!(self.get_dek(address), self.get_total_quota(address))?;

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
    use alloy::primitives::U256;
    use std::sync::atomic::{AtomicU32, Ordering};

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
