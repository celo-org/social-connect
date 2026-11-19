use alloy::primitives::Address;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::errors::OdisError;

/// Header name for key version, matching TS `KEY_VERSION_HEADER`.
pub const KEY_VERSION_HEADER: &str = "odis-key-version";

/// Reads the key version a request asks for from its headers. `None` when the
/// header is absent, empty or the string `undefined`, as in TS
/// `getRequestKeyVersion`: the request is then signed with the latest key.
///
/// A header that is sent more than once or is not ASCII is refused. Node joins
/// repeated headers with a comma and the TS signer then fails to read a number
/// from the result, so neither names a version there either.
pub fn requested_key_version(headers: &HeaderMap) -> Result<Option<u32>, OdisError> {
    let mut values = headers.get_all(KEY_VERSION_HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(OdisError::InvalidKeyVersion);
    }
    let value = value
        .to_str()
        .map_err(|_| OdisError::InvalidKeyVersion)?
        .trim();
    if value.is_empty() || value == "undefined" {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| OdisError::InvalidKeyVersion)
}

// -- Authentication --

/// Authentication method, matching TS `AuthenticationMethod` enum.
///
/// The TS signer accepts any string here and verifies by wallet key unless the
/// value is exactly `encryption_key`, so an unknown value must parse too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationMethod {
    WalletKey,
    EncryptionKey,
    /// Any other string. Verified by wallet key.
    #[serde(other)]
    Other,
}

