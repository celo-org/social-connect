//! The combiner's metric names, label names, `app` label and histogram buckets, checked against
//! the TS combiner's (`apps/combiner/src/common/metrics.ts`). This binary installs the
//! combiner's own recorder, so it builds no Rust signers, which install the signer's.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::time::Duration;

use alloy::primitives::Bytes;
use alloy::sol_types::SolCall;
use axum::http::StatusCode;
use axum::{Json, Router};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use odis_combiner::config::{Config, SignerEntry};
use odis_combiner::errors::{FAILURE_TO_GET_DEK, FULL_NODE_ERROR};
use odis_combiner::metrics::install_recorder;
use odis_combiner::server::build_router;
use odis_combiner::signature::KeyVersion;
use odis_signer::config::LogFormat;
use serde_json::{Value, json};

use common::*;

alloy::sol! {
    interface IAccounts {
        function getDataEncryptionKey(address account) external view returns (bytes);
    }
}

const SERVICE_NAME: &str = "odis-combiner";
const CHAIN_ID: u64 = 11142220;

/// A full node that answers `eth_chainId`, and answers `getDataEncryptionKey` with
/// `DEK_PUBLIC_KEY` for `ACCOUNT` and with an error for any other account.
async fn spawn_full_node() -> String {
    let app = Router::new().fallback(|Json(request): Json<Value>| async move {
        let id = request["id"].clone();
        let result = match request["method"].as_str().unwrap() {
            "eth_chainId" => format!("0x{CHAIN_ID:x}"),
            "eth_call" => {
                let call = &request["params"][0];
                let input = call
                    .get("input")
                    .or_else(|| call.get("data"))
                    .and_then(Value::as_str)
                    .unwrap();
                // The account is the call's last argument.
                if !input.to_lowercase().ends_with(&ACCOUNT[2..]) {
                    return Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32000, "message": "execution reverted"},
                    }));
                }
                let dek = hex::decode(&DEK_PUBLIC_KEY[2..]).unwrap();
                let returns =
                    IAccounts::getDataEncryptionKeyCall::abi_encode_returns(&Bytes::from(dek));
                format!("0x{}", hex::encode(returns))
            }
            other => panic!("unexpected method {other}"),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    });
    spawn_test_server(app).await
}

/// A label name and, when the test checks it, its value.
type Label<'a> = (&'a str, Option<&'a str>);

/// One exposition line: a name, its labels whatever their order, and the value.
#[derive(Debug)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    value: f64,
}

impl Sample {
    fn label_names(&self) -> BTreeSet<&str> {
        self.labels.keys().map(String::as_str).collect()
    }

    fn label(&self, name: &str) -> Option<&str> {
        self.labels.get(name).map(String::as_str)
    }
}

fn parse(text: &str) -> Vec<Sample> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Sample {
    let (series, value) = line.rsplit_once(' ').unwrap();
    let (name, labels) = match series.split_once('{') {
        Some((name, labels)) => (name, parse_labels(labels.strip_suffix('}').unwrap())),
        None => (series, BTreeMap::new()),
    };
    Sample {
        name: name.to_string(),
        labels,
        value: value.parse().unwrap(),
    }
}

