//! What the differential test runs, and how it sends a request and compares the answers.
//!
//! The TS combiner runs from its released image and the Rust combiner in-process, each in front
//! of three in-process Rust signers of its own. Each combiner has its own signers because a
//! request sent through both combiners to shared signers would reach every signer twice, and the
//! second time it is a replay. A stub full node answers both combiners' chain ID checks and DEK
//! lookups.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256, address, keccak256};
use alloy::sol;
use alloy::sol_types::SolCall;
use alloy_signer_local::PrivateKeySigner;
use async_trait::async_trait;
use axum::{Json, Router};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use metrics_exporter_prometheus::PrometheusHandle;
use odis_combiner::config::{Config, SignerEntry};
use odis_combiner::server::build_router;
use odis_combiner::signature::KeyVersion;
use odis_signer::account_service::{AccountService, PnpAccount};
use odis_signer::config::LogFormat;
use odis_signer::errors::OdisError;
use serde::Serialize;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::{Value, json};
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::net::TcpStream;

use crate::common::{
    DEK_PUBLIC_KEY, RunningSigner, SignerSetup, dev_keys, spawn_signer_with, spawn_test_server,
};
use crate::comparisons::{Expected, PinnedAnswer};

/// Celo Sepolia's chain ID, which both combiners and the stub full node run on.
const CHAIN_ID: u64 = 11142220;
/// The Accounts contract on Celo Sepolia, where both combiners look up DEKs.
const ACCOUNTS: Address = address!("0x44957232699ca060B607E77083bDACD350d6b6d1");

/// The TS combiner of the release `odis-combiner-4.0.5`, pinned by digest. testcontainers names
/// an image `<name>:<tag>`, so the name ends in `@sha256` and the tag holds the digest's hex.
const TS_COMBINER_IMAGE: &str =
    "us-west1-docker.pkg.dev/devopsre/social-connect/odis-combiner@sha256";
const TS_COMBINER_DIGEST: &str = "50528e8ff6e8ae8cc77b5ee7762693da01d5f97735ab63f52eb9175d39eeeda4";
/// The port the TS combiner listens on inside its container.
const TS_COMBINER_PORT: u16 = 8081;
/// How long the TS combiner may take to answer `/status`. The image is built for amd64 only, and
/// on another architecture it runs under emulation, which starts it slowly.
const TS_COMBINER_START: Duration = Duration::from_secs(120);
/// The name under which the container reaches a port of the host that the test exposes to it.
const HOST_ALIAS: &str = "host.testcontainers.internal";
/// Set in CI, where a missing Docker must fail the test rather than skip it.
const REQUIRE_FLAG: &str = "ODIS_REQUIRE_DIFFERENTIAL_TESTS";
/// How long one request may take, from connecting to the last byte of the answer. A combiner
/// that hangs fails that comparison instead of stalling the test before it reports its failures
/// and the TS combiner's log.
const REQUEST_TIME_LIMIT: Duration = Duration::from_secs(30);

/// The headers an answer is compared on.
const COMPARED_HEADERS: [&str; 4] = [
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "allow",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub status: u16,
    /// A JSON body as `mask_version` writes it, with its `version` replaced by `"<version>"`,
    /// since each combiner reports its own. Any other body as text.
    pub body: String,
    /// The headers of `COMPARED_HEADERS` that are present, and the media type of `content-type`.
    pub headers: BTreeMap<String, String>,
}

pub struct RawRequest {
    pub method: &'static str,
    pub path: String,
    /// Sent in order; a name that appears twice is sent twice.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct TestUser {
    pub private_key: B256,
    /// In lower case, as the SDK's tests write accounts.
    pub account: String,
}

impl TestUser {
    fn address(&self) -> Address {
        self.account.parse().unwrap()
    }
}

/// A wallet key and account of its own for every label.
pub fn test_user(label: &str) -> TestUser {
    let private_key = keccak256(format!("odis differential test: {label}"));
    let address = PrivateKeySigner::from_bytes(&private_key)
        .unwrap()
        .address();
    TestUser {
        private_key,
        account: format!("0x{}", hex::encode(address)),
    }
}

