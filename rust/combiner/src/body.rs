//! The request body as the TS combiner sees it: parsed, then written back as
//! `JSON.stringify` writes it. The signature check, the length limit and the signer calls all
//! use that canonical text, so a body the writer cannot reproduce exactly as JavaScript would
//! is refused.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};

/// The most UTF-16 code units the canonical body may have.
pub const BODY_CHAR_LIMIT: usize = 16_000;

/// The largest integer JavaScript holds exactly, 2^53 - 1.
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// A JSON value as far as the canonical body needs one: integers are the only numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    Integer(i64),
    String(String),
    Array(Vec<JsValue>),
    Object(Vec<(String, JsValue)>),
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum BodyError {
    #[error("the Content-Type is not application/json")]
    MediaType,
    #[error("a Content-Type parameter is not a name=value pair with a token or quoted value")]
    ContentTypeParameter,
    #[error("the charset is not UTF-8")]
    Charset,
    #[error("the body has a Content-Encoding other than identity")]
    ContentEncoding,
    #[error("the body is not valid JSON: {0}")]
    Json(String),
    #[error("the body is not a JSON object")]
    NotAnObject,
    #[error("a number is not an integer between -(2^53 - 1) and 2^53 - 1")]
    UnsafeNumber,
    #[error("the key {0:?} appears twice in one object")]
    DuplicateKey(String),
    #[error("the key {0:?} is an integer, which JavaScript moves to the front")]
    IntegerKey(String),
    #[error("the canonical body is {0} UTF-16 code units long")]
    TooLong(usize),
}

/// A request body that passed every check, with the text JavaScript would write for it.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalBody {
    text: String,
    fields: Vec<(String, JsValue)>,
}

impl CanonicalBody {
    /// Checks the `Content-Type` and `Content-Encoding` as `express.json()` does, then parses
    /// `raw` and writes it back in canonical form. The headers are checked before the body is read.
    pub fn parse(headers: &HeaderMap, raw: &[u8]) -> Result<Self, BodyError> {
        check_content_type(headers.get(CONTENT_TYPE))?;
        check_content_encoding(headers)?;

        let failure = RefCell::new(None);
        let mut deserializer = serde_json::Deserializer::from_slice(raw);
        let parsed = ValueSeed { failure: &failure }
            .deserialize(&mut deserializer)
            .and_then(|value| deserializer.end().map(|()| value));
        let value = parsed.map_err(|error| {
            failure
                .into_inner()
                .unwrap_or_else(|| BodyError::Json(error.to_string()))
        })?;
        let JsValue::Object(fields) = value else {
            return Err(BodyError::NotAnObject);
        };

        let mut text = String::new();
        write_object(&fields, &mut text);
        let units = text.encode_utf16().count();
        if units > BODY_CHAR_LIMIT {
            return Err(BodyError::TooLong(units));
        }
        Ok(Self { text, fields })
    }

