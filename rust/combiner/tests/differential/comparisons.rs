//! The requests both combiners get, and what they must answer.
//!
//! A comparison that expects `SameAnswer` must get equal answers from both. A comparison that
//! names two answers pins a difference listed in rust/combiner/CHANGELOG.md, so neither side can
//! drift from it unnoticed.
//! The lines the TS combiner's Express logs are not compared: they come from a released image,
//! which cannot change.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::LazyLock;

use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use flate2::Compression;
use flate2::write::GzEncoder;
use k256::ecdsa::Signature;
use odis_combiner::errors::{
    EXCEEDED_QUOTA, INVALID_INPUT, INVALID_KEY_VERSION_REQUEST, NOT_ENOUGH_PARTIAL_SIGNATURES,
    UNAUTHENTICATED_USER, UNKNOWN_ERROR,
};

use crate::common::{DEK_PRIVATE_KEY, dek_auth, dek_auth_by, vector_message};
use crate::harness::{RawRequest, TestUser, USER_WITHOUT_DEK, USER_WITHOUT_QUOTA, test_user};

pub struct Comparison {
    pub name: String,
    /// Builds the request for an account used by no other comparison, so comparisons cannot share
    /// quota.
    pub request: Box<dyn Fn(&TestUser) -> RawRequest>,
    pub expected: Expected,
}

pub enum Expected {
    /// Both combiners answer alike, with this status and error string (none for a success), so
    /// a comparison cannot pass by both sides failing alike or refusing for the same wrong reason.
    SameAnswer {
        status: u16,
        error: Option<&'static str>,
    },
    /// Each combiner answers as its side says.
    KnownDifference {
        ts: PinnedAnswer,
        rust: PinnedAnswer,
    },
}

/// The parts of an answer a comparison expects; a part left `None` is not compared.
#[derive(Debug, PartialEq)]
pub struct PinnedAnswer {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
}

impl Comparison {
    fn same_answer(
        name: impl Into<String>,
        status: u16,
        error: Option<&'static str>,
        request: impl Fn(&TestUser) -> RawRequest + 'static,
    ) -> Comparison {
        Comparison {
            name: name.into(),
            request: Box::new(request),
            expected: Expected::SameAnswer { status, error },
        }
    }

    fn known_difference(
        name: impl Into<String>,
        ts: PinnedAnswer,
        rust: PinnedAnswer,
        request: impl Fn(&TestUser) -> RawRequest + 'static,
    ) -> Comparison {
        Comparison {
            name: name.into(),
            request: Box::new(request),
            expected: Expected::KnownDifference { ts, rust },
        }
    }
}

// -- Bodies --

/// Message 0 of the combine vectors, blinded as the TS integration test blinds `test message`.
static BLINDED_QUERY: LazyLock<String> = LazyLock::new(|| BASE64.encode(vector_message(0)));
/// The combined signatures of message 0 under key versions 1 and 2.
const SIGNATURE_V1: &str = "xgFMQtcgAMHJAEX/m9B4VFopYtxqPFSw0024sWzRYvQDvnmFqhXOPdnRDfa8WCEA";
const SIGNATURE_V2: &str = "wUuFV8yFBXGyEzKbyWjBChG6dER264nwjOsqErd/UZieVKE0oDMZcMDG+qObu4QB";
const SESSION_ID: &str = "differential-session";
/// `authenticationMethod` of a request signed with the DEK, as JSON.
const ENCRYPTION_KEY: &str = r#""encryption_key""#;
/// A key that is not the DEK's.
const OTHER_DEK_PRIVATE_KEY: &str =
    "0x00000000000000000000000000000000000000000000000000000000ddddbbbb";