/// The one account without a DEK; every other account has `DEK_PUBLIC_KEY`.
pub static USER_WITHOUT_DEK: LazyLock<TestUser> = LazyLock::new(|| test_user("without a DEK"));
/// The one account that has paid for no queries; every other account has paid for 1000.
pub static USER_WITHOUT_QUOTA: LazyLock<TestUser> = LazyLock::new(|| test_user("without quota"));

// The chain state every lookup reads: the signers through `DifferentialAccounts` and both
// combiners through the stub full node. Both answer from these two functions, so they cannot
// differ.

/// The DEK registered for `account`, with `0x`, or `None`.
fn registered_dek(account: Address) -> Option<&'static str> {
    (account != USER_WITHOUT_DEK.address()).then_some(DEK_PUBLIC_KEY)
}

/// The queries `account` has paid for.
fn total_quota(account: Address) -> u32 {
    if account == USER_WITHOUT_QUOTA.address() {
        0
    } else {
        1000
    }
}

/// The signers' account lookup.
struct DifferentialAccounts;

#[async_trait]
impl AccountService for DifferentialAccounts {
    async fn get_account(&self, address: Address) -> Result<PnpAccount, OdisError> {
        Ok(PnpAccount {
            address,
            // An account without a DEK has an empty one, as `MockAccountService` writes it.
            dek: registered_dek(address).unwrap_or_default().to_string(),
            pnp_total_quota: total_quota(address),
        })
    }
}

/// Three signers with the dev shares 1 to 3 of key versions 1 to 3, each with its own database.
pub async fn differential_signers() -> Vec<RunningSigner> {
    let mut signers = Vec::new();
    for share in 0..3 {
        signers.push(
            spawn_signer_with(SignerSetup::share(share), Arc::new(DifferentialAccounts)).await,
        );
    }
    signers
}

/// The URL of the Rust combiner in front of signers of its own, and those signers. It is built
/// as the binary builds it, with its chain ID check and its DEK lookup against `full_node`.
/// `metrics` is the in-process signers' recorder, which the combiner renders.
pub async fn rust_fleet(
    full_node: &str,
    metrics: PrometheusHandle,
) -> (String, Vec<RunningSigner>) {
    let signers = differential_signers().await;
    let config = Config {
        server_port: 0,
        service_name: "combiner-differential".to_string(),
        signers: signers
            .iter()
            .enumerate()
            .map(|(index, signer)| SignerEntry {
                url: signer.url.clone(),
                share_index: Some(index as u32),
            })
            .collect(),
        signer_timeout: Duration::from_secs(5),
        key_versions: dev_keys()
            .iter()
            .map(|key| {
                KeyVersion::decode(
                    key.key_version,
                    key.threshold,
                    &key.polynomial,
                    &key.pub_key,
                )
                .unwrap()
            })
            .collect(),
        current_key_version: 1,
        blockchain_provider: full_node.to_string(),
        blockchain_api_key: None,
        chain_id: CHAIN_ID,
        full_node_timeout: Duration::from_secs(1),
        full_node_retry_count: 0,
        full_node_retry_delay: Duration::from_millis(1),
        log_format: LogFormat::Text,
    };
    let router = build_router(config, metrics).await.unwrap();
    (spawn_test_server(router).await, signers)
}

sol! {
    interface IAccounts {
        function getDataEncryptionKey(address account) external view returns (bytes);
    }
}

/// A JSON-RPC server that answers `eth_chainId` with Celo Sepolia's chain ID and the Accounts
/// contract's `getDataEncryptionKey` from `registered_dek`. Anything else gets a JSON-RPC error.
pub async fn stub_full_node() -> String {
    let router = Router::new().fallback(|Json(call): Json<Value>| async move {
        let method = call["method"].as_str().unwrap_or_default();
        let answer = match full_node_result(&call) {
            Some(result) => json!({ "jsonrpc": "2.0", "id": call["id"], "result": result }),
            None => json!({
                "jsonrpc": "2.0",
                "id": call["id"],
                "error": {
                    "code": -32601,
                    "message": format!("the stub full node does not answer {method}"),
                },
            }),
        };
        Json(answer)
    });
    spawn_test_server(router).await
}

