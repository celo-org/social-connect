use alloy::primitives::{Address, B256, keccak256};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
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
/// Matches the TS `authenticateUser` in
/// `odis-signer-4.0.6:packages/common/src/utils/authentication.ts`.
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
            // The TS signer falls back here too: the chain returns `0x` for an
            // account without a DEK, which its check for a missing key lets pass.
            _ => {
                warn!(account = %account, "Account does not have registered encryption key, falling back to wallet key");
            }
        }
    }

    verify_wallet_key_signature(body, authorization, account)
}

/// Verify a wallet key (EIP-191 personal_sign) signature.
///
/// The `authorization` header contains a hex-encoded 65-byte ECDSA signature over the
/// raw request body bytes. Like `parseSignature` in @celo/utils, which the TS signer
/// uses, this accepts two layouts: RSV (r || s || v, as alloy and Ethereum JSON-RPC
/// write it) and VRS (v || r || s, as @celo/utils `serializeSignature` writes it).
/// The bytes alone do not say which one it is, so each reading is tried and the one
/// that recovers `account` wins.
fn verify_wallet_key_signature(body: &[u8], authorization: &str, account: Address) -> bool {
    let Some(hash) = wallet_key_message_hash(body) else {
        return false;
    };

    let sig_bytes = match hex::decode(authorization.strip_prefix("0x").unwrap_or(authorization)) {
        Ok(b) => b,
        Err(_) => return false,
    };

    if sig_bytes.len() != 65 {
        return false;
    }

    let as_rsv = (&sig_bytes[..64], sig_bytes[64]);
    let as_vrs = (&sig_bytes[1..], sig_bytes[0]);
    [as_rsv, as_vrs]
        .into_iter()
        .any(|(r_and_s, v)| recovers_account(hash, r_and_s, v, account))
}

/// The hash a wallet key signs for `body`, as `hashMessageWithPrefix` in @celo/utils
/// computes it: the EIP-191 prefix, the length of the body, then the body in UTF-8.
///
/// The length is counted in UTF-16 code units, where EIP-191 counts bytes, because
/// @celo/utils writes the length of a JavaScript string. The two differ for a body
/// with non-ASCII characters, and the TS combiner and signers accept only a signature
/// over this hash. Returns `None` for a body that is not UTF-8, which no JavaScript
/// string encodes to.
fn wallet_key_message_hash(body: &[u8]) -> Option<B256> {
    let length = std::str::from_utf8(body).ok()?.encode_utf16().count();
    let mut message = format!("\x19Ethereum Signed Message:\n{length}").into_bytes();
    message.extend_from_slice(body);
    Some(keccak256(message))
}

/// Whether the signature `r_and_s` with recovery value `v` over `hash` was made by
/// `account`. `v` may be written as 27/28 or as 0/1.
fn recovers_account(hash: B256, r_and_s: &[u8], v: u8, account: Address) -> bool {
    let y_parity = match v {
        0 | 27 => false,
        1 | 28 => true,
        _ => return false,
    };
    alloy::primitives::Signature::from_bytes_and_parity(r_and_s, y_parity)
        .recover_address_from_prehash(&hash)
        .is_ok_and(|recovered| recovered == account)
}