    /// The body as JSON.stringify writes it: compact, keys in the order sent.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// A top-level field of the body.
    pub fn field(&self, key: &str) -> Option<&JsValue> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// Accepts what Express's JSON parser accepts. type-is must parse the header with media-typer
/// 0.3.0, or Express skips the body: `application/json` in any case, then `; name=value`
/// parameters whose value is a token or a quoted string, with spaces around `;` and `=`. As in
/// body-parser, the last `charset` counts; it must be `utf-8` in any case, or empty, which
/// body-parser reads as UTF-8.
fn check_content_type(content_type: Option<&HeaderValue>) -> Result<(), BodyError> {
    // Node strips spaces and tabs around a header value; inside it media-typer allows spaces only.
    let value = content_type
        .ok_or(BodyError::MediaType)?
        .as_bytes()
        .trim_ascii();
    let split = value.iter().position(|&byte| byte == b';');
    let (media_type, mut parameters) = value.split_at(split.unwrap_or(value.len()));
    if !trim_spaces(media_type).eq_ignore_ascii_case(b"application/json") {
        return Err(BodyError::MediaType);
    }
    let mut charset = None;
    while let [b';', rest @ ..] = parameters {
        let (name, rest) = token(trim_spaces(rest)).ok_or(BodyError::ContentTypeParameter)?;
        let [b'=', rest @ ..] = trim_spaces(rest) else {
            return Err(BodyError::ContentTypeParameter);
        };
        let (value, rest) =
            parameter_value(trim_spaces(rest)).ok_or(BodyError::ContentTypeParameter)?;
        if name.eq_ignore_ascii_case(b"charset") {
            charset = Some(value);
        }
        parameters = trim_spaces(rest);
    }
    if !parameters.is_empty() {
        return Err(BodyError::ContentTypeParameter);
    }
    match charset {
        Some(charset) if !charset.is_empty() && !charset.eq_ignore_ascii_case(b"utf-8") => {
            Err(BodyError::Charset)
        }
        _ => Ok(()),
    }
}

/// body-parser inflates `gzip` and `deflate` and refuses other encodings. The combiner decodes
/// none, so it reads only a body sent without one: no header, an empty one, or `identity` in any
/// case. Node joins a repeated header into one value, which is then never `identity`.
fn check_content_encoding(headers: &HeaderMap) -> Result<(), BodyError> {
    let mut values = headers.get_all(CONTENT_ENCODING).iter();
    match (values.next(), values.next()) {
        (None, _) => Ok(()),
        (Some(value), None) => {
            let value = value.as_bytes().trim_ascii();
            if value.is_empty() || value.eq_ignore_ascii_case(b"identity") {
                Ok(())
            } else {
                Err(BodyError::ContentEncoding)
            }
        }
        (Some(_), Some(_)) => Err(BodyError::ContentEncoding),
    }
}

/// Removes leading and trailing spaces, not tabs, as media-typer's ` *` does.
fn trim_spaces(mut bytes: &[u8]) -> &[u8] {
    while let [b' ', rest @ ..] = bytes {
        bytes = rest;
    }
    while let [rest @ .., b' '] = bytes {
        bytes = rest;
    }
    bytes
}

/// A token as RFC 2616 and media-typer define it.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Splits a non-empty token off the front of `bytes`.
fn token(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = bytes
        .iter()
        .position(|&byte| !is_token_byte(byte))
        .unwrap_or(bytes.len());
    (end > 0).then(|| bytes.split_at(end))
}

/// Splits a parameter value off the front of `bytes`: a token, or a quoted string, which is
/// returned without its quotes and escapes.
fn parameter_value(bytes: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    let Some(mut rest) = bytes.strip_prefix(b"\"") else {
        return token(bytes).map(|(value, rest)| (value.to_vec(), rest));
    };
    let mut value = Vec::new();
    loop {
        match rest {
            [b'"', after @ ..] => return Some((value, after)),
            [b'\\', escaped @ 0x20..=0x7e, after @ ..] => {
                value.push(*escaped);
                rest = after;
            }
            [
                byte @ (b' ' | b'!' | 0x23..=0x5b | 0x5d..=0x7e | 0x80..=0xff),
                after @ ..,
            ] => {
                value.push(*byte);
                rest = after;
            }
            _ => return None,
        }
    }
}

/// Reads a JSON value into a `JsValue`, recording the reason for a refusal in `failure` because
/// serde_json turns a custom error into a message.
#[derive(Clone, Copy)]
struct ValueSeed<'a> {
    failure: &'a RefCell<Option<BodyError>>,
}

impl ValueSeed<'_> {
    fn refuse<E: serde::de::Error>(self, error: BodyError) -> E {
        let message = error.to_string();
        *self.failure.borrow_mut() = Some(error);
        E::custom(message)
    }
}

