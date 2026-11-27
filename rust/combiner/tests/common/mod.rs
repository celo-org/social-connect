//! The integration harness: real `odis-signer` routers on real ports, each with its own key
//! shares and its own SQLite file, and the combiner in front of them on a port of its own.
//! Signers that misbehave on purpose are small axum servers.

// Each test binary uses a part of these helpers.
#![allow(dead_code)]

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alloy::primitives::Address;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use async_trait::async_trait;
use axum::Router;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature, SigningKey};
use odis_combiner::config::{Config, SignerEntry};
use odis_combiner::dek::{DekLookup, DekLookupError};
use odis_combiner::server::build_router_with_services;
use odis_combiner::signature::KeyVersion;
use odis_signer::account_service::{AccountService, MockAccountService};
use odis_signer::config::{Config as SignerConfig, DatabaseConfig, KeystoreType, LogFormat};
use odis_signer::errors::OdisError;
use odis_signer::key_management::KeyProvider;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use threshold_bls::curve::bls12377::{G1, Scalar};
use threshold_bls::group::{Element, Scalar as _};
use tokio::net::TcpListener;

/// `PRIVATE_KEY1` of the TS test values
/// (`odis-combiner-4.0.5:packages/common/src/test/values.ts:8`).
pub const PRIVATE_KEY1: &str = "535029bfb19fe5440dbd549b88fbf5ee847b059485e4eafc2a3e3bdfbf9b31ac";
/// `ACCOUNT_ADDRESS1`, the address of `PRIVATE_KEY1` in lower case (`values.ts:9`).
pub const ACCOUNT: &str = "0x82c76935924be4dfbf58d4365071b02de267ba2e";
/// `PRIVATE_KEY2` (`values.ts:10`).
pub const PRIVATE_KEY2: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890fdeccc";
/// `DEK_PUBLIC_KEY` (`values.ts:21`).
pub const DEK_PUBLIC_KEY: &str =
    "0x034846bc781cacdafc66f3a77aa9fc3c56a9dadcd683c72be3c446fee8da041070";
/// `DEK_PRIVATE_KEY` (`values.ts:22`).
pub const DEK_PRIVATE_KEY: &str =
    "0xbf8a2b73baf8402f8fe906ad3f42b560bf14b39f7df7797ece9e293d6f162188";

/// The combiner's version, which every answer of its own carries.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const SESSION_ID: &str = "3e2d8c1a-0b5f-4f7e-9a62-1f0c5e7d9b44";
/// A client version, which the SDK sends in `version` when its caller names one.
pub const CLIENT_VERSION: &str = "5.1.0";
/// What a TS signer with its PNP API off answers.
pub const DISABLED_SIGNER_ANSWER: &str = r#"{"success":false,"version":"1.0.0","error":"CELO_ODIS_WARN_13 BAD_INPUT API is unavailable"}"#;
/// What a signer answers an account without quota.
pub const OUT_OF_QUOTA_ANSWER: &str = r#"{"success":false,"version":"1.0.0","error":"CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota"}"#;

/// One key version of `tests/data/combine_vectors.json`, with every share.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevKey {
    pub key_version: u32,
    pub threshold: usize,
    pub polynomial: String,
    pub pub_key: String,
    pub shares: Vec<String>,
}

#[derive(Deserialize)]
struct PartialVector {
    key: usize,
    share: usize,
    message: usize,
    partial: String,
    valid: bool,
}

#[derive(Deserialize)]
struct CombinedVector {
    key: usize,
    message: usize,
    signature: String,
}

#[derive(Deserialize)]
struct Vectors {
    keys: Vec<DevKey>,
    messages: Vec<String>,
    partials: Vec<PartialVector>,
    combined: Vec<CombinedVector>,
}

fn vectors() -> Vectors {
    serde_json::from_str(include_str!("../data/combine_vectors.json")).unwrap()
}

fn vector_keys() -> Vec<DevKey> {
    vectors().keys
}

/// The position of `key_version` in the vectors' key list.
fn key_index(vectors: &Vectors, key_version: u32) -> usize {
    vectors
        .keys
        .iter()
        .position(|key| key.key_version == key_version)
        .unwrap()
}

