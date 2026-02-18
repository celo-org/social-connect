use std::sync::Arc;
use std::time::Instant;

use ::metrics::{counter, histogram};
use alloy::primitives::Address;
use async_trait::async_trait;

use super::PnpRequestService;
use crate::errors::OdisError;
use crate::metrics;

/// Wraps a `PnpRequestService` to record latency and error metrics.
pub struct MeteredPnpRequestService {
    inner: Arc<dyn PnpRequestService>,
}

impl MeteredPnpRequestService {
    pub fn new(inner: Arc<dyn PnpRequestService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl PnpRequestService for MeteredPnpRequestService {
    async fn get_used_quota(&self, address: Address) -> Result<u32, OdisError> {
        let start = Instant::now();
        let result = self.inner.get_used_quota(address).await;
        let elapsed = start.elapsed().as_secs_f64();

        histogram!(metrics::DB_OPS_LATENCY, "operation" => "getPerformedQueryCount")
            .record(elapsed);

        if result.is_err() {
            counter!(metrics::DATABASE_ERRORS, "type" => "read").increment(1);
        }

        result
    }

    async fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError> {
        let start = Instant::now();
        let result = self
            .inner
            .get_duplicate_request(address, blinded_query)
            .await;
        let elapsed = start.elapsed().as_secs_f64();

        histogram!(metrics::DB_OPS_LATENCY, "operation" => "getRequestIfExists").record(elapsed);

        if result.is_err() {
            counter!(metrics::DATABASE_ERRORS, "type" => "read").increment(1);
        }

        result
    }

    async fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
    ) -> Result<(), OdisError> {
        let start = Instant::now();
        let result = self
            .inner
            .record_request(address, blinded_query, signature)
            .await;
        let elapsed = start.elapsed().as_secs_f64();

        histogram!(metrics::DB_OPS_LATENCY, "operation" => "insertRequest").record(elapsed);

        if result.is_err() {
            counter!(metrics::DATABASE_ERRORS, "type" => "insert").increment(1);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_service::InMemoryPnpRequestService;
    use alloy::primitives::address;

    #[tokio::test]
    async fn records_latency_for_get_used_quota() {
        let inner = Arc::new(InMemoryPnpRequestService::new());
        let metered = MeteredPnpRequestService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        let result = metered.get_used_quota(addr).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn records_latency_for_get_duplicate_request() {
        let inner = Arc::new(InMemoryPnpRequestService::new());
        let metered = MeteredPnpRequestService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        let result = metered.get_duplicate_request(addr, "query").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn records_latency_for_record_request() {
        let inner = Arc::new(InMemoryPnpRequestService::new());
        let metered = MeteredPnpRequestService::new(inner);
        let addr = address!("0x0000000000000000000000000000000000007E57");

        let result = metered.record_request(addr, "query", "sig").await;
        assert!(result.is_ok());
    }
}