impl<'de> DeserializeSeed<'de> for ValueSeed<'_> {
    type Value = JsValue;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<JsValue, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ValueSeed<'_> {
    type Value = JsValue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<JsValue, E> {
        Ok(JsValue::Null)
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<JsValue, E> {
        Ok(JsValue::Bool(value))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<JsValue, E> {
        match i64::try_from(value) {
            Ok(value) if value <= MAX_SAFE_INTEGER => Ok(JsValue::Integer(value)),
            _ => Err(self.refuse(BodyError::UnsafeNumber)),
        }
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<JsValue, E> {
        if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(JsValue::Integer(value))
        } else {
            Err(self.refuse(BodyError::UnsafeNumber))
        }
    }

    /// serde_json delivers `1.0`, `1e2`, `-0` and integers beyond i64 and u64 as floats.
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<JsValue, E> {
        Err(self.refuse(BodyError::UnsafeNumber))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<JsValue, E> {
        Ok(JsValue::String(value.to_owned()))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<JsValue, E> {
        Ok(JsValue::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsValue, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(self)? {
            items.push(item);
        }
        Ok(JsValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsValue, A::Error> {
        let mut fields: Vec<(String, JsValue)> = Vec::new();
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
                return Err(self.refuse(BodyError::IntegerKey(key)));
            }
            if !seen.insert(key.clone()) {
                return Err(self.refuse(BodyError::DuplicateKey(key)));
            }
            fields.push((key, map.next_value_seed(self)?));
        }
        Ok(JsValue::Object(fields))
    }
}

fn write_value(value: &JsValue, out: &mut String) {
    match value {
        JsValue::Null => out.push_str("null"),
        JsValue::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        JsValue::Integer(value) => out.push_str(&value.to_string()),
        JsValue::String(value) => write_string(value, out),
        JsValue::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        JsValue::Object(fields) => write_object(fields, out),
    }
}

fn write_object(fields: &[(String, JsValue)], out: &mut String) {
    out.push('{');
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_string(key, out);
        out.push(':');
        write_value(value, out);
    }
    out.push('}');
}

/// serde_json and JSON.stringify escape the same characters: the quote, the backslash and the
/// control characters (short forms for \b \f \n \r \t, lower case `\u00xx` for the rest).
fn write_string(value: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(value).expect("a string always serializes"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderName;

    /// A header map with each header as given, a header named twice sent twice.
    fn headers(headers: &[(HeaderName, &[u8])]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.append(name, HeaderValue::from_bytes(value).unwrap());
        }
        map
    }

    fn json_headers() -> HeaderMap {
        headers(&[(CONTENT_TYPE, b"application/json")])
    }

    fn vectors() -> Vec<serde_json::Value> {
        let text = include_str!("../tests/data/canonical_body_vectors.json");
        serde_json::from_str(text).unwrap()
    }

    fn body_of(vector: &serde_json::Value) -> Vec<u8> {
        hex::decode(vector["bodyHex"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn canonical_form_equals_json_stringify() {
        let mut checked = 0;
        for vector in vectors() {
            if vector["rustRefuses"].as_bool().unwrap() {
                continue;
            }
            let description = vector["description"].as_str().unwrap();
            let parsed = CanonicalBody::parse(&json_headers(), &body_of(&vector))
                .unwrap_or_else(|e| panic!("{description}: {e:?}"));
            assert_eq!(
                parsed.as_str(),
                vector["js"].as_str().unwrap(),
                "{description}"
            );
            checked += 1;
        }
        assert!(checked > 20);
    }

    #[test]
    fn bodies_javascript_would_write_differently_are_refused() {
        let mut checked = 0;
        for vector in vectors() {
            if !vector["rustRefuses"].as_bool().unwrap() {
                continue;
            }
            let description = vector["description"].as_str().unwrap();
            let error = CanonicalBody::parse(&json_headers(), &body_of(&vector))
                .map(|b| b.as_str().to_string())
                .expect_err(description);
            match vector["reason"].as_str().unwrap() {
                "UnsafeNumber" => assert_eq!(error, BodyError::UnsafeNumber, "{description}"),
                "NotAnObject" => assert_eq!(error, BodyError::NotAnObject, "{description}"),
                "DuplicateKey" => {
                    assert!(matches!(error, BodyError::DuplicateKey(_)), "{description}")
                }
                "IntegerKey" => assert!(matches!(error, BodyError::IntegerKey(_)), "{description}"),
                "Json" => assert!(
                    matches!(error, BodyError::Json(_)),
                    "{description}: {error:?}"
                ),
                other => panic!("unknown reason {other}"),
            }
            checked += 1;
        }
        assert!(checked > 15);
    }

    #[test]
    fn refusals_name_the_offending_key() {
        let parse = |s: &str| CanonicalBody::parse(&json_headers(), s.as_bytes()).err();
        assert_eq!(
            parse(r#"{"a":1,"a":2}"#),
            Some(BodyError::DuplicateKey("a".into()))
        );
        assert_eq!(
            parse(r#"{"12":1}"#),
            Some(BodyError::IntegerKey("12".into()))
        );
        assert_eq!(parse(r#"{"a1":1,"-1":2,"1a":3,"":4}"#), None);
    }

    #[test]
    fn fields_keep_their_values_and_order() {
        let body =
            CanonicalBody::parse(&json_headers(), br#" { "b" : [1, null], "a" : "x" } "#).unwrap();
        assert_eq!(body.as_str(), r#"{"b":[1,null],"a":"x"}"#);
        assert_eq!(body.field("a"), Some(&JsValue::String("x".into())));
        assert_eq!(
            body.field("b"),
            Some(&JsValue::Array(vec![JsValue::Integer(1), JsValue::Null]))
        );
        assert_eq!(body.field("c"), None);
    }

    /// Content types whose body `express.json()` (Express 4.21.2) parses.
    const SERVED_CONTENT_TYPES: [&[u8]; 12] = [
        b"application/json",
        b"application/json; charset=utf-8",
        b"application/json;charset=UTF-8",
        b"Application/JSON",
        b"application/json; charset=\"utf-8\"",
        b"application/json ; charset = utf-8 ",
        b"\tapplication/json\t",
        b"application/json; charset=\"\"",
        b"application/json; charset=\"utf\\-8\"",
        b"application/json; charset=latin1; charset=utf-8",
        b"application/json; foo=bar",
        b"application/json; foo=\"a;b \\\"c\\\" \xe9\"",
    ];

    /// Content types whose body `express.json()` skips, so the TS handler answers 400 WARN_01
    /// (`MediaType`, `ContentTypeParameter`), or refuses with 415 or, for UTF-16, decodes
    /// (`Charset`).
    const REFUSED_CONTENT_TYPES: [(Option<&[u8]>, BodyError); 15] = [
        (None, BodyError::MediaType),
        (Some(b"text/plain"), BodyError::MediaType),
        (Some(b"application/vnd.api+json"), BodyError::MediaType),
        (
            Some(b"application/json\t; charset=utf-8"),
            BodyError::MediaType,
        ),
        (Some(b"application/json;"), BodyError::ContentTypeParameter),
        (
            Some(b"application/json; foo"),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; charset=utf-8; foo"),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; charset="),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; charset=\"utf-8"),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; charset=utf-8 x"),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; =utf-8"),
            BodyError::ContentTypeParameter,
        ),
        (
            Some(b"application/json; charset=utf-16"),
            BodyError::Charset,
        ),
        (
            Some(b"application/json; charset=latin1"),
            BodyError::Charset,
        ),
        (Some(b"application/json; charset=utf8"), BodyError::Charset),
        (
            Some(b"application/json; charset=utf-8; charset=latin1"),
            BodyError::Charset,
        ),
    ];

    #[test]
    fn content_type_follows_express_json() {
        let body = br#"{"a":1}"#;
        for served in SERVED_CONTENT_TYPES {
            let parsed = CanonicalBody::parse(&headers(&[(CONTENT_TYPE, served)]), body)
                .unwrap_or_else(|e| panic!("{}: {e:?}", served.escape_ascii()));
            assert_eq!(parsed.as_str(), r#"{"a":1}"#, "{}", served.escape_ascii());
        }
        for (header, expected) in REFUSED_CONTENT_TYPES {
            let map = match header {
                Some(value) => headers(&[(CONTENT_TYPE, value)]),
                None => HeaderMap::new(),
            };
            assert_eq!(
                CanonicalBody::parse(&map, body).err(),
                Some(expected),
                "{:?}",
                header.map(|h| h.escape_ascii().to_string())
            );
        }
    }

    /// body-parser reads a body without an encoding as is, inflates gzip and deflate, and
    /// refuses other encodings with 415. The combiner reads only a body without an encoding.
    #[test]
    fn content_encoding_other_than_identity_is_refused() {
        let body = br#"{"a":1}"#;
        let with_encodings = |encodings: &[&str]| {
            let mut map = json_headers();
            for encoding in encodings {
                map.append(CONTENT_ENCODING, HeaderValue::from_str(encoding).unwrap());
            }
            CanonicalBody::parse(&map, body).map(|parsed| parsed.as_str().to_string())
        };
        let served: [&[&str]; 5] = [&[], &["identity"], &["IDENTITY"], &[""], &[" identity "]];
        for encodings in served {
            assert_eq!(
                with_encodings(encodings),
                Ok(r#"{"a":1}"#.to_string()),
                "{encodings:?}"
            );
        }
        let refused: [&[&str]; 5] = [
            &["gzip"],
            &["deflate"],
            &["br"],
            &["identity, gzip"],
            &["identity", "identity"],
        ];
        for encodings in refused {
            assert_eq!(
                with_encodings(encodings),
                Err(BodyError::ContentEncoding),
                "{encodings:?}"
            );
        }
    }

    #[test]
    fn length_limit_counts_utf16_units_of_the_canonical_body() {
        // `{"a":"` and `"}` are 8 units around the string.
        let with_filler = |units: usize| format!(r#"{{"a":"{}"}}"#, "x".repeat(units - 8));
        let parse = |s: &str| CanonicalBody::parse(&json_headers(), s.as_bytes());

        let at_limit = parse(&with_filler(16_000)).unwrap();
        assert_eq!(at_limit.as_str().encode_utf16().count(), 16_000);
        assert_eq!(
            parse(&with_filler(16_001)).err(),
            Some(BodyError::TooLong(16_001))
        );

        // Whitespace is not in the canonical body, so it does not count.
        let padded = format!("{}{}", with_filler(16_000), " ".repeat(500));
        assert_eq!(parse(&padded).unwrap().as_str().len(), 16_000);

        // An emoji is two units.
        let emoji = format!(r#"{{"a":"{}"}}"#, "\u{1F600}".repeat(7_996));
        assert_eq!(
            parse(&emoji).unwrap().as_str().encode_utf16().count(),
            16_000
        );
        let emoji = format!(r#"{{"a":"{}x"}}"#, "\u{1F600}".repeat(7_996));
        assert_eq!(parse(&emoji).err(), Some(BodyError::TooLong(16_001)));
    }
}
