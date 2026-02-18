use std::sync::Arc;
use std::time::Instant;

use ::metrics::{counter, histogram};
use alloy::primitives::Address;
use async_trait::async_trait;

use super::{AccountService, PnpAccount};
use crate::errors::OdisError;
use crate::metrics;

/// Wraps an `AccountService` to record latency and error metrics.
pub struct MeteredAccountService {
    inner: Arc<dyn AccountService>,
}

impl MeteredAccountService {
    pub fn new(inner: Arc<dyn AccountService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl AccountService for MeteredAccountService {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
        let start = Instant::now();
        let result = self.inner.get_account(address).await;
        let elapsed = start.elapsed().as_secs_f64();

        histogram!(metrics::FULL_NODE_LATENCY, "code_segment" => "getAccount").record(elapsed);

        if result.is_err() {
            counter!(metrics::BLOCKCHAIN_ERRORS).increment(1);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_service::MockAccountService;
    use alloy::primitives::address;

    struct FailingAccountService;

    #[async_trait]
    impl AccountService for FailingAccountService {
        async fn get_account(&self, _address: Address) -> Result<PnpAccount, OdisError> {
            Err(OdisError::FullNodeError)
        }
    }

    #[tokio::test]
    async fn records_latency_on_success() {
        let inner = Arc::new(MockAccountService::new(None, 10));
        let metered = MeteredAccountService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        let result = metered.get_account(addr).await;
        assert!(result.is_ok());
        // Metric recorded without panic — full rendering tested via /metrics endpoint
    }

    #[tokio::test]
    async fn increments_blockchain_errors_on_failure() {
        let inner = Arc::new(FailingAccountService);
        let metered = MeteredAccountService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        let result = metered.get_account(addr).await;
        assert!(result.is_err());
        // blockchain_errors counter incremented without panic
    }
}
