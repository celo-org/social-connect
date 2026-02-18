use alloy::primitives::Address;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use k256::ecdsa::signature::Verifier;
use k256::ecdsa::{Signature as K256Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::types::AuthenticationMethod;

/// Axum extractor for the `Authorization` header value.
#[derive(Debug)]
pub struct Authorization(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for Authorization {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        Ok(Authorization(value))
    }
}

/// Authenticate a user request.
///
/// Tries DEK verification first (when `authentication_method` is `EncryptionKey` and a DEK
/// public key is available), then falls back to wallet key (EIP-191 personal_sign) verification.
///
/// Matches the TS `authenticateUser` in `packages/common/src/utils/authentication.ts`.
pub fn authenticate_user(
    body: &[u8],
    authorization: Option<&str>,
    account: Address,
    authentication_method: Option<AuthenticationMethod>,
    dek: Option<&str>,
) -> bool {
    let authorization = match authorization {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };

    if authentication_method == Some(AuthenticationMethod::EncryptionKey) {
        match dek {
            Some(dek) if !dek.is_empty() => {
                if verify_dek_signature(body, authorization, dek) {
                    return true;
                }
                info!(account = %account, "DEK verification failed, falling back to wallet key");
            }
            _ => {
                warn!(account = %account, "Account does not have registered encryption key");
                return false;
            }
        }
    }

    verify_wallet_key_signature(body, authorization, account)
}

/// Verify a wallet key (EIP-191 personal_sign) signature.
///
/// The `authorization` header contains a hex-encoded 65-byte ECDSA signature (r, s, v).
/// The signed message is the raw request body bytes.
fn verify_wallet_key_signature(body: &[u8], authorization: &str, account: Address) -> bool {
    let sig_bytes = match hex::decode(authorization.strip_prefix("0x").unwrap_or(authorization)) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let sig = match alloy::primitives::Signature::try_from(sig_bytes.as_slice()) {
        Ok(s) => s,
        Err(_) => return false,
    };

    match sig.recover_address_from_msg(body) {
        Ok(recovered) => recovered == account,
        Err(_) => false,
    }
}

/// Verify a DEK (Data Encryption Key) signature.
///
/// The DEK signing scheme uses:
/// - SHA-256 digest of `JSON.stringify(body_string)` (the "double JSON-stringify" quirk from TS)
/// - secp256k1 ECDSA with the signature encoded as a JSON array of DER bytes
///
/// Matches TS `verifyDEKSignature` + `getMessageDigest` in authentication.ts.
fn verify_dek_signature(body: &[u8], authorization: &str, dek: &str) -> bool {
    let dek_hex = dek.strip_prefix("0x").unwrap_or(dek);
    let dek_bytes = match hex::decode(dek_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let verifying_key = match VerifyingKey::from_sec1_bytes(&dek_bytes) {
        Ok(k) => k,
        Err(_) => return false,
    };

    // The TS code does: getMessageDigest(message) where message = JSON.stringify(request.body)
    // and getMessageDigest does: sha256(JSON.stringify(message)).hexdigest()
    // So the digest is: sha256(JSON.stringify(JSON.stringify(request.body)))
    //
    // `body` is already the raw JSON bytes (= JSON.stringify(request.body) from the client).
    // We need to JSON-encode that string again (adding quotes and escaping), then SHA-256 hash it.
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let double_stringified = serde_json::to_string(body_str).unwrap_or_default();
    let digest = Sha256::digest(double_stringified.as_bytes());
    let digest_hex = hex::encode(digest);

    // The Authorization header contains a JSON array of DER byte values, e.g. [48, 68, 2, ...]
    let der_bytes: Vec<u8> = match serde_json::from_str::<Vec<u8>>(authorization) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let signature = match K256Signature::from_der(&der_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // The TS `key.verify(hexDigest, parsedSig)` passes the hex-encoded digest as a string.
    // elliptic.js treats string inputs as character bytes, so the ECDSA verification runs
    // over the ASCII bytes of the hex string (not the raw digest bytes).
    // We replicate that by passing `digest_hex.as_bytes()` to `Verifier::verify`, which
    // internally hashes those bytes before checking the signature.
    verifying_key
        .verify(digest_hex.as_bytes(), &signature)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;
    use alloy_signer::Signer;
    use alloy_signer_local::PrivateKeySigner;

    /// Produce a DEK signature over `body` using the given signing key, matching the TS
    /// `signWithRawKey` function. Returns the JSON-encoded DER byte array for the
    /// Authorization header.
    fn sign_dek(body: &str, signing_key: &k256::ecdsa::SigningKey) -> String {
        let double_stringified = serde_json::to_string(body).unwrap();
        let digest = Sha256::digest(double_stringified.as_bytes());
        let digest_hex = hex::encode(digest);

        use k256::ecdsa::signature::Signer as _;
        let sig: K256Signature = signing_key.sign(digest_hex.as_bytes());
        serde_json::to_string(&sig.to_der().as_bytes()).unwrap()
    }

    const DEK_ACCOUNT: Address = address!("0xc1912fee45d61c87cc5ea59dae31190fffff232d");
    const DEK_BODY: &str = r#"{"account":"0xc1912fee45d61c87cc5ea59dae31190fffff232d","authenticationMethod":"encryption_key"}"#;
    // Same private key used in the TS test suite (authentication.test.ts).
    const DEK_RAW_KEY_HEX: &str =
        "41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04";

    fn dek_signing_key() -> k256::ecdsa::SigningKey {
        k256::ecdsa::SigningKey::from_slice(&hex::decode(DEK_RAW_KEY_HEX).unwrap()).unwrap()
    }

    fn public_key_hex(key: &k256::ecdsa::SigningKey) -> String {
        hex::encode(key.verifying_key().to_sec1_bytes())
    }

    #[tokio::test]
    async fn wallet_key_valid_signature() {
        let signer = PrivateKeySigner::random();
        let account = signer.address();
        let body = serde_json::json!({
            "account": account,
            "blindedQueryPhoneNumber": "test"
        })
        .to_string();

        let sig = signer.sign_message(body.as_bytes()).await.unwrap();
        let sig_hex = hex::encode(sig.as_bytes());

        assert!(authenticate_user(
            body.as_bytes(),
            Some(&sig_hex),
            account,
            None,
            None,
        ));
    }

    #[tokio::test]
    async fn wallet_key_wrong_signer() {
        let signer = PrivateKeySigner::random();
        let wrong_account = address!("0x0000000000000000000000000000000000007E57");

        let sig = signer.sign_message(b"test body").await.unwrap();
        let sig_hex = hex::encode(sig.as_bytes());

        assert!(!authenticate_user(
            b"test body",
            Some(&sig_hex),
            wrong_account,
            None,
            None,
        ));
    }

    #[test]
    fn missing_authorization_header() {
        assert!(!authenticate_user(
            b"body",
            None,
            Address::ZERO,
            None,
            None,
        ));
    }

    #[test]
    fn empty_authorization_header() {
        assert!(!authenticate_user(
            b"body",
            Some(""),
            Address::ZERO,
            None,
            None,
        ));
    }

    #[test]
    fn invalid_hex_signature() {
        assert!(!authenticate_user(
            b"body",
            Some("not-hex"),
            Address::ZERO,
            None,
            None,
        ));
    }

    #[test]
    fn dek_no_key_registered() {
        assert!(!authenticate_user(
            b"body",
            Some("sig"),
            Address::ZERO,
            Some(AuthenticationMethod::EncryptionKey),
            None,
        ));
    }

    #[test]
    fn dek_empty_key() {
        assert!(!authenticate_user(
            b"body",
            Some("sig"),
            Address::ZERO,
            Some(AuthenticationMethod::EncryptionKey),
            Some(""),
        ));
    }

    #[test]
    fn dek_valid_signature() {
        let signing_key = dek_signing_key();
        let authorization = sign_dek(DEK_BODY, &signing_key);

        assert!(authenticate_user(
            DEK_BODY.as_bytes(),
            Some(&authorization),
            DEK_ACCOUNT,
            Some(AuthenticationMethod::EncryptionKey),
            Some(&public_key_hex(&signing_key)),
        ));
    }

    #[tokio::test]
    async fn dek_invalid_falls_back_to_wallet_key() {
        let signer = PrivateKeySigner::random();
        let account = signer.address();
        let body = serde_json::json!({
            "account": account,
            "authenticationMethod": "encryption_key"
        })
        .to_string();

        let sig = signer.sign_message(body.as_bytes()).await.unwrap();
        let sig_hex = hex::encode(sig.as_bytes());

        // Provide a valid-looking DEK that doesn't match the signature
        let random_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);

        // The wallet key hex signature won't parse as a JSON DER array for DEK,
        // so DEK fails and falls through to wallet key which succeeds.
        assert!(authenticate_user(
            body.as_bytes(),
            Some(&sig_hex),
            account,
            Some(AuthenticationMethod::EncryptionKey),
            Some(&public_key_hex(&random_key)),
        ));
    }

    #[test]
    fn dek_wrong_key_fails() {
        let signing_key = dek_signing_key();
        let authorization = sign_dek(DEK_BODY, &signing_key);

        let wrong_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);

        // DEK fails (wrong key), wallet key also fails (not an EIP-191 sig).
        assert!(!authenticate_user(
            DEK_BODY.as_bytes(),
            Some(&authorization),
            DEK_ACCOUNT,
            Some(AuthenticationMethod::EncryptionKey),
            Some(&public_key_hex(&wrong_key)),
        ));
    }

    #[test]
    fn dek_invalid_public_key() {
        // "notAValidKeyEncryption" is not valid hex for a sec1 public key
        assert!(!authenticate_user(
            DEK_BODY.as_bytes(),
            Some("sig"),
            DEK_ACCOUNT,
            Some(AuthenticationMethod::EncryptionKey),
            Some("notAValidKeyEncryption"),
        ));
    }

    #[test]
    fn dek_message_manipulated_after_signing() {
        let signing_key = dek_signing_key();
        let authorization = sign_dek(DEK_BODY, &signing_key);

        // Modify every fourth character of the body and verify signature fails
        let body_bytes = DEK_BODY.as_bytes();
        for i in (0..body_bytes.len()).step_by(4) {
            let mut modified = body_bytes.to_vec();
            modified[i] = modified[i].wrapping_add(1);

            assert!(
                !authenticate_user(
                    &modified,
                    Some(&authorization),
                    DEK_ACCOUNT,
                    Some(AuthenticationMethod::EncryptionKey),
                    Some(&public_key_hex(&signing_key)),
                ),
                "Should fail for body modified at index {i}"
            );
        }
    }

    #[test]
    fn dek_modified_signature() {
        let signing_key = dek_signing_key();
        let authorization = sign_dek(DEK_BODY, &signing_key);

        // Prepend a 0 byte to the DER array to corrupt it
        let mut der_bytes: Vec<u8> = serde_json::from_str(&authorization).unwrap();
        der_bytes.insert(0, 0);
        let modified = serde_json::to_string(&der_bytes).unwrap();

        assert!(!authenticate_user(
            DEK_BODY.as_bytes(),
            Some(&modified),
            DEK_ACCOUNT,
            Some(AuthenticationMethod::EncryptionKey),
            Some(&public_key_hex(&signing_key)),
        ));
    }

    #[test]
    fn dek_incorrectly_generated_signature() {
        // Sign the raw body without the double-stringify + sha256 digest.
        // This is what happens if someone uses key.sign(body) directly instead of
        // key.sign(getMessageDigest(body)).
        let signing_key = dek_signing_key();

        use k256::ecdsa::signature::Signer as _;
        let sig: K256Signature = signing_key.sign(DEK_BODY.as_bytes());
        let authorization = serde_json::to_string(&sig.to_der().as_bytes()).unwrap();

        assert!(!authenticate_user(
            DEK_BODY.as_bytes(),
            Some(&authorization),
            DEK_ACCOUNT,
            Some(AuthenticationMethod::EncryptionKey),
            Some(&public_key_hex(&signing_key)),
        ));
    }
}
