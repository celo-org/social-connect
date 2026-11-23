//! The request body as the TS combiner sees it: parsed, then written back as
//! `JSON.stringify` writes it. The signature check, the length limit and the signer calls all
//! use that canonical text, so a body the writer cannot reproduce exactly as JavaScript would
//! is refused.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use axum::http::HeaderValue;
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
    #[error("the charset is not UTF-8")]
    Charset,
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
    /// Checks the `Content-Type` as `express.json()` does, then parses `raw` and writes it back
    /// in canonical form. The media type and charset are checked before the body is read.
    pub fn parse(content_type: Option<&HeaderValue>, raw: &[u8]) -> Result<Self, BodyError> {
        check_content_type(content_type)?;

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

/// Accepts what Express's JSON parser accepts: `application/json` in any case, with an optional
/// `charset` of `utf-8` (quoted or not, any case). Other parameters are ignored.
fn check_content_type(content_type: Option<&HeaderValue>) -> Result<(), BodyError> {
    let value = content_type
        .and_then(|v| v.to_str().ok())
        .ok_or(BodyError::MediaType)?;
    let mut parts = value.split(';');
    let media_type = parts.next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("application/json") {
        return Err(BodyError::MediaType);
    }
    for parameter in parts {
        let Some((name, parameter_value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            let charset = parameter_value.trim().trim_matches('"');
            if !charset.eq_ignore_ascii_case("utf-8") {
                return Err(BodyError::Charset);
            }
        }
    }
    Ok(())
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
    use axum::http::HeaderValue;

    fn json_type() -> HeaderValue {
        HeaderValue::from_static("application/json")
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
            let parsed = CanonicalBody::parse(Some(&json_type()), &body_of(&vector))
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
            let error = CanonicalBody::parse(Some(&json_type()), &body_of(&vector))
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
        let parse = |s: &str| CanonicalBody::parse(Some(&json_type()), s.as_bytes()).err();
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
        let body = CanonicalBody::parse(Some(&json_type()), br#" { "b" : [1, null], "a" : "x" } "#)
            .unwrap();
        assert_eq!(body.as_str(), r#"{"b":[1,null],"a":"x"}"#);
        assert_eq!(body.field("a"), Some(&JsValue::String("x".into())));
        assert_eq!(
            body.field("b"),
            Some(&JsValue::Array(vec![JsValue::Integer(1), JsValue::Null]))
        );
        assert_eq!(body.field("c"), None);
    }

    #[test]
    fn content_type_follows_express_json() {
        let body = br#"{"a":1}"#;
        for accepted in [
            "application/json",
            "application/json; charset=utf-8",
            "application/json;charset=UTF-8",
            "Application/JSON",
            "application/json; charset=\"utf-8\"",
        ] {
            let value = HeaderValue::from_str(accepted).unwrap();
            let parsed = CanonicalBody::parse(Some(&value), body)
                .unwrap_or_else(|e| panic!("{accepted}: {e:?}"));
            assert_eq!(parsed.as_str(), r#"{"a":1}"#, "{accepted}");
        }
        for (header, expected) in [
            (None, BodyError::MediaType),
            (Some("text/plain"), BodyError::MediaType),
            (Some("application/vnd.api+json"), BodyError::MediaType),
            (Some("application/json; charset=utf-16"), BodyError::Charset),
            (Some("application/json; charset=latin1"), BodyError::Charset),
            (Some("application/json; charset=utf8"), BodyError::Charset),
        ] {
            let value = header.map(|h| HeaderValue::from_str(h).unwrap());
            assert_eq!(
                CanonicalBody::parse(value.as_ref(), body).err(),
                Some(expected),
                "{header:?}"
            );
        }
    }

    #[test]
    fn length_limit_counts_utf16_units_of_the_canonical_body() {
        // `{"a":"` and `"}` are 8 units around the string.
        let with_filler = |units: usize| format!(r#"{{"a":"{}"}}"#, "x".repeat(units - 8));
        let parse = |s: &str| CanonicalBody::parse(Some(&json_type()), s.as_bytes());

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
