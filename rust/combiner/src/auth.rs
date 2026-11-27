//! Authentication of a PNP request, in the TS combiner's order
//! (`odis-combiner-4.0.5:packages/common/src/utils/authentication.ts:50-98`).

use odis_signer::auth::{authenticate_user, verify_dek_signature};
use odis_signer::types::AuthenticationMethod;

use crate::body::CanonicalBody;
use crate::dek::{DekLookup, DekLookupError};
use crate::errors::FAILURE_TO_GET_DEK;
use crate::request::{PnpRequest, strict_address};

#[derive(Debug, PartialEq)]
pub enum AuthFailure {
    /// No `Authorization` header, or an empty one.
    MissingHeader,
    /// The account is not an address in viem's strict sense.
    NotStrictAddress,
    DekLookup(DekLookupError),
    /// A header without `0x` that is not a valid DEK signature.
    UnprefixedSignature,
    BadSignature,
}

/// Checks the `Authorization` header against the canonical body. The DEK is looked up only for
/// an `encryption_key` request whose account passed the address check.
pub async fn authenticate(
    dek: &dyn DekLookup,
    body: &CanonicalBody,
    request: &PnpRequest,
    authorization: Option<&str>,
) -> Result<(), AuthFailure> {
    let header = authorization
        .filter(|header| !header.is_empty())
        .ok_or(AuthFailure::MissingHeader)?;
    // TODO(all-rust-signers): the signer's `Address` parse also takes an account without `0x`,
    // with `0X`, in upper case or with a wrong checksum, which TS refuses before any lookup.
    let address = strict_address(&request.account).ok_or(AuthFailure::NotStrictAddress)?;

    let method = request.authentication_method;
    let dek = if method == Some(AuthenticationMethod::EncryptionKey) {
        match dek.dek(address).await {
            Ok(dek) => Some(dek),
            Err(error) => {
                tracing::error!(%error, "{FAILURE_TO_GET_DEK}");
                return Err(AuthFailure::DekLookup(error));
            }
        }
    } else {
        None
    };

    let body = body.as_str().as_bytes();
    // TODO(all-rust-signers): the signer also takes a wallet-key signature without `0x`, which
    // TS refuses. A DEK signature is a JSON array and never starts with `0x`.
    if !header.starts_with("0x") {
        let dek_signed = dek
            .as_deref()
            .is_some_and(|dek| !dek.is_empty() && verify_dek_signature(body, header, dek));
        return if dek_signed {
            Ok(())
        } else {
            Err(AuthFailure::UnprefixedSignature)
        };
    }
    if authenticate_user(body, Some(header), address, method, dek.as_deref()) {
        Ok(())
    } else {
        Err(AuthFailure::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use alloy::primitives::Address;
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use async_trait::async_trait;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{HeaderMap, HeaderValue};
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};
    use sha2::{Digest, Sha256};

    use crate::request::parse_quota_request;

    const WALLET_KEY: &str = "535029bfb19fe5440dbd549b88fbf5ee847b059485e4eafc2a3e3bdfbf9b31ac";
    const OTHER_WALLET_KEY: &str =
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890fdeccc";
    const DEK_KEY: &str = "bf8a2b73baf8402f8fe906ad3f42b560bf14b39f7df7797ece9e293d6f162188";
    const DEK_PUBLIC_KEY: &str =
        "034846bc781cacdafc66f3a77aa9fc3c56a9dadcd683c72be3c446fee8da041070";

    /// Answers `Ok(dek)` or `Err(error)` and counts the lookups.
    struct Lookup {
        answer: Result<String, DekLookupError>,
        calls: AtomicUsize,
    }

    impl Lookup {
        fn new(answer: Result<&str, DekLookupError>) -> Lookup {
            Lookup {
                answer: answer.map(str::to_owned),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DekLookup for Lookup {
        async fn dek(&self, _account: Address) -> Result<String, DekLookupError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.clone()
        }
    }

    fn account() -> String {
        PrivateKeySigner::from_str(WALLET_KEY)
            .unwrap()
            .address()
            .to_string()
            .to_lowercase()
    }

    fn body(account: &str, method: Option<&str>) -> CanonicalBody {
        let json = match method {
            None => format!(r#"{{"account":"{account}"}}"#),
            Some(method) => {
                format!(r#"{{"account":"{account}","authenticationMethod":"{method}"}}"#)
            }
        };
        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json"))]);
        CanonicalBody::parse(&headers, json.as_bytes()).unwrap()
    }

    /// `0x` and r, s, v in hex.
    fn wallet_signature(key: &str, body: &CanonicalBody) -> String {
        let signer = PrivateKeySigner::from_str(key).unwrap();
        let signature = signer.sign_message_sync(body.as_str().as_bytes()).unwrap();
        format!("0x{}", hex::encode(signature.as_bytes()))
    }

    /// As `signWithRawKey` writes it.
    fn dek_signature(body: &CanonicalBody) -> String {
        let key = SigningKey::from_slice(&hex::decode(DEK_KEY).unwrap()).unwrap();
        let digest = Sha256::digest(serde_json::to_string(body.as_str()).unwrap().as_bytes());
        let signature: Signature = key.sign_prehash(&digest).unwrap();
        serde_json::to_string(signature.to_der().as_bytes()).unwrap()
    }

    async fn check(
        lookup: &Lookup,
        body: &CanonicalBody,
        authorization: Option<&str>,
    ) -> Result<(), AuthFailure> {
        let request = parse_quota_request(body).unwrap();
        authenticate(lookup, body, &request, authorization).await
    }

    #[tokio::test]
    async fn a_missing_header_is_refused_before_the_address_check() {
        let upper = format!("0x{}", account()[2..].to_uppercase());
        for account in [account(), upper] {
            let body = body(&account, Some("encryption_key"));
            let lookup = Lookup::new(Ok(DEK_PUBLIC_KEY));
            for header in [None, Some("")] {
                assert_eq!(
                    check(&lookup, &body, header).await,
                    Err(AuthFailure::MissingHeader),
                    "{account} {header:?}"
                );
            }
            assert_eq!(lookup.calls(), 0);
        }
    }

    #[tokio::test]
    async fn an_account_viem_refuses_is_refused_before_the_dek_lookup() {
        let hex = &account()[2..];
        let spellings = [
            format!("0x{}", hex.to_uppercase()),
            format!("0X{hex}"),
            hex.to_string(),
        ];
        for account in spellings {
            for method in [None, Some("wallet_key"), Some("encryption_key")] {
                let body = body(&account, method);
                let lookup = Lookup::new(Ok(DEK_PUBLIC_KEY));
                let signature = wallet_signature(WALLET_KEY, &body);
                assert_eq!(
                    check(&lookup, &body, Some(&signature)).await,
                    Err(AuthFailure::NotStrictAddress),
                    "{account} {method:?}"
                );
                assert_eq!(lookup.calls(), 0);
            }
        }
    }

    #[tokio::test]
    async fn the_dek_is_looked_up_only_for_encryption_key() {
        for (method, lookups) in [
            (None, 0),
            (Some("wallet_key"), 0),
            (Some("other"), 0),
            (Some("encryption_key"), 1),
        ] {
            let body = body(&account(), method);
            let lookup = Lookup::new(Ok(DEK_PUBLIC_KEY));
            let signature = wallet_signature(WALLET_KEY, &body);
            assert_eq!(
                check(&lookup, &body, Some(&signature)).await,
                Ok(()),
                "{method:?}"
            );
            assert_eq!(lookup.calls(), lookups, "{method:?}");
        }
    }

    #[tokio::test]
    async fn a_failed_dek_lookup_is_refused() {
        let body = body(&account(), Some("encryption_key"));
        let lookup = Lookup::new(Err(DekLookupError::Timeout));

        assert_eq!(
            check(&lookup, &body, Some(&dek_signature(&body))).await,
            Err(AuthFailure::DekLookup(DekLookupError::Timeout))
        );
    }

    #[tokio::test]
    async fn a_dek_signature_passes_only_for_encryption_key_with_the_dek() {
        let dek_body = body(&account(), Some("encryption_key"));
        assert_eq!(
            check(
                &Lookup::new(Ok(DEK_PUBLIC_KEY)),
                &dek_body,
                Some(&dek_signature(&dek_body))
            )
            .await,
            Ok(())
        );

        // No DEK on chain, a DEK that did not sign, or a method other than encryption_key.
        let other_dek = "02".to_string() + &"11".repeat(32);
        let wallet_body = body(&account(), Some("wallet_key"));
        let cases = [
            (&dek_body, Lookup::new(Ok(""))),
            (&dek_body, Lookup::new(Ok(&other_dek))),
            (&wallet_body, Lookup::new(Ok(DEK_PUBLIC_KEY))),
        ];
        for (body, lookup) in cases {
            assert_eq!(
                check(&lookup, body, Some(&dek_signature(body))).await,
                Err(AuthFailure::UnprefixedSignature),
                "{}",
                body.as_str()
            );
        }
    }

    #[tokio::test]
    async fn a_wallet_signature_needs_a_lower_case_0x() {
        for method in [None, Some("wallet_key"), Some("encryption_key")] {
            let body = body(&account(), method);
            let signature = wallet_signature(WALLET_KEY, &body);
            for header in [signature[2..].to_string(), format!("0X{}", &signature[2..])] {
                assert_eq!(
                    check(&Lookup::new(Ok(DEK_PUBLIC_KEY)), &body, Some(&header)).await,
                    Err(AuthFailure::UnprefixedSignature),
                    "{method:?} {header}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_wallet_signature_is_the_fallback_for_encryption_key() {
        let body = body(&account(), Some("encryption_key"));
        let signature = wallet_signature(WALLET_KEY, &body);
        for dek in [DEK_PUBLIC_KEY, ""] {
            assert_eq!(
                check(&Lookup::new(Ok(dek)), &body, Some(&signature)).await,
                Ok(()),
                "{dek:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_signature_by_another_key_is_refused() {
        for method in [None, Some("encryption_key")] {
            let body = body(&account(), method);
            let signature = wallet_signature(OTHER_WALLET_KEY, &body);
            assert_eq!(
                check(&Lookup::new(Ok(DEK_PUBLIC_KEY)), &body, Some(&signature)).await,
                Err(AuthFailure::BadSignature),
                "{method:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_signature_covers_the_canonical_body() {
        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json"))]);
        let sent = format!("{{ \"account\" : \"{}\" }}", account());
        let body = CanonicalBody::parse(&headers, sent.as_bytes()).unwrap();
        let lookup = Lookup::new(Ok(DEK_PUBLIC_KEY));

        let over_compact = wallet_signature(WALLET_KEY, &body);
        assert_eq!(check(&lookup, &body, Some(&over_compact)).await, Ok(()));

        let signer = PrivateKeySigner::from_str(WALLET_KEY).unwrap();
        let over_sent = signer.sign_message_sync(sent.as_bytes()).unwrap();
        let over_sent = format!("0x{}", hex::encode(over_sent.as_bytes()));
        assert_eq!(
            check(&lookup, &body, Some(&over_sent)).await,
            Err(AuthFailure::BadSignature)
        );
    }
}
