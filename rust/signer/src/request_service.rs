use std::collections::HashMap;
use std::sync::Mutex;

use alloy::primitives::Address;

use crate::errors::OdisError;

/// Tracks PNP sign requests and quota usage.
pub trait PnpRequestService: Send + Sync {
    fn get_used_quota(&self, address: Address) -> Result<u32, OdisError>;
    fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError>;
    fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
    ) -> Result<(), OdisError>;
}

/// In-memory implementation for testing. Will be replaced by a sqlx-backed one later.
pub struct InMemoryPnpRequestService {
    /// address → query count
    quotas: Mutex<HashMap<Address, u32>>,
    /// (address, blinded_query) → signature
    requests: Mutex<HashMap<(Address, String), String>>,
}

impl InMemoryPnpRequestService {
    pub fn new() -> Self {
        Self {
            quotas: Mutex::new(HashMap::new()),
            requests: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryPnpRequestService {
    fn default() -> Self {
        Self::new()
    }
}

impl PnpRequestService for InMemoryPnpRequestService {
    fn get_used_quota(&self, address: Address) -> Result<u32, OdisError> {
        let quotas = self.quotas.lock().map_err(|_| OdisError::DatabaseError)?;
        Ok(*quotas.get(&address).unwrap_or(&0))
    }

    fn get_duplicate_request(
        &self,
        address: Address,
        blinded_query: &str,
    ) -> Result<Option<String>, OdisError> {
        let requests = self.requests.lock().map_err(|_| OdisError::DatabaseError)?;
        let key = (address, blinded_query.to_string());
        Ok(requests.get(&key).cloned())
    }

    fn record_request(
        &self,
        address: Address,
        blinded_query: &str,
        signature: &str,
    ) -> Result<(), OdisError> {
        let mut requests = self.requests.lock().map_err(|_| OdisError::DatabaseError)?;
        let mut quotas = self.quotas.lock().map_err(|_| OdisError::DatabaseError)?;

        requests.insert((address, blinded_query.to_string()), signature.to_string());
        *quotas.entry(address).or_insert(0) += 1;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const ADDR: Address = address!("0x0000000000000000000000000000000000007E57");
    const OTHER_ADDR: Address = address!("0x0000000000000000000000000000000000000001");

    #[test]
    fn in_memory_request_service() {
        let svc = InMemoryPnpRequestService::new();

        // Initially zero quota
        assert_eq!(svc.get_used_quota(ADDR).unwrap(), 0);

        // No duplicate
        assert!(svc.get_duplicate_request(ADDR, "query1").unwrap().is_none());

        // Record a request
        svc.record_request(ADDR, "query1", "sig1").unwrap();
        assert_eq!(svc.get_used_quota(ADDR).unwrap(), 1);
        assert_eq!(
            svc.get_duplicate_request(ADDR, "query1").unwrap(),
            Some("sig1".to_string())
        );

        // Different query increments quota again
        svc.record_request(ADDR, "query2", "sig2").unwrap();
        assert_eq!(svc.get_used_quota(ADDR).unwrap(), 2);

        // Different address is independent
        assert_eq!(svc.get_used_quota(OTHER_ADDR).unwrap(), 0);
    }
}