/// The result of a JSON-RPC call the stub full node answers, or `None`.
fn full_node_result(call: &Value) -> Option<String> {
    match call["method"].as_str()? {
        "eth_chainId" => Some(format!("{CHAIN_ID:#x}")),
        "eth_call" => {
            let transaction = &call["params"][0];
            let to: Address = transaction["to"].as_str()?.parse().ok()?;
            if to != ACCOUNTS {
                return None;
            }
            let input = transaction["input"]
                .as_str()
                .or(transaction["data"].as_str())?;
            let input = hex::decode(input.strip_prefix("0x")?).ok()?;
            let lookup = IAccounts::getDataEncryptionKeyCall::abi_decode(&input).ok()?;
            let dek = registered_dek(lookup.account)
                .map(|dek| hex::decode(&dek[2..]).unwrap())
                .unwrap_or_default();
            let encoded = IAccounts::getDataEncryptionKeyCall::abi_encode_returns(&dek.into());
            Some(format!("0x{}", hex::encode(encoded)))
        }
        _ => None,
    }
}

/// A key version as the TS combiner reads it from `PNP_KEYS_VERSIONS`: `KeyVersionInfo` in
/// `odis-combiner-4.0.5:packages/common/src/utils/key-version.ts`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TsKeyVersion<'a> {
    key_version: u32,
    threshold: usize,
    polynomial: &'a str,
    pub_key: &'a str,
}

/// The port of a URL `http://<host>:<port>`.
fn port_of(url: &str) -> u16 {
    url.rsplit(':').next().unwrap().parse().unwrap()
}

/// The URL under which the TS combiner's container reaches the test's server at `url`.
fn in_container(url: &str) -> String {
    format!("http://{HOST_ALIAS}:{}", port_of(url))
}

/// Starts the TS combiner in front of `signers`, with `full_node` for its DEK lookups, and waits
/// until it answers `/status`. Returns `None` when Docker is unavailable, unless
/// `ODIS_REQUIRE_DIFFERENTIAL_TESTS` is set: then a missing Docker fails the test.
pub async fn ts_combiner(
    signers: &[String],
    full_node: &str,
) -> Option<ContainerAsync<GenericImage>> {
    let exposed_ports: Vec<u16> = signers
        .iter()
        .map(String::as_str)
        .chain([full_node])
        .map(port_of)
        .collect();
    let signer_entries: Vec<Value> = signers
        .iter()
        .map(|url| json!({ "url": in_container(url) }))
        .collect();
    let keys = dev_keys();
    let key_versions: Vec<TsKeyVersion> = keys
        .iter()
        .map(|key| TsKeyVersion {
            key_version: key.key_version,
            threshold: key.threshold,
            polynomial: &key.polynomial,
            pub_key: &key.pub_key,
        })
        .collect();

    // The image's entrypoint is `yarn start:docker`, which runs `node dist/index.js`. Running
    // node directly leaves Yarn out of the process tree. The image is built for amd64 only.
    let started = GenericImage::new(TS_COMBINER_IMAGE, TS_COMBINER_DIGEST)
        .with_entrypoint("node")
        .with_exposed_port(TS_COMBINER_PORT.tcp())
        .with_cmd(["dist/index.js"])
        .with_platform("linux/amd64")
        .with_exposed_host_ports(exposed_ports)
        .with_env_var("PHONE_NUMBER_PRIVACY_API_ENABLED", "true")
        .with_env_var(
            "PNP_ODIS_SERVICES_SIGNERS",
            serde_json::to_string(&signer_entries).unwrap(),
        )
        .with_env_var(
            "PNP_KEYS_VERSIONS",
            serde_json::to_string(&key_versions).unwrap(),
        )
        .with_env_var("PNP_KEYS_CURRENT_VERSION", "1")
        // The TS combiner parses the domain signer list at start even with the domain API off,
        // and stops when it is not JSON.
        .with_env_var("DOMAIN_ODIS_SERVICES_SIGNERS", "[]")
        .with_env_var("BLOCKCHAIN_PROVIDER", in_container(full_node))
        .with_env_var("CHAIN_ID", CHAIN_ID.to_string())
        .with_env_var("SERVER_PORT", TS_COMBINER_PORT.to_string())
        // A passing run needs none of the TS combiner's log; fatal is bunyan's highest level.
        .with_env_var("LOG_LEVEL", "fatal")
        .start()
        .await;
    let container = match started {
        Ok(container) => container,
        Err(e) => {
            if std::env::var_os(REQUIRE_FLAG).is_some() {
                panic!("differential tests are required but Docker is unavailable: {e}");
            }
            eprintln!("skipping the differential test (Docker unavailable): {e}");
            return None;
        }
    };
    wait_for_status(&container).await;
    Some(container)
}