/// Blinded message `index` of the vectors. Message 0 is the TS suite's: `test message`
/// blinded with the seed 0, 1, …, 30, 0.
pub fn vector_message(index: usize) -> Vec<u8> {
    hex::decode(&vectors().messages[index]).unwrap()
}

/// The TS library's partial signature of share `share` (from 0) of `key_version` on vector
/// message `message`, in base64.
pub fn vector_partial(key_version: u32, share: usize, message: usize) -> String {
    let vectors = vectors();
    let key = key_index(&vectors, key_version);
    let partial = vectors
        .partials
        .iter()
        .find(|p| p.key == key && p.share == share && p.message == message && p.valid)
        .unwrap();
    BASE64.encode(hex::decode(&partial.partial).unwrap())
}

/// The TS library's combined signature of `key_version` on vector message `message`, in
/// base64. Every subset of shares gives the same signature.
pub fn vector_signature(key_version: u32, message: usize) -> String {
    let vectors = vectors();
    let key = key_index(&vectors, key_version);
    let combined = vectors
        .combined
        .iter()
        .find(|c| c.key == key && c.message == message)
        .unwrap();
    BASE64.encode(hex::decode(&combined.signature).unwrap())
}

/// The 2-of-3 dev keys, versions 1 to 3.
pub fn dev_keys() -> Vec<DevKey> {
    vector_keys()
        .into_iter()
        .filter(|key| (1..=3).contains(&key.key_version) && key.shares.len() == 3)
        .collect()
}

/// The 4-of-5 key.
pub fn four_of_five_key() -> DevKey {
    vector_keys()
        .into_iter()
        .find(|key| key.threshold == 4 && key.shares.len() == 5)
        .unwrap()
}

// -- Signers --

pub struct SignerSetup {
    /// Key version to share, hex.
    pub shares: HashMap<u32, String>,
    pub total_quota: u32,
    pub dek: Option<String>,
    /// How long the key store takes to hand out a share.
    pub key_delay: Duration,
}

impl SignerSetup {
    /// Share `index` (from 0) of the dev keys, versions 1 to 3, with quota 1000 and the DEK
    /// `DEK_PUBLIC_KEY`.
    pub fn share(index: usize) -> SignerSetup {
        SignerSetup {
            shares: dev_keys()
                .into_iter()
                .map(|key| (key.key_version, key.shares[index].clone()))
                .collect(),
            total_quota: 1000,
            dek: Some(DEK_PUBLIC_KEY.to_string()),
            key_delay: Duration::ZERO,
        }
    }
}

pub struct RunningSigner {
    pub url: String,
    /// The signer's SQLite database, deleted when the signer is dropped.
    _database: NamedTempFile,
}

/// A key store that serves the shares it was given under `phoneNumberPrivacy-<version>`.
struct TestKeys {
    shares: HashMap<String, String>,
    delay: Duration,
}

#[async_trait]
impl KeyProvider for TestKeys {
    async fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError> {
        tokio::time::sleep(self.delay).await;
        self.shares
            .get(&format!("{name}-{version}"))
            .cloned()
            .ok_or(OdisError::KeyFetchError)
    }
}

/// Starts a real signer on 127.0.0.1. It authenticates requests, because a full node is
/// configured, and takes its DEK and quota from `MockAccountService`.
pub async fn spawn_signer(setup: SignerSetup) -> RunningSigner {
    let accounts = Arc::new(MockAccountService::new(
        setup.dek.clone(),
        setup.total_quota,
    ));
    spawn_signer_with(setup, accounts).await
}

