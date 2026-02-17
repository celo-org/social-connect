use alloy::primitives::Address;
use serde::{Deserialize, Serialize};

use crate::errors::OdisError;

/// Header name for key version, matching TS `KEY_VERSION_HEADER`.
pub const KEY_VERSION_HEADER: &str = "odis-key-version";

// Body size is enforced by tower's RequestBodyLimitLayer (16 KB) in server.rs,
// matching the TS REASONABLE_BODY_CHAR_LIMIT of 16,000 chars.

// -- Requests --

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageRequest {
    pub account: Address,
    pub blinded_query_phone_number: String,
    #[serde(default)]
    pub authentication_method: Option<String>,
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
    pub authentication_method: Option<String>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageResponseFailure {
    pub success: bool,
    pub version: String,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_query_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_quota: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnpQuotaResponseSuccess {
    pub success: bool,
    pub version: String,
    pub performed_query_count: u32,
    pub total_quota: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnpQuotaResponseFailure {
    pub success: bool,
    pub version: String,
    pub error: String,
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

/// Parses the key version header value. Returns `None` if absent or empty.
/// Returns `Some(Ok(version))` for valid integers >= 0, `Some(Err(()))` for invalid values.
/// Matches TS `getRequestKeyVersion` / `parseKeyVersionFromHeader`.
pub fn parse_key_version_header(value: Option<&str>) -> Option<Result<u32, ()>> {
    let value = value?.trim();
    if value.is_empty() || value == "undefined" {
        return None;
    }
    Some(value.parse::<u32>().map_err(|_| ()))
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

    #[test]
    fn key_version_header_parsing() {
        // Absent, empty, "undefined" → None
        assert!(parse_key_version_header(None).is_none());
        assert!(parse_key_version_header(Some("")).is_none());
        assert!(parse_key_version_header(Some("  ")).is_none());
        assert!(parse_key_version_header(Some("undefined")).is_none());

        // Valid integers
        assert_eq!(parse_key_version_header(Some("1")), Some(Ok(1)));
        assert_eq!(parse_key_version_header(Some(" 3 ")), Some(Ok(3)));
        assert_eq!(parse_key_version_header(Some("0")), Some(Ok(0)));

        // Invalid
        assert_eq!(parse_key_version_header(Some("abc")), Some(Err(())));
        assert_eq!(parse_key_version_header(Some("-1")), Some(Err(())));
        assert_eq!(parse_key_version_header(Some("1.5")), Some(Err(())));
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
    fn quota_request_deserializes_from_camel_case() {
        let json = r#"{"account": "0x0000000000000000000000000000000000007E57"}"#;
        let req: PnpQuotaRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.account, VALID_ACCOUNT);
        assert!(req.session_id.is_none());
    }

    #[test]
    fn sign_response_serialization() {
        // Success: camelCase keys, empty warnings omitted
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
        assert!(json.get("warnings").is_none());

        // Failure: None fields omitted
        let failure = SignMessageResponseFailure {
            success: false,
            version: "1.0.0".to_string(),
            error: "some error".to_string(),
            performed_query_count: None,
            total_quota: None,
        };
        let json = serde_json::to_value(&failure).unwrap();
        assert!(json.get("performedQueryCount").is_none());
        assert!(json.get("totalQuota").is_none());

        // Failure with quota: fields present
        let failure_with_quota = SignMessageResponseFailure {
            success: false,
            version: "1.0.0".to_string(),
            error: "some error".to_string(),
            performed_query_count: Some(5),
            total_quota: Some(10),
        };
        let json = serde_json::to_value(&failure_with_quota).unwrap();
        assert_eq!(json["performedQueryCount"], 5);
        assert_eq!(json["totalQuota"], 10);
    }
}