/// Verify a DEK (Data Encryption Key) signature.
///
/// The DEK signing scheme uses:
/// - SHA-256 digest of `JSON.stringify(body_string)` (the "double JSON-stringify" quirk from TS)
/// - secp256k1 ECDSA with the signature encoded as a JSON array of DER bytes
///
/// Matches TS `verifyDEKSignature` + `getMessageDigest` in authentication.ts.
pub fn verify_dek_signature(body: &[u8], authorization: &str, dek: &str) -> bool {
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

    // The Authorization header contains a JSON array of DER byte values, e.g. [48, 68, 2, ...]
    let der_bytes: Vec<u8> = match serde_json::from_str::<Vec<u8>>(authorization) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let signature = match K256Signature::from_der(&der_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // k256 only verifies signatures with a low S. elliptic.js, which clients sign with,
    // does not normalise S and accepts both forms, so bring a high S down first.
    let signature = signature.normalize_s().unwrap_or(signature);

    // elliptic.js's `key.verify(hexDigest, sig)` treats the hex string as a BigNumber
    // (via `new BN(msg, 16)`) and uses it directly as the hash value for ECDSA — no
    // additional hashing. We must use `verify_prehash` with the raw digest bytes to match.
    verifying_key.verify_prehash(&digest, &signature).is_ok()
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

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let sig: K256Signature = signing_key.sign_prehash(&digest).unwrap();
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

    /// Rewrites an RSV signature (r || s || v) as VRS (v || r || s), the layout
    /// `serializeSignature` in @celo/utils produces.
    fn to_vrs(rsv: &[u8]) -> Vec<u8> {
        let mut vrs = vec![rsv[64]];
        vrs.extend_from_slice(&rsv[..64]);
        vrs
    }

    /// The TS signer accepts a wallet signature in either layout, with the
    /// recovery value written as 27/28 or as 0/1.
    #[tokio::test]
    async fn wallet_key_accepts_both_layouts_and_both_recovery_encodings() {
        let signer = PrivateKeySigner::random();
        let account = signer.address();
        let body = r#"{"account":"0x1","blindedQueryPhoneNumber":"test"}"#;
        let rsv = signer
            .sign_message(body.as_bytes())
            .await
            .unwrap()
            .as_bytes();
        assert!(rsv[64] == 27 || rsv[64] == 28);

        let mut rsv_low_v = rsv;
        rsv_low_v[64] -= 27;
        let cases = [
            ("RSV, v as 27/28", rsv.to_vec()),
            ("RSV, v as 0/1", rsv_low_v.to_vec()),
            ("VRS, v as 27/28", to_vrs(&rsv)),
            ("VRS, v as 0/1", to_vrs(&rsv_low_v)),
        ];

        let stranger = PrivateKeySigner::random().address();
        for (name, signature) in cases {
            let authorization = format!("0x{}", hex::encode(&signature));
            assert!(
                authenticate_user(body.as_bytes(), Some(&authorization), account, None, None),
                "{name} must authenticate the signer"
            );
            assert!(
                !authenticate_user(body.as_bytes(), Some(&authorization), stranger, None, None),
                "{name} must not authenticate another account"
            );
        }
    }

    /// About 1 in 128 VRS signatures ends in a byte that looks like a recovery
    /// value. The layout must be decided by which reading recovers the account,
    /// not by where a 27 or 28 happens to sit.
    #[tokio::test]
    async fn wallet_key_accepts_a_vrs_signature_that_ends_like_an_rsv_one() {
        let signer: PrivateKeySigner =
            "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
                .parse()
                .unwrap();
        let account = signer.address();

        let mut found = None;
        for nonce in 0..5000 {
            let body = format!(r#"{{"account":"0x1","sessionID":"{nonce}"}}"#);
            let rsv = signer
                .sign_message(body.as_bytes())
                .await
                .unwrap()
                .as_bytes();
            // The last byte of s becomes the last byte of the VRS form.
            if rsv[63] == 27 || rsv[63] == 28 {
                found = Some((body, to_vrs(&rsv)));
                break;
            }
        }
        let (body, vrs) = found.expect("no ambiguous signature in 5000 tries");

        let authorization = format!("0x{}", hex::encode(&vrs));
        assert!(authenticate_user(
            body.as_bytes(),
            Some(&authorization),
            account,
            None,
            None
        ));
    }

    /// One request body signed with the wallet key in two ways, read from
    /// `tests/data/wallet_key_vectors.json`.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WalletKeyVector {
        description: String,
        body: String,
        utf16_length: usize,
        byte_length: usize,
        /// Made by @celo/utils, which the TS services verify with.
        celo_utils_signature: String,
        /// Made by viem, as EIP-191 says.
        standard_signature: String,
        /// What @celo/utils `verifySignature` answers for the standard signature.
        ts_accepts_standard_signature: bool,
    }

    #[derive(serde::Deserialize)]
    struct WalletKeyVectors {
        account: Address,
        vectors: Vec<WalletKeyVector>,
    }

    fn wallet_key_vectors() -> WalletKeyVectors {
        serde_json::from_str(include_str!("../tests/data/wallet_key_vectors.json")).unwrap()
    }

    /// The TS services write the length in the EIP-191 prefix in UTF-16 code
    /// units, EIP-191 in bytes. Over an ASCII body the two signatures are both
    /// valid. Over any other body only the TS one is, here as in the TS services.
    #[test]
    fn wallet_key_answers_ts_and_standard_signatures_as_the_ts_services_do() {
        let WalletKeyVectors { account, vectors } = wallet_key_vectors();

        // (bytes - UTF-16 units) of each body: 0 for ASCII, 1 for a two-byte
        // character, 2 for a three-byte one, 2 for a surrogate pair, 5 for all.
        let length_differences: Vec<usize> = vectors
            .iter()
            .map(|vector| {
                assert_eq!(vector.body.encode_utf16().count(), vector.utf16_length);
                assert_eq!(vector.body.len(), vector.byte_length);
                vector.byte_length - vector.utf16_length
            })
            .collect();
        assert_eq!(length_differences, [0, 1, 2, 2, 5]);

        for vector in &vectors {
            assert_eq!(
                vector.ts_accepts_standard_signature,
                vector.utf16_length == vector.byte_length
            );
            for (kind, signature, ts_accepts) in [
                ("@celo/utils", &vector.celo_utils_signature, true),
                (
                    "standard",
                    &vector.standard_signature,
                    vector.ts_accepts_standard_signature,
                ),
            ] {
                assert_eq!(
                    authenticate_user(vector.body.as_bytes(), Some(signature), account, None, None),
                    ts_accepts,
                    "{kind} signature over a body with {}",
                    vector.description
                );
            }
        }
    }

    /// A body is JSON, which is UTF-8. Other bytes have no JavaScript string to
    /// take a length from, so no signature over them is accepted.
    #[tokio::test]
    async fn wallet_key_refuses_a_body_that_is_not_utf8() {
        let signer = PrivateKeySigner::random();
        let body = b"\xff\xfe";
        let signature = signer.sign_message(body).await.unwrap();

        assert!(!authenticate_user(
            body,
            Some(&hex::encode(signature.as_bytes())),
            signer.address(),
            None,
            None,
        ));
    }

    #[test]
    fn wallet_key_refuses_a_vector_signature_for_another_body_or_account() {
        let WalletKeyVectors { account, vectors } = wallet_key_vectors();
        let stranger = address!("0x0000000000000000000000000000000000007E57");

        for (index, vector) in vectors.iter().enumerate() {
            for signature in [&vector.celo_utils_signature, &vector.standard_signature] {
                for (other_index, other) in vectors.iter().enumerate() {
                    if other_index == index {
                        continue;
                    }
                    assert!(
                        !authenticate_user(
                            other.body.as_bytes(),
                            Some(signature),
                            account,
                            None,
                            None
                        ),
                        "a signature over a body with {} must not verify over one with {}",
                        vector.description,
                        other.description
                    );
                }
                assert!(
                    !authenticate_user(
                        vector.body.as_bytes(),
                        Some(signature),
                        stranger,
                        None,
                        None
                    ),
                    "a signature over a body with {} must not authenticate another account",
                    vector.description
                );
            }
        }
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
        assert!(!authenticate_user(b"body", None, Address::ZERO, None, None,));
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

    /// Signatures made by the TS SDK (`signWithRawKey`, elliptic 6.6.1) with the DEK test
    /// key. elliptic does not normalise S, so about half of what clients send has a high S.
    #[test]
    fn dek_signatures_from_the_ts_sdk_verify_with_low_and_high_s() {
        let dek = Some(public_key_hex(&dek_signing_key()));
        let vectors = [
            (
                "low S",
                r#"{"account":"0xc1912fee45d61c87cc5ea59dae31190fffff232d","authenticationMethod":"encryption_key","sessionID":"vector-2"}"#,
                "[48,68,2,32,55,234,200,222,162,18,178,238,148,191,10,96,152,167,230,72,40,116,160,249,110,24,164,233,253,16,92,217,41,71,151,195,2,32,101,174,109,236,83,83,238,63,135,247,251,172,121,226,207,31,100,150,65,90,139,254,103,148,110,176,100,240,237,5,95,59]",
            ),
            (
                "high S",
                r#"{"account":"0xc1912fee45d61c87cc5ea59dae31190fffff232d","authenticationMethod":"encryption_key","sessionID":"vector-0"}"#,
                "[48,69,2,32,108,69,202,239,20,151,18,175,86,171,254,245,244,243,204,106,211,172,69,130,204,54,192,130,44,35,60,94,147,55,216,189,2,33,0,222,245,206,243,210,56,174,71,182,149,143,239,127,189,254,146,14,174,91,139,241,16,245,204,205,214,154,86,18,10,44,187]",
            ),
        ];

        for (name, body, authorization) in vectors {
            assert!(
                authenticate_user(
                    body.as_bytes(),
                    Some(authorization),
                    DEK_ACCOUNT,
                    Some(AuthenticationMethod::EncryptionKey),
                    dek.as_deref(),
                ),
                "{name} signature must verify"
            );
            // The same signature over a different body must not verify.
            assert!(
                !authenticate_user(
                    DEK_BODY.as_bytes(),
                    Some(authorization),
                    DEK_ACCOUNT,
                    Some(AuthenticationMethod::EncryptionKey),
                    dek.as_deref(),
                ),
                "{name} signature must not verify for another body"
            );
        }
    }

    #[tokio::test]
    async fn encryption_key_request_without_a_dek_falls_back_to_wallet_key() {
        let signer = PrivateKeySigner::random();
        let account = signer.address();
        let body = serde_json::json!({
            "account": account,
            "authenticationMethod": "encryption_key"
        })
        .to_string();
        let sig = signer.sign_message(body.as_bytes()).await.unwrap();
        let sig_hex = hex::encode(sig.as_bytes());

        for dek in [None, Some("")] {
            assert!(authenticate_user(
                body.as_bytes(),
                Some(&sig_hex),
                account,
                Some(AuthenticationMethod::EncryptionKey),
                dek,
            ));

            // A signature by another wallet is still refused.
            let other = PrivateKeySigner::random();
            let other_sig = other.sign_message(body.as_bytes()).await.unwrap();
            assert!(!authenticate_user(
                body.as_bytes(),
                Some(&hex::encode(other_sig.as_bytes())),
                account,
                Some(AuthenticationMethod::EncryptionKey),
                dek,
            ));
        }
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
