use alloy::primitives::Address;
use async_trait::async_trait;

use super::{AccountService, PnpAccount};
use crate::errors::OdisError;

/// Mock account service that returns configured values for any address.
pub struct MockAccountService {
    dek: Option<String>,
    total_quota: u32,
}

impl MockAccountService {
    pub fn new(dek: Option<String>, total_quota: u32) -> Self {
        Self { dek, total_quota }
    }
}

#[async_trait]
impl AccountService for MockAccountService {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
        Ok(PnpAccount {
            address,
            dek: self.dek.clone().unwrap_or_default(),
            pnp_total_quota: self.total_quota,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[tokio::test]
    async fn returns_configured_values() {
        let service = MockAccountService::new(Some("04abcd".to_string()), 42);
        let account = service
            .get_account(address!("0x0000000000000000000000000000000000007E57"))
            .await
            .unwrap();

        assert_eq!(
            account.address,
            address!("0x0000000000000000000000000000000000007E57")
        );
        assert_eq!(account.dek, "04abcd");
        assert_eq!(account.pnp_total_quota, 42);
    }

    #[tokio::test]
    async fn returns_empty_dek_when_none() {
        let service = MockAccountService::new(None, 10);
        let account = service.get_account(Address::ZERO).await.unwrap();

        assert_eq!(account.dek, "");
        assert_eq!(account.pnp_total_quota, 10);
    }
}