// -- Requests --

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageRequest {
    pub account: Address,
    pub blinded_query_phone_number: String,
    #[serde(default)]
    pub authentication_method: Option<AuthenticationMethod>,
    #[serde(default, rename = "sessionID")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

impl SignMessageRequest {
    pub fn validate(&self) -> Result<(), OdisError> {
        if !is_valid_blinded_phone_number(&self.blinded_query_phone_number) {
            return Err(OdisError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnpQuotaRequest {
    pub account: Address,
    #[serde(default)]
    pub authentication_method: Option<AuthenticationMethod>,
    #[serde(default, rename = "sessionID")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

// -- Responses --

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageResponseSuccess {
    pub success: bool,
    pub version: String,
    pub signature: String,
    pub performed_query_count: u32,
    pub total_quota: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnpQuotaResponseSuccess {
    pub success: bool,
    pub version: String,
    pub performed_query_count: u32,
    pub total_quota: u32,
    pub warnings: Vec<String>,
}

// -- Validation --

/// Validates a blinded phone number: must be exactly 64 characters long
/// and valid base64. Matches TS `hasValidBlindedPhoneNumberParam`.
fn is_valid_blinded_phone_number(value: &str) -> bool {
    use base64::Engine;
    value.len() == 64
        && base64::engine::general_purpose::STANDARD
            .decode(value)
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    const VALID_ACCOUNT: Address = address!("0x0000000000000000000000000000000000007E57");
    const VALID_BLINDED_QUERY: &str =
        "n/I9srniwEHm5o6t3y0tTUB5fn7xjxRrLP1F/i8ORCdqV++WWiaAzUo3GA2UNHiB";

    #[test]
    fn sign_request_validate() {
        let valid = SignMessageRequest {
            account: VALID_ACCOUNT,
            blinded_query_phone_number: VALID_BLINDED_QUERY.to_string(),
            authentication_method: None,
            session_id: None,
            version: None,
        };
        assert!(valid.validate().is_ok());

        // Bad blinded query
        let bad_query = SignMessageRequest {
            blinded_query_phone_number: "too-short".to_string(),
            ..valid
        };
        assert!(bad_query.validate().is_err());
    }

    #[test]
    fn sign_request_rejects_invalid_account() {
        let json = r#"{
            "account": "not-an-address",
            "blindedQueryPhoneNumber": "abc123"
        }"#;
        assert!(serde_json::from_str::<SignMessageRequest>(json).is_err());
    }

    #[test]
    fn quota_request_rejects_invalid_account() {
        let json = r#"{"account": "bad"}"#;
        assert!(serde_json::from_str::<PnpQuotaRequest>(json).is_err());
    }

    /// Headers with one key version header per given value.
    fn key_version_headers(values: &[&[u8]]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                KEY_VERSION_HEADER,
                axum::http::HeaderValue::from_bytes(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn key_version_header_parsing() {
        // Absent, empty, "undefined" → None
        let absent: [&[&[u8]]; 4] = [&[], &[b""], &[b"  "], &[b"undefined"]];
        for values in absent {
            assert_eq!(
                requested_key_version(&key_version_headers(values)),
                Ok(None)
            );
        }

        // Valid integers
        let valid: [(&[u8], u32); 4] = [(b"1", 1), (b" 3 ", 3), (b"0", 0), (b"2", 2)];
        for (value, version) in valid {
            assert_eq!(
                requested_key_version(&key_version_headers(&[value])),
                Ok(Some(version))
            );
        }

        // Invalid: not a whole number, not ASCII, or sent twice
        let invalid: [&[&[u8]]; 6] = [
            &[b"abc"],
            &[b"-1"],
            &[b"1.5"],
            &[b"\xff1"],
            &[b"1", b"1"],
            &[b"", b""],
        ];
        for values in invalid {
            assert_eq!(
                requested_key_version(&key_version_headers(values)),
                Err(OdisError::InvalidKeyVersion)
            );
        }
    }

    #[test]
    fn sign_request_deserializes_from_camel_case() {
        let json = r#"{
            "account": "0x0000000000000000000000000000000000007E57",
            "blindedQueryPhoneNumber": "abc123",
            "sessionID": "sess-1"
        }"#;
        let req: SignMessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.account, VALID_ACCOUNT);
        assert_eq!(req.blinded_query_phone_number, "abc123");
        assert_eq!(req.session_id.as_deref(), Some("sess-1"));
        assert!(req.authentication_method.is_none());
        assert!(req.version.is_none());
    }

    #[test]
    fn authentication_method_accepts_any_string() {
        let parse = |method: &str| {
            let json = format!(
                r#"{{"account": "0x0000000000000000000000000000000000007E57", "authenticationMethod": {method}}}"#
            );
            serde_json::from_str::<PnpQuotaRequest>(&json).map(|req| req.authentication_method)
        };

        assert_eq!(
            parse(r#""wallet_key""#).unwrap(),
            Some(AuthenticationMethod::WalletKey)
        );
        assert_eq!(
            parse(r#""encryption_key""#).unwrap(),
            Some(AuthenticationMethod::EncryptionKey)
        );
        for unknown in [
            r#""WALLET_KEY""#,
            r#""ENCRYPTION_KEY""#,
            r#""foo""#,
            r#""""#,
        ] {
            assert_eq!(
                parse(unknown).unwrap(),
                Some(AuthenticationMethod::Other),
                "{unknown}"
            );
        }
        // The TS signer refuses a value that is not a string.
        assert!(parse("1").is_err());
        assert!(parse("true").is_err());
    }

    #[test]
    fn quota_request_deserializes_from_camel_case() {
        let json = r#"{"account": "0x0000000000000000000000000000000000007E57"}"#;
        let req: PnpQuotaRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.account, VALID_ACCOUNT);
        assert!(req.session_id.is_none());
    }

    #[test]
    fn sign_response_serialization() {
        // Success: camelCase keys, empty warnings always present
        let success = SignMessageResponseSuccess {
            success: true,
            version: "1.0.0".to_string(),
            signature: "sig".to_string(),
            performed_query_count: 1,
            total_quota: 10,
            warnings: vec![],
        };
        let json = serde_json::to_value(&success).unwrap();
        assert_eq!(json["performedQueryCount"], 1);
        assert_eq!(json["totalQuota"], 10);
        assert_eq!(json["warnings"], serde_json::json!([]));
    }
}