/// The URL under which the test reaches the TS combiner.
pub async fn ts_combiner_url(container: &ContainerAsync<GenericImage>) -> String {
    let port = container
        .get_host_port_ipv4(TS_COMBINER_PORT.tcp())
        .await
        .expect("the TS combiner's port is mapped");
    format!("http://127.0.0.1:{port}")
}

/// Waits until the TS combiner answers `/status` with 200. Fails with the container's log when it
/// stops, or when it does not answer within `TS_COMBINER_START`.
async fn wait_for_status(container: &ContainerAsync<GenericImage>) {
    let url = format!("{}/status", ts_combiner_url(container).await);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let deadline = Instant::now() + TS_COMBINER_START;
    while Instant::now() < deadline {
        if let Ok(response) = client.get(&url).send().await
            && response.status() == 200
        {
            return;
        }
        if !container.is_running().await.unwrap_or(false) {
            panic!(
                "the TS combiner stopped before it answered /status:\n{}",
                container_logs(container).await
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "the TS combiner did not answer /status within {TS_COMBINER_START:?}:\n{}",
        container_logs(container).await
    );
}

/// What the container has written to stdout and stderr so far.
pub async fn container_logs(container: &ContainerAsync<GenericImage>) -> String {
    let stdout = container.stdout_to_vec().await.unwrap_or_default();
    let stderr = container.stderr_to_vec().await.unwrap_or_default();
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    )
}

/// The answer to `request` from the combiner at `base`, or why there is none: it could not be
/// sent, the connection broke, or no answer came within `REQUEST_TIME_LIMIT`.
pub async fn send(base: &str, request: &RawRequest) -> Result<Answer, String> {
    tokio::time::timeout(REQUEST_TIME_LIMIT, exchange(base, request))
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "{} {} to {base} got no answer within {REQUEST_TIME_LIMIT:?}",
                request.method, request.path
            ))
        })
}

