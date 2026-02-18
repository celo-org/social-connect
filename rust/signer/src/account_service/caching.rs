use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::Address;
use async_trait::async_trait;
use moka::future::Cache;

use super::{AccountService, PnpAccount};
use crate::errors::OdisError;

const MAX_CAPACITY: u64 = 500;
const TTL: Duration = Duration::from_secs(5);

/// Wraps an `AccountService` with an async LRU cache (moka).
pub struct CachingAccountService {
    inner: Arc<dyn AccountService>,
    cache: Cache<Address, PnpAccount>,
}

impl CachingAccountService {
    pub fn new(inner: Arc<dyn AccountService>) -> Self {
        let cache = Cache::builder()
            .max_capacity(MAX_CAPACITY)
            .time_to_live(TTL)
            .build();
        Self { inner, cache }
    }
}

#[async_trait]
impl AccountService for CachingAccountService {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
        let inner = self.inner.clone();
        self.cache
            .try_get_with(address, async move { inner.get_account(address).await })
            .await
            .map_err(|e| (*e).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::account_service::MockAccountService;

    #[tokio::test]
    async fn cache_hit_avoids_inner_call() {
        let call_count = Arc::new(AtomicU32::new(0));
        let counter = call_count.clone();

        // Wrap MockAccountService with a call counter
        let mock = Arc::new(CountingAccountService {
            inner: MockAccountService::new(None, 10),
            call_count: counter,
        });

        let caching = CachingAccountService::new(mock);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        // First call hits inner
        let account = caching.get_account(addr).await.unwrap();
        assert_eq!(account.pnp_total_quota, 10);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Second call should be cached
        let account = caching.get_account(addr).await.unwrap();
        assert_eq!(account.pnp_total_quota, 10);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn error_is_not_cached() {
        let inner = Arc::new(FailingAccountService);
        let caching = CachingAccountService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        // Should propagate the error
        let result = caching.get_account(addr).await;
        assert!(result.is_err());

        // Cache should be empty after error
        assert!(caching.cache.get(&addr).await.is_none());
    }

    /// AccountService wrapper that counts calls.
    struct CountingAccountService {
        inner: MockAccountService,
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl AccountService for CountingAccountService {
        async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.inner.get_account(address).await
        }
    }

    /// AccountService that always fails.
    struct FailingAccountService;

    #[async_trait]
    impl AccountService for FailingAccountService {
        async fn get_account(&self, _address: Address) -> Result<PnpAccount, OdisError> {
            Err(OdisError::FullNodeError)
        }
    }
}