/// `key="value",…` with the exposition format's escapes in the values.
fn parse_labels(mut rest: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    while !rest.is_empty() {
        let (key, after) = rest.split_once("=\"").unwrap();
        let mut value = String::new();
        let mut chars = after.char_indices();
        let end = loop {
            match chars.next().unwrap() {
                (_, '\\') => match chars.next().unwrap().1 {
                    'n' => value.push('\n'),
                    escaped => value.push(escaped),
                },
                (i, '"') => break i,
                (_, c) => value.push(c),
            }
        };
        labels.insert(key.to_string(), value);
        rest = &after[end + 1..];
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
    labels
}

#[tokio::test]
async fn metrics_expose_the_ts_names_with_snake_case_labels() -> Result<(), Box<dyn Error>> {
    let signer1 = partial_signer(0, Duration::ZERO).await.0;
    let signer2 = partial_signer(1, Duration::ZERO).await.0;
    let signer3 = fake_signer(
        StatusCode::INTERNAL_SERVER_ERROR,
        r#"{"success":false,"version":"1.0.0","error":"CELO_ODIS_ERR_00 Something went wrong"}"#,
        Duration::ZERO,
    )
    .await
    .0;
    let key = &dev_keys()[0];
    let config = Config {
        server_port: 0,
        service_name: SERVICE_NAME.to_string(),
        signers: [signer1, signer2, signer3]
            .into_iter()
            .enumerate()
            .map(|(index, url)| SignerEntry {
                url,
                share_index: Some(index as u32),
            })
            .collect(),
        signer_timeout: Duration::from_secs(5),
        key_versions: vec![KeyVersion::decode(
            key.key_version,
            key.threshold,
            &key.polynomial,
            &key.pub_key,
        )?],
        current_key_version: key.key_version,
        blockchain_provider: spawn_full_node().await,
        blockchain_api_key: None,
        chain_id: CHAIN_ID,
        full_node_timeout: Duration::from_secs(1),
        full_node_retry_count: 0,
        full_node_retry_delay: Duration::from_millis(1),
        log_format: LogFormat::Text,
    };
    let router = build_router(config, install_recorder(SERVICE_NAME)?).await?;
    let url = spawn_test_server(router).await;

    let query = BASE64.encode(vector_message(0));
    let wallet_body = format!(r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}"}}"#);
    let dek_body = format!(
        r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}","authenticationMethod":"encryption_key"}}"#
    );
    let failing_dek_body = format!(
        r#"{{"account":"{}","blindedQueryPhoneNumber":"{query}","authenticationMethod":"encryption_key"}}"#,
        account2()
    );
    let quota_body = format!(r#"{{"account":"{ACCOUNT}"}}"#);
    let requests = [
        (
            "/sign",
            wallet_body.clone(),
            wallet_auth(&wallet_body).await,
        ),
        ("/sign", dek_body.clone(), dek_auth(&dek_body)),
        (
            "/sign",
            failing_dek_body.clone(),
            dek_auth(&failing_dek_body),
        ),
        (
            "/quotaStatus",
            quota_body.clone(),
            wallet_auth(&quota_body).await,
        ),
        ("/sign", "{}".to_string(), wallet_auth("{}").await),
        ("/domain/sign", "{}".to_string(), String::new()),
    ];
    let mut statuses = Vec::new();
    for (path, body, authorization) in requests {
        let (status, _, answer) =
            post(&url, path, body, &[("authorization", &authorization)]).await;
        statuses.push((path, status.as_u16(), answer));
    }
    let client = reqwest::Client::builder().no_proxy().build()?;
    let get = client.get(format!("{url}/sign")).send().await?;
    statuses.push(("GET /sign", get.status().as_u16(), get.text().await?));
    let codes: Vec<u16> = statuses.iter().map(|(_, status, _)| *status).collect();
    assert_eq!(codes, [200, 200, 401, 200, 400, 503, 405], "{statuses:#?}");

    let text = client
        .get(format!("{url}/metrics"))
        .send()
        .await?
        .text()
        .await?;
    let samples = parse(&text);

    for sample in &samples {
        assert_eq!(sample.label("app"), Some(SERVICE_NAME), "{sample:?}");
        for camel_case in ["statusCode", "codeSegment"] {
            assert!(sample.label(camel_case).is_none(), "{sample:?}");
        }
        assert!(
            !sample.name.starts_with("combiner_event_loop_lag"),
            "{sample:?}"
        );
    }

    // Each name with its labels besides `app` and, for buckets, `le`. A value is checked when
    // one is given.
    let expected: &[(&str, &[Label])] = &[
        ("combiner_requests_total", &[("endpoint", None)]),
        (
            "combiner_responses_total",
            &[("endpoint", None), ("status_code", None)],
        ),
        ("combiner_endpoint_latency_bucket", &[("endpoint", None)]),
        (
            "combiner_signer_latency_bucket",
            &[("endpoint", None), ("signer", None)],
        ),
        (
            "combiner_signature_aggregation_latency_bucket",
            &[("endpoint", Some("/sign"))],
        ),
        ("signer_tail_latency_bucket", &[("endpoint", Some("/sign"))]),
        (
            "combiner_full_node_latency_bucket",
            &[("code_segment", Some("getDataEncryptionKey"))],
        ),
        (
            "combiner_sig_response_total",
            &[("status", None), ("signer", None), ("endpoint", None)],
        ),
        (
            "combiner_sig_response_errors_total",
            &[
                ("status", Some("500")),
                ("signer", None),
                ("endpoint", None),
            ],
        ),
        (
            "combiner_errors_total",
            &[
                ("endpoint", Some("NA")),
                ("error_type", Some(FULL_NODE_ERROR)),
            ],
        ),
        (
            "combiner_blockchain_errors_total",
            &[
                ("endpoint", Some("NA")),
                ("error_type", Some(FAILURE_TO_GET_DEK)),
            ],
        ),
        (
            "combiner_warnings_total",
            &[("endpoint", None), ("warning_type", None)],
        ),
        ("combiner_connection_closed_total", &[]),
    ];
    for (name, labels) in expected {
        let mut names: BTreeSet<&str> = labels.iter().map(|(label, _)| *label).collect();
        names.insert("app");
        if name.ends_with("_bucket") {
            names.insert("le");
        }
        let found = samples.iter().any(|sample| {
            sample.name == *name
                && sample.label_names() == names
                && labels
                    .iter()
                    .all(|(label, value)| value.is_none_or(|v| sample.label(label) == Some(v)))
        });
        assert!(found, "no {name} with labels {names:?} in\n{text}");
    }

    let value = |name: &str, labels: &[(&str, &str)]| -> Vec<f64> {
        samples
            .iter()
            .filter(|s| s.name == name && labels.iter().all(|(l, v)| s.label(l) == Some(v)))
            .map(|s| s.value)
            .collect()
    };
    assert_eq!(value("combiner_connection_closed_total", &[]), [0.0]);
    // Four POSTs and the GET.
    assert_eq!(
        value("combiner_requests_total", &[("endpoint", "/sign")]),
        [5.0]
    );
    assert_eq!(
        value(
            "combiner_responses_total",
            &[("endpoint", "/sign"), ("status_code", "405")]
        ),
        [1.0]
    );

    let bounds: Vec<&str> = samples
        .iter()
        .filter(|s| {
            s.name == "combiner_endpoint_latency_bucket" && s.label("endpoint") == Some("/sign")
        })
        .map(|s| s.label("le").unwrap())
        .collect();
    assert_eq!(
        bounds,
        ["0.001", "0.01", "0.1", "0.5", "1", "2", "5", "10", "+Inf"]
    );
    Ok(())
}