/// Starts a real signer on 127.0.0.1, as `spawn_signer` does, that takes the DEK and quota of
/// each account from `accounts` instead of from `setup`.
pub async fn spawn_signer_with(
    setup: SignerSetup,
    accounts: Arc<dyn AccountService>,
) -> RunningSigner {
    let database = NamedTempFile::new().unwrap();
    let config = SignerConfig {
        server_port: 0,
        keystore_type: KeystoreType::Mock,
        pnp_key_name_base: "phoneNumberPrivacy".to_string(),
        pnp_latest_key_version: setup.shares.keys().copied().max().unwrap_or(1),
        database: DatabaseConfig::Sqlite {
            path: database.path().to_str().unwrap().to_string(),
        },
        migrate_legacy_data: false,
        db_timeout_ms: 1000,
        blockchain_provider: Some("http://localhost:8545".to_string()),
        blockchain_api_key: None,
        chain_id: 11142220,
        accounts_contract_address: None,
        odis_payments_contract_address: None,
        full_node_retry_count: 0,
        full_node_retry_delay_ms: 100,
        full_node_timeout_ms: 1000,
        timeout_ms: 5000,
        query_price_per_cusd: 0.001,
        google_project_id: None,
        request_pruning_days: 7,
        request_pruning_interval_secs: 0,
        log_format: LogFormat::Text,
    };
    let keys = TestKeys {
        shares: setup
            .shares
            .into_iter()
            .map(|(version, share)| (format!("phoneNumberPrivacy-{version}"), share))
            .collect(),
        delay: setup.key_delay,
    };
    let router = odis_signer::server::build_router_with_services(config, accounts, Arc::new(keys))
        .await
        .unwrap();
    RunningSigner {
        url: spawn_test_server(router).await,
        _database: database,
    }
}

/// Serves `router` on a free port of 127.0.0.1 and returns its URL.
pub async fn spawn_test_server(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

/// The URL of a signer that is down: a server that accepts every connection and closes it at
/// once. It keeps its port, so no other test can take the port and answer in its place.
pub async fn down_signer() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            if let Ok((connection, _)) = listener.accept().await {
                drop(connection);
            }
        }
    });
    url
}

/// A signer that answers every request with `status`, `body` and, when given, the
/// `odis-key-version` header, after `delay`. It counts the requests it receives.
pub async fn answering_signer(
    status: StatusCode,
    key_version: Option<u32>,
    body: String,
    delay: Duration,
) -> (String, Arc<AtomicUsize>) {
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    let router = Router::new().fallback(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let body = body.clone();
        async move {
            tokio::time::sleep(delay).await;
            let mut headers = HeaderMap::new();
            if let Some(version) = key_version {
                headers.insert("odis-key-version", version.into());
            }
            (status, headers, body)
        }
    });
    (spawn_test_server(router).await, received)
}

/// A signer that answers every request with `status` and `body` after `delay`, and counts the
/// requests it receives.
pub async fn fake_signer(
    status: StatusCode,
    body: &str,
    delay: Duration,
) -> (String, Arc<AtomicUsize>) {
    answering_signer(status, None, body.to_string(), delay).await
}

/// A signer's success for message 0 under key version 1, with the vector partial of `share`.
pub fn partial_answer(share: usize) -> String {
    format!(
        r#"{{"success":true,"version":"1.0.0","signature":"{}","performedQueryCount":1,"totalQuota":1000}}"#,
        vector_partial(1, share, 0)
    )
}

/// A test signer that answers `/sign` for message 0 under key version 1 with the vector
/// partial of `share`, after `delay`.
pub async fn partial_signer(share: usize, delay: Duration) -> (String, Arc<AtomicUsize>) {
    answering_signer(StatusCode::OK, Some(1), partial_answer(share), delay).await
}

/// A signer whose PNP API is off, as the TS suite takes a signer down. The Rust signer has no
/// such switch, so a test server answers as a disabled TS signer does.
pub async fn disabled_signer() -> String {
    fake_signer(
        StatusCode::SERVICE_UNAVAILABLE,
        DISABLED_SIGNER_ANSWER,
        Duration::ZERO,
    )
    .await
    .0
}

// -- The combiner --

enum DekAnswer {
    Dek(String),
    Failure,
    Panic,
}

/// A DEK lookup with a fixed answer, which counts how often it was asked.
pub struct FixedDek {
    answer: DekAnswer,
    calls: AtomicUsize,
}

impl FixedDek {
    /// Answers `dek` for every account.
    pub fn returning(dek: &str) -> Arc<FixedDek> {
        Self::new(DekAnswer::Dek(dek.to_string()))
    }

    /// Answers `DEK_PUBLIC_KEY` for every account.
    pub fn with_test_dek() -> Arc<FixedDek> {
        Self::returning(DEK_PUBLIC_KEY.trim_start_matches("0x"))
    }

    /// Answers as a full node that cannot be reached.
    pub fn failing() -> Arc<FixedDek> {
        Self::new(DekAnswer::Failure)
    }

    pub fn panicking() -> Arc<FixedDek> {
        Self::new(DekAnswer::Panic)
    }

