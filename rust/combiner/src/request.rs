//! The request fields the combiner checks before it calls any signer, and the key version it
//! resolves. The rules are the TS combiner's, not the signer's.

use std::str::FromStr;

use alloy::primitives::Address;
use axum::http::HeaderMap;
use odis_signer::types::{AuthenticationMethod, KEY_VERSION_HEADER, is_valid_blinded_phone_number};

use crate::body::{CanonicalBody, JsValue};

/// The fields of a `/sign` or `/quotaStatus` request that the combiner reads.
#[derive(Debug, Clone, PartialEq)]
pub struct PnpRequest {
    /// As sent.
    pub account: String,
    /// `Some` for `/sign`.
    pub blinded_query_phone_number: Option<String>,
    pub authentication_method: Option<AuthenticationMethod>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum InvalidRequest {
    #[error("the field {0} is missing")]
    Missing(&'static str),
    #[error("the field {0} is not a string")]
    NotAString(&'static str),
    #[error("the account is not an address")]
    Account,
    #[error("the blinded query is not 64 base64 characters")]
    BlindedQuery,
}

pub fn parse_sign_request(body: &CanonicalBody) -> Result<PnpRequest, InvalidRequest> {
    let account = account(body)?;
    let query = match body.field("blindedQueryPhoneNumber") {
        None => return Err(InvalidRequest::Missing("blindedQueryPhoneNumber")),
        Some(JsValue::String(query)) if is_valid_blinded_phone_number(query) => query.clone(),
        Some(_) => return Err(InvalidRequest::BlindedQuery),
    };
    let authentication_method = optional_fields(body)?;
    Ok(PnpRequest {
        account,
        blinded_query_phone_number: Some(query),
        authentication_method,
    })
}

/// Reads a `/quotaStatus` request. A `blindedQueryPhoneNumber` in the body is ignored.
pub fn parse_quota_request(body: &CanonicalBody) -> Result<PnpRequest, InvalidRequest> {
    let account = account(body)?;
    let authentication_method = optional_fields(body)?;
    Ok(PnpRequest {
        account,
        blinded_query_phone_number: None,
        authentication_method,
    })
}

fn account(body: &CanonicalBody) -> Result<String, InvalidRequest> {
    match body.field("account") {
        None => Err(InvalidRequest::Missing("account")),
        // TODO(all-rust-signers): the signer's `Address` also reads a JSON array of 20 numbers;
        // TS refuses it, so the combiner accepts strings only.
        Some(JsValue::String(account)) if is_valid_address(account) => Ok(account.clone()),
        Some(_) => Err(InvalidRequest::Account),
    }
}

/// Checks that `sessionID` and `version` are strings when present, and returns the
/// authentication method. An explicit `null` is refused, as the TS schema does.
fn optional_fields(body: &CanonicalBody) -> Result<Option<AuthenticationMethod>, InvalidRequest> {
    let string = |name: &'static str| match body.field(name) {
        None => Ok(None),
        Some(JsValue::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(InvalidRequest::NotAString(name)),
    };
    string("sessionID")?;
    string("version")?;
    Ok(string("authenticationMethod")?.map(|method| match method {
        "encryption_key" => AuthenticationMethod::EncryptionKey,
        "wallet_key" => AuthenticationMethod::WalletKey,
        _ => AuthenticationMethod::Other,
    }))
}

/// `@celo/utils` `isValidAddress`: an optional `0x` or `0X` and 40 hex digits in any case. Its
/// checksum test never fails once the shape matches, so the 400 check accepts what the strict
/// check in `strict_address` later refuses.
pub fn is_valid_address(account: &str) -> bool {
    let digits = account
        .strip_prefix("0x")
        .or_else(|| account.strip_prefix("0X"))
        .unwrap_or(account);
    digits.len() == 40 && digits.bytes().all(|b| b.is_ascii_hexdigit())
}

/// viem `isAddress` in strict mode, the 401 check. `Some` only for `0x` and 40 hex digits, all
/// lower case or with a correct EIP-55 checksum.
// TODO(all-rust-signers): the signer's `Address` parse is looser; TS refuses these spellings
// with 401 before a signer sees them.
pub fn strict_address(account: &str) -> Option<Address> {
    let digits = account.strip_prefix("0x")?;
    if digits.len() != 40 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let address = Address::from_str(account).ok()?;
    (account == account.to_lowercase() || address.to_checksum(None) == account).then_some(address)
}

#[derive(Debug, PartialEq)]
pub struct UnsupportedKeyVersion;

/// The version in the `odis-key-version` header if it is plain decimal digits after trimming,
/// else `current`. Digits beyond `u32` name a version no one configures, so they are refused.
pub fn resolve_key_version(
    headers: &HeaderMap,
    current: u32,
) -> Result<u32, UnsupportedKeyVersion> {
    key_version_header(headers).unwrap_or(Ok(current))
}

/// The `odis-key-version` header of a request or a signer's answer: `None` when it is absent or
/// not plain decimal digits after trimming, an error for digits beyond `u32`.
///
/// TS reads the header with `Number()`, which turns other spellings (`1.0`, `0x1`, `+1`) into a
/// version too; here they count as absent. A header sent twice joins to `1, 2` in Node, which
/// `Number()` reads as NaN, so it also counts as absent.
pub fn key_version_header(headers: &HeaderMap) -> Option<Result<u32, UnsupportedKeyVersion>> {
    let mut values = Vec::new();
    for value in headers.get_all(KEY_VERSION_HEADER) {
        values.push(value.to_str().ok()?);
    }
    let joined = values.join(", ");
    let trimmed = joined.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.parse().map_err(|_| UnsupportedKeyVersion))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const LOWER: &str = "0xd3cbed9a5e1e3c9bd5eb3e9a8b0a4a0ab3d7c0de";

    fn body(json: &str) -> CanonicalBody {
        let content_type = HeaderValue::from_static("application/json");
        CanonicalBody::parse(Some(&content_type), json.as_bytes()).unwrap()
    }

    fn checksummed() -> String {
        Address::from_str(LOWER).unwrap().to_checksum(None)
    }

    fn quota(fields: &str) -> Result<PnpRequest, InvalidRequest> {
        parse_quota_request(&body(&format!("{{{fields}}}")))
    }

    #[test]
    fn account_check_is_celo_utils_is_valid_address() {
        let hex40 = &LOWER[2..];
        let accepted = [
            LOWER.to_string(),
            checksummed(),
            format!("0x{}", hex40.to_uppercase()),
            format!("0X{hex40}"),
            hex40.to_string(),
        ];
        for account in accepted {
            let request = quota(&format!(r#""account":"{account}""#)).unwrap();
            assert_eq!(request.account, account);
            assert!(is_valid_address(&account), "{account}");
        }

        let refused = [
            format!(r#""{}""#, &LOWER[..LOWER.len() - 1]),
            format!(r#""{LOWER}0""#),
            format!(r#""0x{}g""#, &hex40[..39]),
            "1".to_string(),
            format!("[{}]", vec!["1"; 20].join(",")),
            "null".to_string(),
        ];
        for account in refused {
            assert_eq!(
                quota(&format!(r#""account":{account}"#)),
                Err(InvalidRequest::Account),
                "{account}"
            );
        }
        assert_eq!(quota(""), Err(InvalidRequest::Missing("account")));
    }

    #[test]
    fn strict_address_is_viem_is_address() {
        let hex40 = &LOWER[2..];
        let checksummed = checksummed();
        assert_ne!(checksummed, LOWER, "the vector needs upper case letters");

        let expected = Address::from_str(LOWER).unwrap();
        assert_eq!(strict_address(LOWER), Some(expected));
        assert_eq!(strict_address(&checksummed), Some(expected));

        let wrong_checksum: String = checksummed
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect();
        for account in [
            format!("0x{}", hex40.to_uppercase()),
            wrong_checksum,
            hex40.to_string(),
            format!("0X{hex40}"),
            format!("{LOWER}0"),
            String::new(),
        ] {
            assert_eq!(strict_address(&account), None, "{account}");
        }
    }

    #[test]
    fn optional_fields_are_absent_or_strings() {
        let account = format!(r#""account":"{LOWER}""#);
        for field in ["authenticationMethod", "sessionID", "version"] {
            for value in ["null", "1", "{}"] {
                assert_eq!(
                    quota(&format!(r#"{account},"{field}":{value}"#)),
                    Err(InvalidRequest::NotAString(field)),
                    "{field}: {value}"
                );
            }
        }

        let request = quota(&format!(
            r#"{account},"sessionID":"s","version":"v","other":[1]"#
        ));
        assert_eq!(
            request,
            Ok(PnpRequest {
                account: LOWER.to_string(),
                blinded_query_phone_number: None,
                authentication_method: None,
            })
        );

        for (sent, method) in [
            ("encryption_key", AuthenticationMethod::EncryptionKey),
            ("wallet_key", AuthenticationMethod::WalletKey),
            ("WALLET_KEY", AuthenticationMethod::Other),
        ] {
            let request = quota(&format!(r#"{account},"authenticationMethod":"{sent}""#)).unwrap();
            assert_eq!(request.authentication_method, Some(method), "{sent}");
        }
    }

    #[test]
    fn blinded_query_is_64_base64_characters() {
        let account = format!(r#""account":"{LOWER}""#);
        let sign = |query: &str| {
            parse_sign_request(&body(&format!(
                r#"{{{account},"blindedQueryPhoneNumber":"{query}"}}"#
            )))
        };

        let valid = "A".repeat(64);
        assert_eq!(
            sign(&valid),
            Ok(PnpRequest {
                account: LOWER.to_string(),
                blinded_query_phone_number: Some(valid),
                authentication_method: None,
            })
        );
        for query in ["A".repeat(63), "A".repeat(65), "!".repeat(64)] {
            assert_eq!(sign(&query), Err(InvalidRequest::BlindedQuery), "{query}");
        }
        assert_eq!(
            parse_sign_request(&body(&format!("{{{account}}}"))),
            Err(InvalidRequest::Missing("blindedQueryPhoneNumber"))
        );
        assert_eq!(
            parse_sign_request(&body(&format!(
                r#"{{{account},"blindedQueryPhoneNumber":1}}"#
            ))),
            Err(InvalidRequest::BlindedQuery)
        );

        let request = quota(&format!(r#"{account},"blindedQueryPhoneNumber":1"#)).unwrap();
        assert_eq!(request.blinded_query_phone_number, None);
    }

    fn headers(values: &[&[u8]]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(KEY_VERSION_HEADER, HeaderValue::from_bytes(value).unwrap());
        }
        headers
    }

    #[test]
    fn key_version_resolution() {
        const CURRENT: u32 = 9;
        let long = "9".repeat(25);

        type Case<'a> = (&'a [&'a [u8]], Result<u32, UnsupportedKeyVersion>);
        let cases: [Case; 17] = [
            (&[], Ok(CURRENT)),
            (&[b"2"], Ok(2)),
            (&[b" 3 "], Ok(3)),
            (&[b"007"], Ok(7)),
            (&[b""], Ok(CURRENT)),
            (&[b"undefined"], Ok(CURRENT)),
            (&[b"abc"], Ok(CURRENT)),
            (&[b"1.0"], Ok(CURRENT)),
            (&[b"0x1"], Ok(CURRENT)),
            (&[b"+1"], Ok(CURRENT)),
            (&[b"1e0"], Ok(CURRENT)),
            (&[b"-1"], Ok(CURRENT)),
            (&[b"4294967296"], Err(UnsupportedKeyVersion)),
            (&[long.as_bytes()], Err(UnsupportedKeyVersion)),
            // Node joins repeated headers with ", ", which Number() reads as NaN.
            (&[b"1", b"2"], Ok(CURRENT)),
            (&[b"\xc3\xa9"], Ok(CURRENT)),
            (&[b"4294967295"], Ok(u32::MAX)),
        ];
        for (values, expected) in cases {
            assert_eq!(
                resolve_key_version(&headers(values), CURRENT),
                expected,
                "{:?}",
                values
                    .iter()
                    .map(|v| String::from_utf8_lossy(v))
                    .collect::<Vec<_>>()
            );
        }
    }
}