/// Sends the request's exact bytes on a connection of its own, as Node's `http.request` does
/// without an agent: the request's headers in order, then `Host`, `Connection: close` and, for a
/// POST that names no transfer encoding, `Content-Length`.
async fn exchange(base: &str, request: &RawRequest) -> Result<Answer, String> {
    let authority = base.strip_prefix("http://").unwrap();
    let stream = TcpStream::connect(authority)
        .await
        .map_err(|e| format!("cannot connect to {base}: {e}"))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| format!("no HTTP connection to {base}: {e}"))?;
    // A server may answer before it has read the whole body (413) and close the connection while
    // the body is still being written; that error comes after the answer.
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut builder = hyper::Request::builder()
        .method(request.method)
        .uri(request.path.as_str());
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder = builder
        .header("host", authority)
        .header("connection", "close");
    let names_encoding = request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("transfer-encoding"));
    if request.method == "POST" && !names_encoding {
        builder = builder.header("content-length", request.body.len());
    }
    let built = builder
        .body(Full::new(Bytes::from(request.body.clone())))
        .map_err(|e| format!("{} {} cannot be sent: {e}", request.method, request.path))?;
    let response = sender.send_request(built).await.map_err(|e| {
        format!(
            "{} {} to {base} got no answer: {e}",
            request.method, request.path
        )
    })?;

    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for name in COMPARED_HEADERS {
        // Node joins the values of a repeated header with ", ".
        let values: Vec<String> = response
            .headers()
            .get_all(name)
            .iter()
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
            .collect();
        if !values.is_empty() {
            headers.insert(name.to_string(), values.join(", "));
        }
    }
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type = String::from_utf8_lossy(content_type.as_bytes());
        let media_type = content_type.split(';').next().unwrap().trim();
        headers.insert("content-type".to_string(), media_type.to_string());
    }
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| {
            format!(
                "the answer to {} {} from {base} broke off: {e}",
                request.method, request.path
            )
        })?
        .to_bytes();
    Ok(Answer {
        status,
        body: mask_version(&String::from_utf8_lossy(&body)),
        headers,
    })
}

/// The members of a JSON object in the order the text has them, as `JSON.parse` keeps them.
struct Members(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MembersVisitor;

        impl<'de> Visitor<'de> for MembersVisitor {
            type Value = Members;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }

        deserializer.deserialize_map(MembersVisitor)
    }
}

/// `text` with the members of its top-level object in their order, written compactly, and the
/// `version` member's value replaced by `"<version>"`. Each value is written as serde_json writes
/// a `Value`, which sorts the keys of nested objects. Other JSON is written as serde_json writes
/// it; `text` itself when it is not JSON.
fn mask_version(text: &str) -> String {
    if let Ok(Members(members)) = serde_json::from_str(text) {
        let written: Vec<String> = members
            .into_iter()
            .map(|(key, value)| {
                let value = if key == "version" {
                    Value::from("<version>")
                } else {
                    value
                };
                format!("{}:{value}", Value::from(key))
            })
            .collect();
        return format!("{{{}}}", written.join(","));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => value.to_string(),
        Err(_) => text.to_string(),
    }
}

/// The `error` of a JSON answer.
fn error_of(answer: &Answer) -> Option<String> {
    let body: Value = serde_json::from_str(&answer.body).ok()?;
    body.get("error")?.as_str().map(str::to_string)
}

/// The fields of `answer` that `expected` names.
fn only(answer: &Answer, expected: &PinnedAnswer) -> PinnedAnswer {
    PinnedAnswer {
        status: expected.status.map(|_| answer.status),
        body: expected.body.as_ref().map(|_| answer.body.clone()),
        headers: expected.headers.as_ref().map(|_| answer.headers.clone()),
    }
}

/// How the answers to a comparison differ from what it expects, or `None` when they do not.
pub fn difference(expect: &Expected, ts: &Answer, rust: &Answer) -> Option<String> {
    match expect {
        Expected::SameAnswer { status, error } => {
            if rust != ts {
                return Some(format!("the combiners differ: TS {ts:?}, Rust {rust:?}"));
            }
            // A comparison cannot pass by both sides failing alike or refusing for the same wrong
            // reason.
            let answered = (ts.status, error_of(ts));
            let expected = (*status, error.map(str::to_string));
            (answered != expected)
                .then(|| format!("both answer {answered:?}, the comparison expects {expected:?}"))
        }
        Expected::KnownDifference {
            ts: expected_ts,
            rust: expected_rust,
        } => {
            let wrong: Vec<String> = [("TS", ts, expected_ts), ("Rust", rust, expected_rust)]
                .into_iter()
                .filter_map(|(side, answer, expected)| {
                    let picked = only(answer, expected);
                    (picked != *expected).then(|| {
                        format!("{side} answers {picked:?}, the comparison expects {expected:?}")
                    })
                })
                .collect();
            (!wrong.is_empty()).then(|| wrong.join("; "))
        }
    }
}