/// `value` as a JSON string.
fn string(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

/// The members of a JSON object in order: each key with its value as JSON text.
struct Fields(Vec<(&'static str, String)>);

impl Fields {
    /// The fields with `key` set to the JSON `value`, where the key stands or else at the end, as
    /// `{ ...fields, [key]: value }` sets it.
    fn set(mut self, key: &'static str, value: &str) -> Fields {
        match self.0.iter_mut().find(|(name, _)| *name == key) {
            Some(member) => member.1 = value.to_string(),
            None => self.0.push((key, value.to_string())),
        }
        self
    }

    fn without(mut self, key: &str) -> Fields {
        self.0.retain(|(name, _)| *name != key);
        self
    }

    fn is_encryption_key(&self) -> bool {
        self.0
            .iter()
            .any(|(name, value)| *name == "authenticationMethod" && value == ENCRYPTION_KEY)
    }

    /// `JSON.stringify(fields)`.
    fn compact(&self) -> String {
        let members: Vec<String> = self
            .0
            .iter()
            .map(|(key, value)| format!("{}:{value}", string(key)))
            .collect();
        format!("{{{}}}", members.join(","))
    }

    /// `JSON.stringify(fields, null, 2)`, for fields whose values are not objects or arrays.
    fn pretty(&self) -> String {
        let members: Vec<String> = self
            .0
            .iter()
            .map(|(key, value)| format!("  {}: {value}", string(key)))
            .collect();
        format!("{{\n{}\n}}", members.join(",\n"))
    }
}

/// signFields: the fields of a sign request by `user`.
fn sign_fields(user: &TestUser) -> Fields {
    Fields(vec![
        ("account", string(&user.account)),
        ("blindedQueryPhoneNumber", string(&BLINDED_QUERY)),
        ("authenticationMethod", string("wallet_key")),
        ("sessionID", string(SESSION_ID)),
    ])
}

/// quotaFields: the fields of a quota request by `user`.
fn quota_fields(user: &TestUser) -> Fields {
    Fields(vec![
        ("account", string(&user.account)),
        ("authenticationMethod", string("wallet_key")),
        ("sessionID", string(SESSION_ID)),
    ])
}

/// The compact sign body of `user` with `members` (raw JSON) appended.
fn sign_body_with(user: &TestUser, members: &str) -> String {
    let body = sign_fields(user).compact();
    format!("{},{members}}}", &body[..body.len() - 1])
}

/// The sign fields with an extra field that makes the compact body exactly `length` UTF-16 code
/// units long.
fn padded_to(user: &TestUser, length: usize) -> Fields {
    let empty = sign_fields(user)
        .set("padding", r#""""#)
        .compact()
        .encode_utf16()
        .count();
    sign_fields(user).set("padding", &string(&"x".repeat(length - empty)))
}

/// The member `key` holding `depth` nested empty arrays, inside the object that holds it.
fn nested(key: &str, depth: usize) -> String {
    format!("{}:{}{}", string(key), "[".repeat(depth), "]".repeat(depth))
}

fn gzip(body: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

// -- Signatures --

/// A wallet-key signature of `message` as `@celo/utils` writes it: `0x` and v, r, s in hex,
/// over the EIP-191 hash with the length counted in UTF-16 code units, as JavaScript counts the
/// length of a string.
fn wallet_key_signature(user: &TestUser, message: &str) -> String {
    let length = message.encode_utf16().count();
    let mut prefixed = format!("\x19Ethereum Signed Message:\n{length}").into_bytes();
    prefixed.extend_from_slice(message.as_bytes());
    let signature = PrivateKeySigner::from_bytes(&user.private_key)
        .unwrap()
        .sign_hash_sync(&alloy::primitives::keccak256(prefixed))
        .unwrap();
    let rsv = signature.as_bytes();
    format!("0x{:02x}{}", rsv[64], hex::encode(&rsv[..64]))
}

/// A wallet-key signature of `message` as viem's `signMessage` writes it: `0x` and r, s, v in
/// hex, over the EIP-191 hash, as `wallet_auth_by` makes it.
fn viem_signature(user: &TestUser, message: &str) -> String {
    let signature = PrivateKeySigner::from_bytes(&user.private_key)
        .unwrap()
        .sign_message_sync(message.as_bytes())
        .unwrap();
    format!("0x{}", hex::encode(signature.as_bytes()))
}

/// A DEK signature in each form elliptic reads. The SDK sends `der`.
struct DekSignatures {
    der: String,
    hex: String,
    rs: String,
}

fn dek_signatures(message: &str, private_key: &str) -> DekSignatures {
    let der = dek_auth_by(private_key, message);
    let der_bytes: Vec<u8> = serde_json::from_str(&der).unwrap();
    let (r, s) = Signature::from_der(&der_bytes).unwrap().split_bytes();
    // elliptic writes r and s with BN's `toString(16)`, which drops leading zeros.
    let unpadded = |bytes: &[u8]| hex::encode(bytes).trim_start_matches('0').to_string();
    DekSignatures {
        hex: string(&hex::encode(&der_bytes)),
        rs: format!(r#"{{"r":"{}","s":"{}"}}"#, unpadded(&r), unpadded(&s)),
        der,
    }
}

// -- Requests --

/// A header as Node's `http.request` takes it: a name and its values, one line per value. A name
/// without values is not sent.
type Header = (&'static str, Vec<String>);

fn header(name: &'static str, value: &str) -> Header {
    (name, vec![value.to_string()])
}

fn header_values(name: &'static str, values: &[&str]) -> Header {
    (name, values.iter().map(|value| value.to_string()).collect())
}

fn no_header(name: &'static str) -> Header {
    (name, vec![])
}

fn post(path: &str, body: impl Into<Vec<u8>>, headers: Vec<Header>) -> RawRequest {
    RawRequest {
        method: "POST",
        path: path.to_string(),
        headers: headers
            .into_iter()
            .flat_map(|(name, values)| {
                values
                    .into_iter()
                    .map(move |value| (name.to_string(), value))
            })
            .collect(),
        body: body.into(),
    }
}

fn request(method: &'static str, path: &str) -> RawRequest {
    RawRequest {
        method,
        path: path.to_string(),
        headers: vec![],
        body: vec![],
    }
}

/// POSTs `body` as JSON, signed over `canonical`, the text the TS combiner checks the signature
/// over: the body as body-parser reads it (UTF-8, a leading byte order mark dropped), written back
/// by `JSON.stringify`. It is signed with the DEK when it names `encryption_key`, else with the
/// user's wallet key. `overrides` replace headers of the same name where they stand, or add to
/// them.
fn signed_over(
    path: &str,
    user: &TestUser,
    body: impl Into<Vec<u8>>,
    canonical: &str,
    overrides: Vec<Header>,
) -> RawRequest {
    // Every canonical text here is compact, and none names a nested authenticationMethod.
    let authorization =
        if canonical.contains(&format!(r#""authenticationMethod":{ENCRYPTION_KEY}"#)) {
            dek_signatures(canonical, DEK_PRIVATE_KEY).der
        } else {
            wallet_key_signature(user, canonical)
        };
    let mut headers = vec![
        header("content-type", "application/json"),
        header("authorization", &authorization),
    ];
    for (name, values) in overrides {
        match headers.iter_mut().find(|(existing, _)| *existing == name) {
            Some(existing) => existing.1 = values,
            None => headers.push((name, values)),
        }
    }
    post(path, body, headers)
}

/// POSTs the compact JSON `body`, which is its own canonical text, signed as `signed_over` signs.
fn signed(path: &str, user: &TestUser, body: &str, overrides: Vec<Header>) -> RawRequest {
    signed_over(path, user, body, body, overrides)
}

/// A request as the SDK sends it through cross-fetch
/// (`odis-combiner-4.0.5:packages/identity/src/odis/query.ts`).
fn sdk_request(path: &str, user: &TestUser, fields: Fields) -> RawRequest {
    let body = fields.compact();
    let authorization = if fields.is_encryption_key() {
        dek_auth(&body)
    } else {
        viem_signature(user, &body)
    };
    post(
        path,
        body,
        vec![
            header("accept", "application/json"),
            header("content-type", "application/json"),
            // The SDK sets the header to `undefined` without a key version, and fetch sends it as
            // text.
            header("odis-key-version", "undefined"),
            header("authorization", &authorization),
            header(
                "user-agent",
                "node-fetch/1.0 (+https://github.com/bitinn/node-fetch)",
            ),
            header("accept-encoding", "gzip,deflate"),
        ],
    )
}

/// `user` with the hex digits of the account in upper case.
fn upper_case(user: &TestUser) -> TestUser {
    TestUser {
        private_key: user.private_key,
        account: format!("0x{}", user.account[2..].to_uppercase()),
    }
}

// -- Answers --

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

const CORS: [(&str, &str); 3] = [
    ("access-control-allow-origin", "*"),
    ("access-control-allow-methods", "GET, POST, OPTIONS"),
    (
        "access-control-allow-headers",
        "Origin, X-Requested-With, Content-Type, Accept, Authorization, odis-key-version",
    ),
];

/// The CORS headers and `extra`.
fn cors_and(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    headers(&[&CORS[..], extra].concat())
}

fn answer(status: u16, body: &str, headers: BTreeMap<String, String>) -> PinnedAnswer {
    PinnedAnswer {
        status: Some(status),
        body: Some(body.to_string()),
        headers: Some(headers),
    }
}

fn json_answer(status: u16, body: &str) -> PinnedAnswer {
    answer(
        status,
        body,
        cors_and(&[("content-type", "application/json")]),
    )
}

fn signed_answer(signature: &str) -> PinnedAnswer {
    json_answer(
        200,
        &format!(
            r#"{{"success":true,"version":"<version>","signature":"{signature}","performedQueryCount":1,"totalQuota":1000,"warnings":[]}}"#
        ),
    )
}

fn quota_answer() -> PinnedAnswer {
    json_answer(
        200,
        r#"{"success":true,"version":"<version>","performedQueryCount":0,"totalQuota":1000,"warnings":[]}"#,
    )
}

fn failure(status: u16, error: &str) -> PinnedAnswer {
    json_answer(
        status,
        &format!(r#"{{"success":false,"version":"<version>","error":"{error}"}}"#),
    )
}

/// Express's error page (finalhandler) under NODE_ENV=production.
fn express_error(status: u16, message: &str, extra: &[(&str, &str)]) -> PinnedAnswer {
    answer(
        status,
        &format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{message}</pre>\n</body>\n</html>\n"
        ),
        headers(&[extra, &[("content-type", "text/html")]].concat()),
    )
}

fn rust_empty_404() -> PinnedAnswer {
    answer(404, "", cors_and(&[]))
}

fn invalid_input() -> PinnedAnswer {
    failure(400, INVALID_INPUT)
}

fn unauthenticated() -> PinnedAnswer {
    failure(401, UNAUTHENTICATED_USER)
}

fn invalid_key_version() -> PinnedAnswer {
    failure(400, INVALID_KEY_VERSION_REQUEST)
}

// -- Comparison families --

/// A body with an extra field that both combiners accept and sign. `canonical` is the extra
/// field as `JSON.stringify` writes it back.
fn extra_field(name: &str, members: impl Into<String>, canonical: impl Into<String>) -> Comparison {
    let (members, canonical) = (members.into(), canonical.into());
    Comparison::same_answer(
        format!("/sign with an extra field: {name}"),
        200,
        None,
        move |user| {
            signed_over(
                "/sign",
                user,
                sign_body_with(user, &members),
                &sign_body_with(user, &canonical),
                vec![],
            )
        },
    )
}

/// A body the Rust combiner refuses with 400 WARN_01 and the TS combiner signs. `body` gives the
/// bytes sent and `canonical` the text the TS combiner writes them back to.
fn ts_signs_rust_refuses(
    name: &str,
    body: impl Fn(&TestUser) -> Vec<u8> + 'static,
    canonical: impl Fn(&TestUser) -> String + 'static,
) -> Comparison {
    Comparison::known_difference(
        name,
        signed_answer(SIGNATURE_V1),
        invalid_input(),
        move |user| signed_over("/sign", user, body(user), &canonical(user), vec![]),
    )
}

/// `ts_signs_rust_refuses` for a sign body with `members` appended, which the TS combiner writes
/// back with `canonical` appended instead.
fn ts_signs_rust_refuses_member(
    name: &str,
    members: impl Into<String>,
    canonical: impl Into<String>,
) -> Comparison {
    let (members, canonical) = (members.into(), canonical.into());
    ts_signs_rust_refuses(
        name,
        move |user| sign_body_with(user, &members).into_bytes(),
        move |user| sign_body_with(user, &canonical),
    )
}

pub fn all() -> Vec<Comparison> {
    let mut comparisons = vec![
        // Both endpoints, both authentication methods, each key version.
        Comparison::same_answer("/sign with wallet key", 200, None, |user| {
            signed("/sign", user, &sign_fields(user).compact(), vec![])
        }),
    ];
    for version in ["1", "2", "3"] {
        comparisons.push(Comparison::same_answer(
            format!("/sign with key version {version}"),
            200,
            None,
            move |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("odis-key-version", version)],
                )
            },
        ));
    }
    comparisons.extend([
        Comparison::same_answer("/sign with DEK", 200, None, |user| {
            let fields = sign_fields(user).set("authenticationMethod", ENCRYPTION_KEY);
            signed("/sign", user, &fields.compact(), vec![])
        }),
        Comparison::same_answer("/quotaStatus with wallet key", 200, None, |user| {
            signed("/quotaStatus", user, &quota_fields(user).compact(), vec![])
        }),
        Comparison::same_answer("/quotaStatus with DEK", 200, None, |user| {
            let fields = quota_fields(user).set("authenticationMethod", ENCRYPTION_KEY);
            signed("/quotaStatus", user, &fields.compact(), vec![])
        }),
        // The SDK's bytes and headers (`odis-combiner-4.0.5:packages/identity/src/odis/identifier.ts`
        // and `quota.ts`). Without a client version the SDK sets `version: undefined`, which
        // JSON.stringify leaves out.
        Comparison::same_answer("/sign from the SDK with wallet key", 200, None, |user| {
            sdk_request("/sign", user, sign_fields(user))
        }),
        Comparison::same_answer(
            "/sign from the SDK with DEK and a client version",
            200,
            None,
            |user| {
                let fields = Fields(vec![
                    ("account", string(&user.account)),
                    ("blindedQueryPhoneNumber", string(&BLINDED_QUERY)),
                    ("version", string("5.1.2")),
                    ("authenticationMethod", ENCRYPTION_KEY.to_string()),
                    ("sessionID", string(SESSION_ID)),
                ]);
                sdk_request("/sign", user, fields)
            },
        ),
        Comparison::same_answer(
            "/quotaStatus from the SDK with wallet key",
            200,
            None,
            |user| sdk_request("/quotaStatus", user, quota_fields(user)),
        ),
        Comparison::same_answer("/quotaStatus from the SDK with DEK", 200, None, |user| {
            let fields = quota_fields(user).set("authenticationMethod", ENCRYPTION_KEY);
            sdk_request("/quotaStatus", user, fields)
        }),
        // Whitespace: the signature is checked over the compact form.
        Comparison::same_answer(
            "/sign signed compact and sent with whitespace, wallet key",
            200,
            None,
            |user| {
                let fields = sign_fields(user);
                signed_over("/sign", user, fields.pretty(), &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign signed compact and sent with whitespace, DEK",
            200,
            None,
            |user| {
                let fields = sign_fields(user).set("authenticationMethod", ENCRYPTION_KEY);
                signed_over("/sign", user, fields.pretty(), &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign signed and sent with whitespace",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let body = sign_fields(user).pretty();
                let authorization = wallet_key_signature(user, &body);
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        // Quota.
        Comparison::same_answer("/sign out of quota", 403, Some(EXCEEDED_QUOTA), |_| {
            let user = &*USER_WITHOUT_QUOTA;
            signed("/sign", user, &sign_fields(user).compact(), vec![])
        }),
        Comparison::same_answer("/quotaStatus without quota", 200, None, |_| {
            let user = &*USER_WITHOUT_QUOTA;
            signed("/quotaStatus", user, &quota_fields(user).compact(), vec![])
        }),
        // Extra fields that both write back alike.
        extra_field(
            "emoji",
            "\"extra\":\"a\u{1F600}b\"",
            "\"extra\":\"a\u{1F600}b\"",
        ),
        extra_field("CJK", r#""extra":"中文""#, r#""extra":"中文""#),
        extra_field(
            "escaped newline and tab",
            r#""extra":"a\nb\tc""#,
            r#""extra":"a\nb\tc""#,
        ),
        extra_field(
            r"escapes \u0001 and \u001f",
            r#""extra":"\u0001\u001f""#,
            r#""extra":"\u0001\u001f""#,
        ),
        extra_field(
            "escaped quote and backslash",
            r#""extra":"\"\\""#,
            r#""extra":"\"\\""#,
        ),
        extra_field("escaped slash", r#""extra":"a\/b""#, r#""extra":"a/b""#),
        extra_field(
            "U+2028 and U+2029",
            "\"extra\":\"\u{2028}\u{2029}\"",
            "\"extra\":\"\u{2028}\u{2029}\"",
        ),
        extra_field("U+007F", "\"extra\":\"\u{7f}\"", "\"extra\":\"\u{7f}\""),
        extra_field(
            r"escapes \b \f \r and \u000b",
            r#""extra":"\b\f\r\u000b""#,
            r#""extra":"\b\f\r\u000b""#,
        ),
        extra_field(
            "upper-case hex escapes",
            "\"extra\":\"\\u00E9\\u00AB\"",
            "\"extra\":\"\u{e9}\u{ab}\"",
        ),
        extra_field("empty key", r#""":1"#, r#""":1"#),
        extra_field(
            "__proto__ key",
            r#""__proto__":{"x":1}"#,
            r#""__proto__":{"x":1}"#,
        ),
        extra_field(
            "nested objects and arrays",
            r#""extra":{"b":[1,{"c":[]},[[]],{}],"a":{"z":1,"y":2}}"#,
            r#""extra":{"b":[1,{"c":[]},[[]],{}],"a":{"z":1,"y":2}}"#,
        ),
        extra_field(
            "true, false and null",
            r#""extra":[true,false,null]"#,
            r#""extra":[true,false,null]"#,
        ),
        extra_field(
            "largest and smallest safe integers",
            r#""max":9007199254740991,"min":-9007199254740991"#,
            r#""max":9007199254740991,"min":-9007199254740991"#,
        ),
        extra_field(
            "127 levels of nesting",
            nested("extra", 126),
            nested("extra", 126),
        ),
        Comparison::same_answer(
            "/sign with a canonical body of 16,000 characters",
            200,
            None,
            |user| signed("/sign", user, &padded_to(user, 16_000).compact(), vec![]),
        ),
        // Key versions.
        Comparison::same_answer(
            "/sign with an unsupported key version and a bad signature",
            400,
            Some(INVALID_KEY_VERSION_REQUEST),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![
                        header("odis-key-version", "4"),
                        header(
                            "authorization",
                            &wallet_key_signature(user, "something else"),
                        ),
                    ],
                )
            },
        ),
        Comparison::same_answer(
            "/quotaStatus with an unsupported key version and a bad signature",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                signed(
                    "/quotaStatus",
                    user,
                    &quota_fields(user).compact(),
                    vec![
                        header("odis-key-version", "4"),
                        header(
                            "authorization",
                            &wallet_key_signature(user, "something else"),
                        ),
                    ],
                )
            },
        ),
        // Both read it as a version that is not configured; on /quotaStatus they differ (below).
        Comparison::same_answer(
            "/sign with key version 4294967296",
            400,
            Some(INVALID_KEY_VERSION_REQUEST),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("odis-key-version", "4294967296")],
                )
            },
        ),
        Comparison::same_answer(r#"/sign with key version "invalid""#, 200, None, |user| {
            signed(
                "/sign",
                user,
                &sign_fields(user).compact(),
                vec![header("odis-key-version", "invalid")],
            )
        }),
        Comparison::same_answer(r#"/sign with key version " 2 ""#, 200, None, |user| {
            signed(
                "/sign",
                user,
                &sign_fields(user).compact(),
                vec![header("odis-key-version", " 2 ")],
            )
        }),
        // TS reads 1.0 as version 1 and Rust as absent, which is the current version 1 too.
        Comparison::same_answer("/sign with key version 1.0", 200, None, |user| {
            signed(
                "/sign",
                user,
                &sign_fields(user).compact(),
                vec![header("odis-key-version", "1.0")],
            )
        }),
        // Node joins the values to `2, 3`, which TS reads as NaN; both use the current version.
        Comparison::same_answer(
            "/sign with the key version header sent twice",
            200,
            None,
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header_values("odis-key-version", &["2", "3"])],
                )
            },
        ),
        // Accounts and authentication.
        Comparison::same_answer(
            "/sign with an upper-case account, wallet key",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let upper = upper_case(user);
                signed("/sign", &upper, &sign_fields(&upper).compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with an upper-case account, DEK",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let upper = upper_case(user);
                let fields = sign_fields(&upper).set("authenticationMethod", ENCRYPTION_KEY);
                signed("/sign", &upper, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with an account without 0x",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let fields = sign_fields(user).set("account", &string(&user.account[2..]));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with an account with 0X",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let account = format!("0X{}", &user.account[2..]);
                let fields = sign_fields(user).set("account", &string(&account));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign without an Authorization header",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![no_header("authorization")],
                )
            },
        ),
        Comparison::same_answer(
            "/sign signed by another wallet key",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let other = test_user(&format!("{} other", user.account));
                signed("/sign", &other, &sign_fields(user).compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with a wallet-key signature without 0x",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let body = sign_fields(user).compact();
                let authorization = wallet_key_signature(user, &body);
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization[2..]),
                    ],
                )
            },
        ),
        Comparison::same_answer(
            "/sign with DEK signed by another key",
            401,
            Some(UNAUTHENTICATED_USER),
            |user| {
                let body = sign_fields(user)
                    .set("authenticationMethod", ENCRYPTION_KEY)
                    .compact();
                let authorization = dek_signatures(&body, OTHER_DEK_PRIVATE_KEY).der;
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        Comparison::same_answer(
            "/sign with DEK signed by the wallet key falls back to it",
            200,
            None,
            |user| {
                let body = sign_fields(user)
                    .set("authenticationMethod", ENCRYPTION_KEY)
                    .compact();
                let authorization = wallet_key_signature(user, &body);
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        Comparison::same_answer(
            "/sign with DEK for an account without a DEK, signed by the wallet key",
            200,
            None,
            |_| {
                let user = &*USER_WITHOUT_DEK;
                let body = sign_fields(user)
                    .set("authenticationMethod", ENCRYPTION_KEY)
                    .compact();
                let authorization = wallet_key_signature(user, &body);
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        // Bodies both refuse.
        Comparison::same_answer(
            "/sign without an account",
            400,
            Some(INVALID_INPUT),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).without("account").compact(),
                    vec![],
                )
            },
        ),
        Comparison::same_answer(
            "/sign with an account of 39 hex digits",
            400,
            Some(INVALID_INPUT),
            |user| {
                let account = &user.account[..user.account.len() - 1];
                let fields = sign_fields(user).set("account", &string(account));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with an account as an array of 20 numbers",
            400,
            Some(INVALID_INPUT),
            |user| {
                let bytes = hex::decode(&user.account[2..]).unwrap();
                let fields =
                    sign_fields(user).set("account", &serde_json::to_string(&bytes).unwrap());
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign without a blinded query",
            400,
            Some(INVALID_INPUT),
            |user| {
                let fields = sign_fields(user).without("blindedQueryPhoneNumber");
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with a blinded query of 63 characters",
            400,
            Some(INVALID_INPUT),
            |user| {
                let fields =
                    sign_fields(user).set("blindedQueryPhoneNumber", &string(&BLINDED_QUERY[1..]));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with a blinded query that is not base64",
            400,
            Some(INVALID_INPUT),
            |user| {
                let query = format!("!{}", &BLINDED_QUERY[1..]);
                let fields = sign_fields(user).set("blindedQueryPhoneNumber", &string(&query));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with a numeric sessionID",
            400,
            Some(INVALID_INPUT),
            |user| {
                let fields = sign_fields(user).set("sessionID", "7");
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
    ]);
    for field in ["authenticationMethod", "sessionID", "version"] {
        comparisons.push(Comparison::same_answer(
            format!("/sign with {field} null"),
            400,
            Some(INVALID_INPUT),
            move |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).set(field, "null").compact(),
                    vec![],
                )
            },
        ));
    }
    comparisons.extend([
        Comparison::same_answer(
            "/quotaStatus without an account",
            400,
            Some(INVALID_INPUT),
            |user| {
                let fields = quota_fields(user).without("account");
                signed("/quotaStatus", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/quotaStatus with authenticationMethod null",
            400,
            Some(INVALID_INPUT),
            |user| {
                let fields = quota_fields(user).set("authenticationMethod", "null");
                signed("/quotaStatus", user, &fields.compact(), vec![])
            },
        ),
        Comparison::same_answer(
            "/sign with a top-level array",
            400,
            Some(INVALID_INPUT),
            |user| {
                let body = format!("[{}]", sign_fields(user).compact());
                signed("/sign", user, &body, vec![])
            },
        ),
        Comparison::same_answer("/sign with an empty body", 400, Some(INVALID_INPUT), |_| {
            post(
                "/sign",
                "",
                vec![header("content-type", "application/json")],
            )
        }),
        Comparison::same_answer(
            "/sign with a canonical body of 16,001 characters",
            400,
            Some(INVALID_INPUT),
            |user| signed("/sign", user, &padded_to(user, 16_001).compact(), vec![]),
        ),
        Comparison::same_answer(
            "/sign with Content-Type text/plain",
            400,
            Some(INVALID_INPUT),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-type", "text/plain")],
                )
            },
        ),
        Comparison::same_answer(
            "/sign without a Content-Type",
            400,
            Some(INVALID_INPUT),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![no_header("content-type")],
                )
            },
        ),
    ]);
    // Content types that real HTTP libraries send, and content types that type-is parses, or
    // not; Express skips the body of one it cannot parse.
    let content_types = [
        ("application/json;charset=UTF-8", 200, None),
        (r#"application/json; charset="utf-8""#, 200, None),
        (r#"application/json; charset="""#, 200, None),
        ("application/json; charset=latin1; charset=utf-8", 200, None),
        (r#"application/json ; foo = "a;b""#, 200, None),
        ("application/json;", 400, Some(INVALID_INPUT)),
        ("application/json; foo", 400, Some(INVALID_INPUT)),
        (
            "application/json; charset=utf-8; foo",
            400,
            Some(INVALID_INPUT),
        ),
        ("application/json; charset=", 400, Some(INVALID_INPUT)),
    ];
    for (content_type, status, error) in content_types {
        comparisons.push(Comparison::same_answer(
            format!("/sign with Content-Type {content_type}"),
            status,
            error,
            move |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-type", content_type)],
                )
            },
        ));
    }
    // Content encodings body-parser reads as none.
    for encoding in ["identity", "IDENTITY", ""] {
        comparisons.push(Comparison::same_answer(
            format!(r#"/sign with Content-Encoding "{encoding}""#),
            200,
            None,
            move |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-encoding", encoding)],
                )
            },
        ));
    }
    // Routing and transport ("Routes and transport" in the changelog).
    for path in ["/sign", "/quotaStatus"] {
        // Express answers OPTIONS itself for a path it routes, with the methods as the body.
        comparisons.push(Comparison::known_difference(
            format!("OPTIONS {path}"),
            answer(
                200,
                "POST",
                cors_and(&[("allow", "POST"), ("content-type", "text/html")]),
            ),
            answer(200, "", cors_and(&[("allow", "POST")])),
            move |_| request("OPTIONS", path),
        ));
    }
    comparisons.extend([
        Comparison::known_difference(
            "/SIGN",
            signed_answer(SIGNATURE_V1),
            rust_empty_404(),
            |user| signed("/SIGN", user, &sign_fields(user).compact(), vec![]),
        ),
        Comparison::known_difference("/quotastatus", quota_answer(), rust_empty_404(), |user| {
            signed("/quotastatus", user, &quota_fields(user).compact(), vec![])
        }),
        Comparison::known_difference(
            "/sign/",
            signed_answer(SIGNATURE_V1),
            rust_empty_404(),
            |user| signed("/sign/", user, &sign_fields(user).compact(), vec![]),
        ),
        Comparison::known_difference(
            "GET /sign",
            express_error(404, "Cannot GET /sign", &CORS),
            answer(405, "", cors_and(&[("allow", "POST")])),
            |_| request("GET", "/sign"),
        ),
        Comparison::known_difference(
            "GET on an unknown path",
            express_error(404, "Cannot GET /unknown", &CORS),
            rust_empty_404(),
            |_| request("GET", "/unknown"),
        ),
        Comparison::known_difference(
            "OPTIONS on an unknown path",
            express_error(404, "Cannot OPTIONS /unknown", &CORS),
            answer(200, "", cors_and(&[])),
            |_| request("OPTIONS", "/unknown"),
        ),
        Comparison::known_difference(
            "/sign with Content-Encoding gzip",
            signed_answer(SIGNATURE_V1),
            invalid_input(),
            |user| {
                let body = sign_fields(user).compact();
                let authorization = wallet_key_signature(user, &body);
                post(
                    "/sign",
                    gzip(&body),
                    vec![
                        header("content-type", "application/json"),
                        header("content-encoding", "gzip"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with Content-Encoding gzip and a plain body",
            express_error(400, "Bad Request", &[]),
            invalid_input(),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-encoding", "gzip")],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with Content-Encoding br",
            express_error(415, "Unsupported Media Type", &[]),
            invalid_input(),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-encoding", "br")],
                )
            },
        ),
        // Node joins the values to `identity, identity`, which body-parser does not know.
        Comparison::known_difference(
            "/sign with Content-Encoding identity sent twice",
            express_error(415, "Unsupported Media Type", &[]),
            invalid_input(),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header_values("content-encoding", &["identity", "identity"])],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with malformed JSON",
            express_error(400, "Bad Request", &[]),
            invalid_input(),
            |_| {
                post(
                    "/sign",
                    r#"{"account":"#,
                    vec![header("content-type", "application/json")],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with charset utf-16",
            signed_answer(SIGNATURE_V1),
            invalid_input(),
            |user| {
                let body = sign_fields(user).compact();
                let authorization = wallet_key_signature(user, &body);
                let mut bytes = vec![0xff, 0xfe];
                bytes.extend(body.encode_utf16().flat_map(u16::to_le_bytes));
                post(
                    "/sign",
                    bytes,
                    vec![
                        header("content-type", "application/json; charset=utf-16"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with charset latin1",
            express_error(415, "Unsupported Media Type", &[]),
            invalid_input(),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("content-type", "application/json; charset=latin1")],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with a body of 300 KB",
            express_error(413, "Payload Too Large", &[]),
            answer(
                413,
                "length limit exceeded",
                cors_and(&[("content-type", "text/plain")]),
            ),
            |user| {
                signed(
                    "/sign",
                    user,
                    &padded_to(user, 300 * 1024).compact(),
                    vec![],
                )
            },
        ),
        // Without a Content-Length the limit is met while the body is read, not before.
        Comparison::known_difference(
            "/sign with a chunked body of 300 KB",
            express_error(413, "Payload Too Large", &[]),
            answer(
                413,
                "Failed to buffer the request body: length limit exceeded",
                cors_and(&[("content-type", "text/plain")]),
            ),
            |user| {
                signed(
                    "/sign",
                    user,
                    &padded_to(user, 300 * 1024).compact(),
                    vec![header("transfer-encoding", "chunked")],
                )
            },
        ),
        // Request fields ("Request fields" in the changelog).
        Comparison::known_difference(
            "/quotaStatus with an unsupported key version",
            failure(500, UNKNOWN_ERROR),
            invalid_key_version(),
            |user| {
                signed(
                    "/quotaStatus",
                    user,
                    &quota_fields(user).compact(),
                    vec![header("odis-key-version", "4")],
                )
            },
        ),
        Comparison::known_difference(
            "/quotaStatus with key version 4294967296",
            failure(500, UNKNOWN_ERROR),
            invalid_key_version(),
            |user| {
                signed(
                    "/quotaStatus",
                    user,
                    &quota_fields(user).compact(),
                    vec![header("odis-key-version", "4294967296")],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with key version 2.0",
            signed_answer(SIGNATURE_V2),
            signed_answer(SIGNATURE_V1),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("odis-key-version", "2.0")],
                )
            },
        ),
        Comparison::known_difference(
            "/sign with a key version of 400 digits",
            signed_answer(SIGNATURE_V1),
            invalid_key_version(),
            |user| {
                signed(
                    "/sign",
                    user,
                    &sign_fields(user).compact(),
                    vec![header("odis-key-version", &"1".repeat(400))],
                )
            },
        ),
        // TS's is-base64 accepts padding bits that are not zero, so the TS combiner forwards the
        // query. Depends on the signers being Rust: they refuse it with 400, and the TS combiner
        // answers with their status. TS signers accept it and then cannot sign, and in front of
        // them the TS combiner answers 500.
        Comparison::known_difference(
            "/sign with a blinded query in non-canonical base64",
            failure(400, NOT_ENOUGH_PARTIAL_SIGNATURES),
            invalid_input(),
            |user| {
                let query = format!("{}B==", "A".repeat(61));
                let fields = sign_fields(user).set("blindedQueryPhoneNumber", &string(&query));
                signed("/sign", user, &fields.compact(), vec![])
            },
        ),
        // Bodies the Rust combiner cannot write back as JavaScript does ("Request bodies").
        ts_signs_rust_refuses_member("/sign with 1.0", r#""extra":1.0"#, r#""extra":1"#),
        ts_signs_rust_refuses_member("/sign with 1e2", r#""extra":1e2"#, r#""extra":100"#),
        ts_signs_rust_refuses_member("/sign with -0", r#""extra":-0"#, r#""extra":0"#),
        ts_signs_rust_refuses_member("/sign with 1e400", r#""extra":1e400"#, r#""extra":null"#),
        // The nearest double, as JavaScript writes it.
        ts_signs_rust_refuses_member(
            "/sign with a 20-digit integer",
            r#""extra":12345678901234567890"#,
            r#""extra":12345678901234567000"#,
        ),
        // The last value, where the key first stands.
        ts_signs_rust_refuses_member(
            "/sign with a duplicate key",
            r#""extra":1,"extra":2"#,
            r#""extra":2"#,
        ),
        // JavaScript writes integer-like keys first.
        ts_signs_rust_refuses(
            "/sign with an integer-like key",
            |user| sign_body_with(user, r#""1":"x""#).into_bytes(),
            |user| format!(r#"{{"1":"x",{}"#, &sign_fields(user).compact()[1..]),
        ),
        ts_signs_rust_refuses_member(
            "/sign with a lone surrogate escape",
            r#""extra":"\ud83d""#,
            r#""extra":"\ud83d""#,
        ),
        ts_signs_rust_refuses(
            "/sign with a byte order mark",
            |user| [&[0xef, 0xbb, 0xbf], sign_fields(user).compact().as_bytes()].concat(),
            |user| sign_fields(user).compact(),
        ),
        // body-parser reads the byte as U+FFFD.
        ts_signs_rust_refuses(
            "/sign with an invalid UTF-8 byte",
            |user| {
                let start = sign_body_with(user, r#""extra":"x"#);
                [&start.as_bytes()[..start.len() - 1], &[0xff], br#"y"}"#].concat()
            },
            |user| sign_body_with(user, "\"extra\":\"x\u{FFFD}y\""),
        ),
        ts_signs_rust_refuses_member(
            "/sign with 128 levels of nesting",
            nested("extra", 127),
            nested("extra", 127),
        ),
        // Signatures the TS services read loosely ("Authentication" in the changelog). The TS
        // combiner accepts each and forwards it. The next three comparisons depend on the signers
        // being Rust: they refuse it with 401, where TS signers sign, and the TS combiner answers
        // with their status. Its error is ERR_07, not the WARN_02 of a signature it refuses itself.
        //
        // The Rust signers take only a lower-case `0x`.
        Comparison::known_difference(
            "/sign with a wallet-key signature prefixed 0X",
            failure(401, NOT_ENOUGH_PARTIAL_SIGNATURES),
            unauthenticated(),
            |user| {
                let body = sign_fields(user).compact();
                let authorization = format!("0X{}", &wallet_key_signature(user, &body)[2..]);
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        // The Rust signers take a DEK signature only as a JSON array of DER bytes.
        Comparison::known_difference(
            "/sign with a DEK signature as a hex string",
            failure(401, NOT_ENOUGH_PARTIAL_SIGNATURES),
            unauthenticated(),
            |user| {
                let body = sign_fields(user)
                    .set("authenticationMethod", ENCRYPTION_KEY)
                    .compact();
                let authorization = dek_signatures(&body, DEK_PRIVATE_KEY).hex;
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
        // The Rust signers take a DEK signature only as a JSON array of DER bytes.
        Comparison::known_difference(
            "/sign with a DEK signature as r and s",
            failure(401, NOT_ENOUGH_PARTIAL_SIGNATURES),
            unauthenticated(),
            |user| {
                let body = sign_fields(user)
                    .set("authenticationMethod", ENCRYPTION_KEY)
                    .compact();
                let authorization = dek_signatures(&body, DEK_PRIVATE_KEY).rs;
                post(
                    "/sign",
                    body,
                    vec![
                        header("content-type", "application/json"),
                        header("authorization", &authorization),
                    ],
                )
            },
        ),
    ]);
    comparisons
}