    fn new(answer: DekAnswer) -> Arc<FixedDek> {
        Arc::new(FixedDek {
            answer,
            calls: AtomicUsize::new(0),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl DekLookup for FixedDek {
    async fn dek(&self, _account: Address) -> Result<String, DekLookupError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.answer {
            DekAnswer::Dek(dek) => Ok(dek.clone()),
            DekAnswer::Failure => Err(DekLookupError::FullNode("connection refused".to_string())),
            DekAnswer::Panic => panic!("FixedDek panics on purpose"),
        }
    }
}

pub struct CombinerSetup {
    pub signers: Vec<SignerEntry>,
    pub key_versions: Vec<DevKey>,
    pub current_key_version: u32,
    pub timeout: Duration,
    pub dek: Arc<FixedDek>,
}

impl CombinerSetup {
    /// The signers at `urls` with share indices from 0, the dev keys with version 1 current,
    /// a 5 second deadline, and `DEK_PUBLIC_KEY` for every account.
    pub fn over(urls: &[&str]) -> CombinerSetup {
        CombinerSetup {
            signers: urls
                .iter()
                .enumerate()
                .map(|(index, url)| SignerEntry {
                    url: url.to_string(),
                    share_index: Some(index as u32),
                })
                .collect(),
            key_versions: dev_keys(),
            current_key_version: 1,
            timeout: Duration::from_secs(5),
            dek: FixedDek::with_test_dek(),
        }
    }
}

pub struct RunningCombiner {
    pub url: String,
}

/// Starts the combiner on 127.0.0.1. It renders the metrics of the signer's recorder, which
/// in-process signers install; the combiner's own `install_recorder` would fail after them.
pub async fn spawn_combiner(setup: CombinerSetup) -> RunningCombiner {
    let key_versions = setup
        .key_versions
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
        .collect();
    let config = Config {
        server_port: 0,
        service_name: "combiner-test".to_string(),
        signers: setup.signers,
        signer_timeout: setup.timeout,
        key_versions,
        current_key_version: setup.current_key_version,
        blockchain_provider: "http://127.0.0.1:1".to_string(),
        blockchain_api_key: None,
        chain_id: 11142220,
        full_node_timeout: Duration::from_secs(1),
        full_node_retry_count: 0,
        full_node_retry_delay: Duration::from_millis(1),
        log_format: LogFormat::Text,
    };
    let router =
        build_router_with_services(config, setup.dek, odis_signer::metrics::install_recorder())
            .unwrap();
    RunningCombiner {
        url: spawn_test_server(router).await,
    }
}

/// The combiner in front of real signers made from `setups`, and those signers.
pub struct Fleet {
    pub combiner: RunningCombiner,
    pub signers: Vec<RunningSigner>,
    pub dek: Arc<FixedDek>,
}

pub async fn fleet_of(setups: Vec<SignerSetup>, dek: Arc<FixedDek>) -> Fleet {
    fleet_with(setups, |setup| CombinerSetup { dek, ..setup }).await
}

/// The combiner in front of real signers made from `setups`, set up by `configure` from
/// `CombinerSetup::over` the signers.
pub async fn fleet_with(
    setups: Vec<SignerSetup>,
    configure: impl FnOnce(CombinerSetup) -> CombinerSetup,
) -> Fleet {
    let mut signers = Vec::new();
    for setup in setups {
        signers.push(spawn_signer(setup).await);
    }
    let urls: Vec<&str> = signers.iter().map(|s| s.url.as_str()).collect();
    let setup = configure(CombinerSetup::over(&urls));
    let dek = setup.dek.clone();
    let combiner = spawn_combiner(setup).await;
    Fleet {
        combiner,
        signers,
        dek,
    }
}

/// Three signers with shares 1 to 3, quota 1000 and the DEK.
pub async fn fleet() -> Fleet {
    let setups = (0..3).map(SignerSetup::share).collect();
    fleet_of(setups, FixedDek::with_test_dek()).await
}

/// A combiner in front of `urls`, with a DEK lookup that answers the DEK.
pub async fn combiner_over(urls: &[&str]) -> RunningCombiner {
    spawn_combiner(CombinerSetup::over(urls)).await
}

// -- Requests --

/// A wallet-key signature of `body` by `PRIVATE_KEY1`: `0x` and r, s, v in hex.
pub async fn wallet_auth(body: &str) -> String {
    wallet_auth_by(PRIVATE_KEY1, body).await
}

/// A wallet-key signature of `body` by `private_key`.
pub async fn wallet_auth_by(private_key: &str, body: &str) -> String {
    let signer = PrivateKeySigner::from_str(private_key).unwrap();
    let signature = signer.sign_message(body.as_bytes()).await.unwrap();
    format!("0x{}", hex::encode(signature.as_bytes()))
}

/// A DEK signature of `body` by `DEK_PRIVATE_KEY`, as `signWithRawKey` writes it: the DER
/// bytes of a signature over SHA-256 of `JSON.stringify(body)`, as a JSON array.
pub fn dek_auth(body: &str) -> String {
    dek_auth_by(DEK_PRIVATE_KEY, body)
}

/// A DEK signature of `body` by `private_key`, as `dek_auth` writes it.
pub fn dek_auth_by(private_key: &str, body: &str) -> String {
    let key = SigningKey::from_slice(&hex::decode(&private_key[2..]).unwrap()).unwrap();
    let digest = Sha256::digest(serde_json::to_string(body).unwrap().as_bytes());
    let signature: Signature = key.sign_prehash(&digest).unwrap();
    serde_json::to_string(signature.to_der().as_bytes()).unwrap()
}

/// POSTs `body` to `url` + `path` with `Content-Type: application/json`, unless `headers`
/// names another, and every header in `headers` added as given, so a name may repeat.
pub async fn post(
    url: &str,
    path: &str,
    body: impl Into<reqwest::Body>,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, String) {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut request = client.post(format!("{url}{path}")).body(body);
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        request = request.header("content-type", "application/json");
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    (status, headers, response.text().await.unwrap())
}

/// The success body of `/quotaStatus`.
pub fn quota_success(performed_query_count: u64, total_quota: u64, warnings: &[&str]) -> String {
    let warnings = serde_json::to_string(warnings).unwrap();
    format!(
        r#"{{"success":true,"version":"{VERSION}","performedQueryCount":{performed_query_count},"totalQuota":{total_quota},"warnings":{warnings}}}"#
    )
}

/// The failure body of either endpoint.
pub fn failure(error: &str) -> String {
    format!(r#"{{"success":false,"version":"{VERSION}","error":"{error}"}}"#)
}

/// `ACCOUNT_ADDRESS2` of the TS test values: the checksummed address of `PRIVATE_KEY2`.
pub fn account2() -> String {
    PrivateKeySigner::from_str(PRIVATE_KEY2)
        .unwrap()
        .address()
        .to_checksum(None)
}

/// A valid blinded query, a different one for each `n` from 1: the generator times `n`.
pub fn blinded_query(n: u64) -> String {
    let mut scalar = Scalar::zero();
    scalar.set_int(n);
    let mut point = G1::one();
    point.mul(&scalar);
    BASE64.encode(bincode::serialize(&point).unwrap())
}

/// Spends `count` queries of `ACCOUNT` at the signer, with `/sign` requests made directly.
pub async fn use_queries(signer: &RunningSigner, first: u64, count: u64) {
    for n in first..first + count {
        let body = format!(
            r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{}"}}"#,
            blinded_query(n)
        );
        let authorization = wallet_auth(&body).await;
        let (status, _, answer) = post(
            &signer.url,
            "/sign",
            body,
            &[("authorization", authorization.as_str())],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
    }
}

/// The value of `series`, written as the exposition names it (`name{label="value",...}`), on the
/// combiner's `/metrics`; 0 before it is first exported.
pub async fn counter(combiner: &RunningCombiner, series: &str) -> u64 {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let text = client
        .get(format!("{}/metrics", combiner.url))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix(series)?.strip_prefix(' '))
        .map_or(0, |value| value.parse().unwrap())
}

/// The account's used quota as the signer reports it, asked directly with a wallet-key signature.
pub async fn signer_used_quota(signer: &RunningSigner) -> u64 {
    let body = format!(r#"{{"account":"{ACCOUNT}"}}"#);
    let authorization = wallet_auth(&body).await;
    let (status, _, answer) = post(
        &signer.url,
        "/quotaStatus",
        body,
        &[("authorization", authorization.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
    answer["performedQueryCount"].as_u64().unwrap()
}
