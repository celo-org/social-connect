//! `/sign` through the combiner, in front of real Rust signers. The cases of the TS combiner's
//! suite (`apps/combiner/test/integration/pnp.test.ts`) come first, with the same expected
//! bodies, then cases the TS suite does not cover or where the Rust combiner differs from TS
//! (CHANGELOG.md).

mod common;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use odis_combiner::config::SignerEntry;
use odis_combiner::errors::{
    EXCEEDED_QUOTA, INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS, INVALID_INPUT,
    INVALID_KEY_VERSION_REQUEST, NOT_ENOUGH_PARTIAL_SIGNATURES, SIGNER_RESPONSE_DISCREPANCIES,
    TIMEOUT_FROM_SIGNER, UNAUTHENTICATED_USER, VERIFY_PARTIAL_SIGNATURE_ERROR,
};
use rand_chacha::ChaChaRng;
use rand_core::SeedableRng;
use threshold_bls::schemes::bls12_377::G2Scheme;
use threshold_bls::sig::BlindScheme;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

use common::*;

/// What the TS combiner answers for message 0 under key versions 1 to 3 (`pnp.test.ts:263-267`).
const TS_SIGNATURES: [&str; 3] = [
    "xgFMQtcgAMHJAEX/m9B4VFopYtxqPFSw0024sWzRYvQDvnmFqhXOPdnRDfa8WCEA",
    "wUuFV8yFBXGyEzKbyWjBChG6dER264nwjOsqErd/UZieVKE0oDMZcMDG+qObu4QB",
    "PJHqBGavcQG3NGFl3hiR8GymeDNumxbl1DnCJzWz+Ik5yCN2ZpAITBe24RTX0iMA",
];
/// Those signatures unblinded (`pnp.test.ts:270-274`).
const TS_UNBLINDED_SIGNATURES: [&str; 3] = [
    "lOASnDJNbJBTMYfkbU4fMiK7FcNwSyqZo8iQSM95X8YK+/158be4S1A+jcQsCUYA",
    "QIT7HtHTe/d0Tq40Mf3rpHCT8qY20+8q7ZW9PXHFMWGvwSGhk7l3Pfwnx8YdXomB",
    "XW//DolLzaXYS/gk9WBHfeKy5HKrGjuF/OpCok/i6fprE4AGFH2PjE7zeKTfOQ+A",
];
/// What the TS combiner answers for `second test message` under key version 1
/// (`pnp.test.ts:716-717`).
const TS_SECOND_MESSAGE_SIGNATURE: &str =
    "PWvuSYIA249x1dx+qzgl6PKSkoulXXE/P4WHJvGmtw77pCRilEWTn3xSp+6JS9+A";
/// A failure body of a signer that broke.
const SIGNER_ERROR_ANSWER: &str =
    r#"{"success":false,"version":"1.0.0","error":"CELO_ODIS_ERR_00 Something went wrong"}"#;

// -- Messages --

type BlindingFactor = <G2Scheme as BlindScheme>::Token;

/// The TS suite's blinding seed: 0, 1, …, 30, 0 (`pnp.test.ts:310-315`).
fn ts_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    for (i, byte) in seed[..31].iter_mut().enumerate() {
        *byte = i as u8;
    }
    seed
}

/// `message` blinded as the TS library blinds it with `seed`, as `rust/signer/src/crypto.rs`
/// does.
fn blind_with(message: &[u8], seed: [u8; 32]) -> (BlindingFactor, Vec<u8>) {
    G2Scheme::blind_msg(message, &mut ChaChaRng::from_seed(seed)).unwrap()
}

/// `message` blinded with the TS suite's seed.
fn blind(message: &[u8]) -> (BlindingFactor, Vec<u8>) {
    blind_with(message, ts_seed())
}

/// The base64 signature in a `/sign` success, unblinded with `blinding_factor`, in base64.
fn unblinded(answer: &str, blinding_factor: &BlindingFactor) -> String {
    let answer: serde_json::Value = serde_json::from_str(answer).unwrap();
    let signature = BASE64
        .decode(answer["signature"].as_str().unwrap())
        .unwrap();
    BASE64.encode(G2Scheme::unblind_sig(blinding_factor, &signature).unwrap())
}

/// Message 0 of the vectors, `test message` blinded, in base64.
fn query() -> String {
    BASE64.encode(vector_message(0))
}

// -- Requests and answers --

/// The TS suite's request (`pnp.test.ts:226-232`).
fn sign_body(query: &str) -> String {
    format!(
        r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}","sessionID":"{SESSION_ID}"}}"#
    )
}

/// The TS suite's request with `authenticationMethod` set afterwards, so it comes last.
fn dek_sign_body(query: &str) -> String {
    format!(
        r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}","sessionID":"{SESSION_ID}","authenticationMethod":"encryption_key"}}"#
    )
}

async fn sign(combiner: &RunningCombiner, body: &str, authorization: &str) -> (StatusCode, String) {
    sign_with(combiner, body, &[("authorization", authorization)]).await
}

async fn sign_with(
    combiner: &RunningCombiner,
    body: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let (status, _, answer) = post(&combiner.url, "/sign", body.to_string(), headers).await;
    (status, answer)
}

/// Sends the TS suite's request for message 0, signed with the wallet key.
async fn sign_query(combiner: &RunningCombiner) -> (StatusCode, String) {
    let body = sign_body(&query());
    sign(combiner, &body, &wallet_auth(&body).await).await
}

fn sign_success(
    signature: &str,
    performed_query_count: u64,
    total_quota: u64,
    warnings: &[&str],
) -> String {
    let warnings = serde_json::to_string(warnings).unwrap();
    format!(
        r#"{{"success":true,"version":"{VERSION}","signature":"{signature}","performedQueryCount":{performed_query_count},"totalQuota":{total_quota},"warnings":{warnings}}}"#
    )
}

/// The answer to the TS suite's request on a fresh fleet: the key version 1 signature, one
/// query used of 1000.
fn first_success() -> (StatusCode, String) {
    (StatusCode::OK, sign_success(TS_SIGNATURES[0], 1, 1000, &[]))
}

fn not_enough(status: StatusCode) -> (StatusCode, String) {
    (status, failure(NOT_ENOUGH_PARTIAL_SIGNATURES))
}

fn warnings_series(warning: &str) -> String {
    format!(r#"combiner_warnings_total{{endpoint="/sign",warning_type="{warning}"}}"#)
}

fn errors_series(error: &str) -> String {
    format!(r#"combiner_errors_total{{endpoint="/sign",error_type="{error}"}}"#)
}

const NOT_ENOUGH_SIG_SERIES: &str = r#"combiner_not_enough_sig_errors_total{endpoint="/sign"}"#;

fn bls_errors_series(signer: &str) -> String {
    format!(r#"combiner_bls_compute_errors_total{{endpoint="/sign",signer="{signer}"}}"#)
}

/// The values of `series` on the combiner's `/metrics`.
async fn counters(combiner: &RunningCombiner, series: &[String]) -> Vec<u64> {
    let mut values = Vec::new();
    for s in series {
        values.push(counter(combiner, s).await);
    }
    values
}

// -- Signers --

/// Share `index` of the dev keys, except that key version 1 holds key version 2's share of
/// the same index: what a TS signer signs with on a replay under another version. The share
/// index matches, so only verification catches it.
fn other_version_share(index: usize) -> SignerSetup {
    let mut setup = SignerSetup::share(index);
    setup.shares.insert(1, dev_keys()[1].shares[index].clone());
    setup
}

/// Share `index` of the 4-of-5 key, version 4.
fn four_of_five_share(index: usize) -> SignerSetup {
    SignerSetup {
        shares: HashMap::from([(4, four_of_five_key().shares[index].clone())]),
        ..SignerSetup::share(0)
    }
}

/// The combiner in front of the signers made from `setups`, with the 4-of-5 key current.
async fn four_of_five_fleet(setups: Vec<SignerSetup>) -> Fleet {
    fleet_with(setups, |setup| CombinerSetup {
        key_versions: vec![four_of_five_key()],
        current_key_version: 4,
        ..setup
    })
    .await
}

/// Sends on its channel when dropped.
struct DropGuard(Option<oneshot::Sender<()>>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

/// A signer that never answers. The receiver resolves when the server drops the request,
/// which it does once the combiner closes the connection.
async fn silent_signer() -> (String, oneshot::Receiver<()>) {
    let (sender, dropped) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let url = spawn_test_server(Router::new().fallback(move || {
        let guard = DropGuard(sender.lock().unwrap().take());
        async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        }
    }))
    .await;
    (url, dropped)
}

/// Waits up to 5 seconds for the signer to report `expected` used queries for `ACCOUNT`.
async fn wait_for_used_quota(signer: &RunningSigner, expected: u64) {
    let started = Instant::now();
    loop {
        let used = signer_used_quota(signer).await;
        if used == expected {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{} reports {used} used queries, not {expected}",
            signer.url
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// -- The TS suite: signers operating correctly --

#[tokio::test]
async fn combined_signature_equals_the_ts_combiners() {
    let (blinding_factor, blinded) = blind(b"test message");
    assert_eq!(blinded, vector_message(0));
    let body = sign_body(&BASE64.encode(&blinded));
    let authorization = wallet_auth(&body).await;

    for (header, version) in [(None, 1), (Some("1"), 1), (Some("2"), 2), (Some("3"), 3)] {
        let fleet = fleet().await;
        let mut headers = vec![("authorization", authorization.as_str())];
        if let Some(header) = header {
            headers.push(("odis-key-version", header));
        }

        let (status, answer) = sign_with(&fleet.combiner, &body, &headers).await;

        assert_eq!(
            (status, answer.as_str()),
            (
                StatusCode::OK,
                sign_success(TS_SIGNATURES[version - 1], 1, 1000, &[]).as_str()
            ),
            "header {header:?}"
        );
        assert_eq!(
            unblinded(&answer, &blinding_factor),
            TS_UNBLINDED_SIGNATURES[version - 1],
            "header {header:?}"
        );
    }
}

#[tokio::test]
async fn repeated_request_succeeds() {
    let fleet = fleet().await;
    let body = sign_body(&query());
    let authorization = wallet_auth(&body).await;

    // The query is charged once.
    for _ in 0..2 {
        assert_eq!(
            sign(&fleet.combiner, &body, &authorization).await,
            first_success()
        );
    }
}

#[tokio::test]
async fn new_message_increments_the_count() {
    let fleet = fleet().await;
    assert_eq!(sign_query(&fleet.combiner).await, first_success());

    let (_, second) = blind(b"second test message");
    let body = sign_body(&BASE64.encode(second));

    assert_eq!(
        sign(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (
            StatusCode::OK,
            sign_success(TS_SECOND_MESSAGE_SIGNATURE, 2, 1000, &[])
        )
    );
}

#[tokio::test]
async fn same_message_with_another_seed_unblinds_to_the_same_signature() {
    let fleet = fleet().await;
    let (first_factor, first) = blind(b"test message");
    let mut seed = ts_seed();
    seed[0] += 1;
    let (second_factor, second) = blind_with(b"test message", seed);
    assert_ne!(first, second);

    let first_body = sign_body(&BASE64.encode(first));
    let first_answer = sign(
        &fleet.combiner,
        &first_body,
        &wallet_auth(&first_body).await,
    )
    .await;
    assert_eq!(first_answer, first_success());
    let second_body = sign_body(&BASE64.encode(second));
    let (status, second_answer) = sign(
        &fleet.combiner,
        &second_body,
        &wallet_auth(&second_body).await,
    )
    .await;

    // Another blinded message is another query.
    let signature: serde_json::Value = serde_json::from_str(&second_answer).unwrap();
    assert_eq!(
        (status, second_answer.as_str()),
        (
            StatusCode::OK,
            sign_success(signature["signature"].as_str().unwrap(), 2, 1000, &[]).as_str()
        )
    );
    assert_eq!(
        unblinded(&first_answer.1, &first_factor),
        TS_UNBLINDED_SIGNATURES[0]
    );
    assert_eq!(
        unblinded(&second_answer, &second_factor),
        TS_UNBLINDED_SIGNATURES[0]
    );
}

#[tokio::test]
async fn extra_fields_are_accepted() {
    let fleet = fleet().await;
    let body = format!(
        r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{}","sessionID":"{SESSION_ID}","extraField":"dummyString"}}"#,
        query()
    );

    assert_eq!(
        sign(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        first_success()
    );
}

#[tokio::test]
async fn dek_authentication_succeeds() {
    let fleet = fleet().await;
    let body = dek_sign_body(&query());

    assert_eq!(
        sign(&fleet.combiner, &body, &dek_auth(&body)).await,
        first_success()
    );
    assert_eq!(fleet.dek.calls(), 1);
}

/// TS sends `invalid`. `a` is a second header that `Number()` reads as NaN.
#[tokio::test]
async fn invalid_key_version_header_uses_the_current_version() {
    let fleet = fleet().await;
    let body = dek_sign_body(&query());
    let authorization = dek_auth(&body);

    for header in ["a", "invalid"] {
        assert_eq!(
            sign_with(
                &fleet.combiner,
                &body,
                &[
                    ("authorization", &authorization),
                    ("odis-key-version", header)
                ]
            )
            .await,
            first_success(),
            "{header}"
        );
    }
}

#[tokio::test]
async fn missing_fields_are_400() {
    let fleet = fleet().await;
    let series = warnings_series(INVALID_INPUT);
    let before = counter(&fleet.combiner, &series).await;
    let body = format!(
        r#"{{"blindedQueryPhoneNumber":"{}","sessionID":"{SESSION_ID}"}}"#,
        query()
    );

    assert_eq!(
        sign(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::BAD_REQUEST, failure(INVALID_INPUT))
    );
    assert_eq!(counter(&fleet.combiner, &series).await, before + 1);
}

#[tokio::test]
async fn unsupported_key_version_is_400_warn_12() {
    let fleet = fleet().await;
    let series = warnings_series(INVALID_KEY_VERSION_REQUEST);
    let before = counter(&fleet.combiner, &series).await;
    let body = sign_body(&query());
    let authorization = wallet_auth(&body).await;

    assert_eq!(
        sign_with(
            &fleet.combiner,
            &body,
            &[("authorization", &authorization), ("odis-key-version", "4")]
        )
        .await,
        (
            StatusCode::BAD_REQUEST,
            failure(INVALID_KEY_VERSION_REQUEST)
        )
    );
    assert_eq!(counter(&fleet.combiner, &series).await, before + 1);
}

/// TS sends the request from `mockAccount`, `0x…7E57`, whose checksum is wrong, so TS refuses it
/// at the address check. Here the account is `ACCOUNT_ADDRESS2`, correctly checksummed, so the
/// signature check refuses it.
#[tokio::test]
async fn failed_wallet_key_auth_is_401() {
    let fleet = fleet().await;
    let series = warnings_series(UNAUTHENTICATED_USER);
    let before = counter(&fleet.combiner, &series).await;
    let body = format!(
        r#"{{"account":"{}","blindedQueryPhoneNumber":"{}","sessionID":"{SESSION_ID}"}}"#,
        account2(),
        query()
    );

    assert_eq!(
        sign(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
    assert_eq!(counter(&fleet.combiner, &series).await, before + 1);
}

/// TS uses `mockAccount` here too, refused before the DEK lookup. With `ACCOUNT_ADDRESS2` the DEK
/// is looked up and the signature by another key fails both checks.
#[tokio::test]
async fn failed_dek_auth_is_401() {
    let fleet = fleet().await;
    let different_key = "0x00000000000000000000000000000000000000000000000000000000ddddbbbb";
    let body = format!(
        r#"{{"account":"{}","blindedQueryPhoneNumber":"{}","sessionID":"{SESSION_ID}","authenticationMethod":"encryption_key"}}"#,
        account2(),
        query()
    );

    assert_eq!(
        sign(&fleet.combiner, &body, &dek_auth_by(different_key, &body)).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
    assert_eq!(fleet.dek.calls(), 1);
}

#[tokio::test]
async fn failure_to_fetch_the_dek_is_401() {
    let fleet = fleet_of(
        (0..3).map(SignerSetup::share).collect(),
        FixedDek::failing(),
    )
    .await;
    let body = dek_sign_body(&query());

    assert_eq!(
        sign(&fleet.combiner, &body, &dek_auth(&body)).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
    assert_eq!(fleet.dek.calls(), 1);
}

/// The TS combiner means to count `WARN_03` here and never does.
#[tokio::test]
async fn out_of_quota_is_403_warn_03_and_counted() {
    let setups = (0..3)
        .map(|index| SignerSetup {
            total_quota: 0,
            ..SignerSetup::share(index)
        })
        .collect();
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        warnings_series(EXCEEDED_QUOTA),
        NOT_ENOUGH_SIG_SERIES.to_string(),
    ];
    let before = counters(&fleet.combiner, &series).await;

    assert_eq!(
        sign_query(&fleet.combiner).await,
        (StatusCode::FORBIDDEN, failure(EXCEEDED_QUOTA))
    );
    assert_eq!(
        counters(&fleet.combiner, &series).await,
        [before[0] + 1, before[1]]
    );
}

// -- The TS suite: signers not operating correctly --

/// TS gives signer 1 a share with index 0 and a wrong value. Here signer 1 holds key version
/// 2's share 1 under version 1, which has the configured index and fails verification.
#[tokio::test]
async fn two_of_three_correct_signatures_succeed() {
    let setups = vec![
        other_version_share(0),
        SignerSetup::share(1),
        SignerSetup::share(2),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;

    assert_eq!(sign_query(&fleet.combiner).await, first_success());
}

#[tokio::test]
async fn one_of_three_correct_fails_with_err_07() {
    let setups = vec![
        SignerSetup::share(0),
        other_version_share(1),
        other_version_share(2),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        NOT_ENOUGH_SIG_SERIES.to_string(),
        errors_series(NOT_ENOUGH_PARTIAL_SIGNATURES),
    ];
    let before = counters(&fleet.combiner, &series).await;

    assert_eq!(
        sign_query(&fleet.combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );
    assert_eq!(
        counters(&fleet.combiner, &series).await,
        [before[0] + 1, before[1] + 1]
    );
}

#[tokio::test]
async fn two_of_three_signers_down_fails() {
    let signer = spawn_signer(SignerSetup::share(0)).await;
    let (down1, down2) = (disabled_signer().await, disabled_signer().await);
    let combiner = combiner_over(&[&signer.url, &down1, &down2]).await;

    // The majority status of the failures.
    assert_eq!(
        sign_query(&combiner).await,
        not_enough(StatusCode::SERVICE_UNAVAILABLE)
    );
}

#[tokio::test]
async fn one_of_three_signers_down_succeeds() {
    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(1)).await;
    let down = disabled_signer().await;
    let combiner = combiner_over(&[&signer1.url, &signer2.url, &down]).await;

    assert_eq!(sign_query(&combiner).await, first_success());
}

/// The TS suite sets the signers' own timeout to 0. Here the signers answer after the
/// combiner's deadline, which ends the same way, and shows that the deadline is kept.
#[tokio::test]
async fn signer_timeouts_fail() {
    let mut urls = Vec::new();
    for share in 0..3 {
        urls.push(partial_signer(share, Duration::from_secs(30)).await.0);
    }
    let url_refs: Vec<&str> = urls.iter().map(String::as_str).collect();
    let combiner = spawn_combiner(CombinerSetup {
        timeout: Duration::from_millis(300),
        ..CombinerSetup::over(&url_refs)
    })
    .await;
    let timeouts: Vec<String> = urls
        .iter()
        .map(|url| {
            format!(
                r#"combiner_sig_request_errors_total{{signer="{url}",endpoint="/sign",error_type="{TIMEOUT_FROM_SIGNER}"}}"#
            )
        })
        .collect();
    let before = counters(&combiner, &timeouts).await;

    let started = Instant::now();
    let answer = sign_query(&combiner).await;

    assert_eq!(answer, not_enough(StatusCode::INTERNAL_SERVER_ERROR));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        counters(&combiner, &timeouts).await,
        before.iter().map(|n| n + 1).collect::<Vec<_>>()
    );
}

// -- The TS suite: 4 of 5 --

#[tokio::test]
async fn four_of_five_succeeds() {
    let fleet = four_of_five_fleet((0..5).map(four_of_five_share).collect()).await;

    assert_eq!(
        sign_query(&fleet.combiner).await,
        (
            StatusCode::OK,
            sign_success(&vector_signature(4, 0), 1, 1000, &[])
        )
    );
}

#[tokio::test]
async fn four_of_five_out_of_quota_is_403() {
    let setups = (0..5)
        .map(|index| SignerSetup {
            total_quota: 0,
            ..four_of_five_share(index)
        })
        .collect();
    let fleet = four_of_five_fleet(setups).await;

    assert_eq!(
        sign_query(&fleet.combiner).await,
        (StatusCode::FORBIDDEN, failure(EXCEEDED_QUOTA))
    );
}

/// Two failures leave 5 − 2 < 4 signers, so the request ends without waiting for the signer
/// that never answers.
#[tokio::test]
async fn four_of_five_ends_early_on_a_minority_of_failures() {
    let signer1 = spawn_signer(four_of_five_share(0)).await;
    let signer2 = spawn_signer(four_of_five_share(1)).await;
    let (failing1, _) = fake_signer(
        StatusCode::INTERNAL_SERVER_ERROR,
        SIGNER_ERROR_ANSWER,
        Duration::ZERO,
    )
    .await;
    let (failing2, _) = fake_signer(
        StatusCode::INTERNAL_SERVER_ERROR,
        SIGNER_ERROR_ANSWER,
        Duration::ZERO,
    )
    .await;
    let (silent, _dropped) = silent_signer().await;
    let urls = [
        signer1.url.as_str(),
        signer2.url.as_str(),
        &failing1,
        &failing2,
        &silent,
    ];
    let combiner = spawn_combiner(CombinerSetup {
        key_versions: vec![four_of_five_key()],
        current_key_version: 4,
        ..CombinerSetup::over(&urls)
    })
    .await;

    let started = Instant::now();
    let answer = sign_query(&combiner).await;

    assert_eq!(answer, not_enough(StatusCode::INTERNAL_SERVER_ERROR));
    // The deadline is 5 seconds.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

// -- Beyond the TS suite --

/// Signer 2 answers late, so signer 3's partial is in the first combination, which fails, and
/// signer 3 is dropped before signer 2's partial completes the threshold.
#[tokio::test]
async fn partial_of_another_key_version_is_dropped() {
    let setups = vec![
        SignerSetup::share(0),
        SignerSetup {
            key_delay: Duration::from_millis(300),
            ..SignerSetup::share(1)
        },
        other_version_share(2),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        bls_errors_series(&fleet.signers[2].url),
        errors_series(VERIFY_PARTIAL_SIGNATURE_ERROR),
    ];
    let before = counters(&fleet.combiner, &series).await;

    assert_eq!(sign_query(&fleet.combiner).await, first_success());
    assert_eq!(
        counters(&fleet.combiner, &series).await,
        [before[0] + 1, before[1] + 1]
    );

    let setups = vec![
        SignerSetup::share(0),
        other_version_share(1),
        other_version_share(2),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    assert_eq!(
        sign_query(&fleet.combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );
}

/// Signer 3 has a wrong share and quota 1010, and answers before signer 2. Its partial is
/// dropped and its quota numbers still count: the total quotas differ by 10, a standard
/// deviation of 4.7, so the answer warns and takes the numbers of the signer at the threshold
/// when sorted by remaining quota, not those of signer 3, which has the most remaining.
#[tokio::test]
async fn a_bad_partial_still_counts_its_quota() {
    let setups = vec![
        SignerSetup::share(0),
        SignerSetup {
            key_delay: Duration::from_millis(300),
            ..SignerSetup::share(1)
        },
        SignerSetup {
            total_quota: 1010,
            ..other_version_share(2)
        },
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let best_guess =
        format!("{INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS}, using threshold signer as best guess");

    assert_eq!(
        sign_query(&fleet.combiner).await,
        (
            StatusCode::OK,
            sign_success(
                TS_SIGNATURES[0],
                1,
                1000,
                &[
                    SIGNER_RESPONSE_DISCREPANCIES,
                    INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
                    &best_guess
                ]
            )
        )
    );
}

/// The `/quotaStatus` case `large_total_quota_discrepancy_fails_with_err_23`, on `/sign`. Signer
/// 3 answers late, so signers 1 and 2 combine. Their total quotas, 1015 and 1000, have a
/// standard deviation of 7.5, above 5, so the quota state cannot be computed. The combined
/// signature is then not answered, and the request fails as one without enough partials, as in
/// TS.
#[tokio::test]
async fn large_total_quota_discrepancy_fails_with_err_07() {
    let setups = vec![
        SignerSetup {
            total_quota: 1015,
            ..SignerSetup::share(0)
        },
        SignerSetup::share(1),
        SignerSetup {
            key_delay: Duration::from_millis(300),
            ..SignerSetup::share(2)
        },
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        errors_series(NOT_ENOUGH_PARTIAL_SIGNATURES),
        NOT_ENOUGH_SIG_SERIES.to_string(),
    ];
    let before = counters(&fleet.combiner, &series).await;

    assert_eq!(
        sign_query(&fleet.combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );
    assert_eq!(
        counters(&fleet.combiner, &series).await,
        [before[0] + 1, before[1] + 1]
    );
}

/// The signers have share indices 0 to 2. Signer 3 holds share 1, index 0. Signer 2 answers
/// late, so signer 3's partial arrives before the threshold is reached.
#[tokio::test]
async fn partial_of_another_share_is_dropped_and_named() {
    let setups = vec![
        SignerSetup::share(0),
        SignerSetup {
            key_delay: Duration::from_millis(300),
            ..SignerSetup::share(1)
        },
        SignerSetup::share(0),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        bls_errors_series(&fleet.signers[2].url),
        bls_errors_series(&fleet.signers[0].url),
        errors_series(VERIFY_PARTIAL_SIGNATURE_ERROR),
    ];
    let before = counters(&fleet.combiner, &series).await;

    assert_eq!(sign_query(&fleet.combiner).await, first_success());
    assert_eq!(
        counters(&fleet.combiner, &series).await,
        [before[0] + 1, before[1], before[2] + 1]
    );

    let setups = vec![
        SignerSetup::share(0),
        SignerSetup::share(0),
        SignerSetup::share(0),
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    assert_eq!(
        sign_query(&fleet.combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );
}

/// Without share indices, two partials of the same share pass every check alone, and together
/// they do not combine. The request fails, and succeeds once a third, distinct share arrives.
#[tokio::test]
async fn same_share_twice_without_share_index() {
    let without_indices = |urls: &[&str]| CombinerSetup {
        signers: urls
            .iter()
            .map(|url| SignerEntry {
                url: url.to_string(),
                share_index: None,
            })
            .collect(),
        ..CombinerSetup::over(urls)
    };

    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(0)).await;
    let down = down_signer().await;
    let combiner = spawn_combiner(without_indices(&[&signer1.url, &signer2.url, &down])).await;
    assert_eq!(
        sign_query(&combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );

    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(0)).await;
    let signer3 = spawn_signer(SignerSetup::share(2)).await;
    let combiner =
        spawn_combiner(without_indices(&[&signer1.url, &signer2.url, &signer3.url])).await;
    assert_eq!(sign_query(&combiner).await, first_success());
}

/// The JSON failure arrives first; the 502s 300 ms later, so the request ends early on the
/// first of them. A tie goes to the lower status, so only the 503 case would show a 502 that
/// entered the majority. The 403 case shows that a quota refusal as the majority answers
/// `WARN_03`, not `ERR_07`.
#[tokio::test]
async fn html_and_empty_failures_do_not_enter_the_majority() {
    let late = Duration::from_millis(300);
    let out_of_quota = spawn_signer(SignerSetup {
        total_quota: 0,
        ..SignerSetup::share(0)
    })
    .await;
    let disabled = disabled_signer().await;
    for (json, expected) in [
        (
            out_of_quota.url.clone(),
            (StatusCode::FORBIDDEN, failure(EXCEEDED_QUOTA)),
        ),
        (disabled, not_enough(StatusCode::SERVICE_UNAVAILABLE)),
    ] {
        let (html, _) = fake_signer(
            StatusCode::BAD_GATEWAY,
            "<html><body>502 Bad Gateway</body></html>",
            late,
        )
        .await;
        let (empty, _) = fake_signer(StatusCode::BAD_GATEWAY, "", late).await;
        let combiner = combiner_over(&[&json, &html, &empty]).await;

        assert_eq!(sign_query(&combiner).await, expected, "{json}");
    }
}

/// The oversized answer is a valid success with the right partial, so only the size limit
/// drops it.
#[tokio::test]
async fn answer_above_the_size_limit_is_dropped() {
    let oversized = |share: usize| {
        let answer = partial_answer(share);
        let open = &answer[..answer.len() - 1];
        format!(r#"{open},"padding":"{}"}}"#, "x".repeat(70 * 1024))
    };

    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(1)).await;
    let (large, _) = answering_signer(StatusCode::OK, Some(1), oversized(2), Duration::ZERO).await;
    let combiner = combiner_over(&[&signer1.url, &signer2.url, &large]).await;
    assert_eq!(sign_query(&combiner).await, first_success());

    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let (large2, _) = answering_signer(StatusCode::OK, Some(1), oversized(1), Duration::ZERO).await;
    let (large3, _) = answering_signer(StatusCode::OK, Some(1), oversized(2), Duration::ZERO).await;
    let combiner = combiner_over(&[&signer1.url, &large2, &large3]).await;
    assert_eq!(
        sign_query(&combiner).await,
        not_enough(StatusCode::INTERNAL_SERVER_ERROR)
    );
}

/// The signers check the signature over what they receive, so a 200 shows they received the
/// compact form.
#[tokio::test]
async fn compact_signed_body_sent_with_whitespace_passes() {
    let fleet = fleet().await;
    let query = query();
    let wallet_sent = format!(
        "{{\n  \"account\" : \"{ACCOUNT}\",\n  \"blindedQueryPhoneNumber\": \"{query}\",\n  \"sessionID\": \"{SESSION_ID}\"\n}}"
    );
    let dek_sent = format!(
        "{{ \"account\": \"{ACCOUNT}\", \"blindedQueryPhoneNumber\": \"{query}\",\r\n \"sessionID\": \"{SESSION_ID}\", \"authenticationMethod\": \"encryption_key\" }}"
    );

    for (sent, authorization) in [
        (wallet_sent, wallet_auth(&sign_body(&query)).await),
        (dek_sent, dek_auth(&dek_sign_body(&query))),
    ] {
        assert_eq!(
            sign(&fleet.combiner, &sent, &authorization).await,
            first_success(),
            "{sent}"
        );
    }
}

#[tokio::test]
async fn body_signed_with_whitespace_is_401() {
    let fleet = fleet().await;
    let query = query();
    let wallet_body = format!(
        "{{\n  \"account\" : \"{ACCOUNT}\",\n  \"blindedQueryPhoneNumber\": \"{query}\"\n}}"
    );
    let dek_body = format!(
        "{{ \"account\": \"{ACCOUNT}\", \"blindedQueryPhoneNumber\": \"{query}\", \"authenticationMethod\": \"encryption_key\" }}"
    );

    for (body, authorization) in [
        (&wallet_body, wallet_auth(&wallet_body).await),
        (&dek_body, dek_auth(&dek_body)),
    ] {
        assert_eq!(
            sign(&fleet.combiner, body, &authorization).await,
            (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER)),
            "{body}"
        );
    }
}

/// The SDK's bodies (`packages/identity/src/odis/identifier.ts:214-220`): `account`,
/// `blindedQueryPhoneNumber`, `version`, `authenticationMethod`, `sessionID`, which is neither
/// sorted nor the order of a Rust struct. Without a client version `JSON.stringify` leaves
/// `version` out.
#[tokio::test]
async fn sdk_sign_body_passes_with_wallet_key_and_dek() {
    let fleet = fleet().await;
    let query = query();
    let mut bodies = Vec::new();
    for method in ["wallet_key", "encryption_key"] {
        bodies.push(format!(
            r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}","version":"{CLIENT_VERSION}","authenticationMethod":"{method}","sessionID":"{SESSION_ID}"}}"#
        ));
        bodies.push(format!(
            r#"{{"account":"{ACCOUNT}","blindedQueryPhoneNumber":"{query}","authenticationMethod":"{method}","sessionID":"{SESSION_ID}"}}"#
        ));
    }

    for body in &bodies {
        let authorization = if body.contains("encryption_key") {
            dek_auth(body)
        } else {
            wallet_auth(body).await
        };
        assert_eq!(
            sign(&fleet.combiner, body, &authorization).await,
            first_success(),
            "{body}"
        );
    }
}

/// `/sign` reads the key version before it authenticates, unlike `/quotaStatus`.
#[tokio::test]
async fn unsupported_version_and_bad_signature_is_400() {
    let fleet = fleet().await;
    let body = sign_body(&query());
    let signed_elsewhere = wallet_auth("{}").await;

    assert_eq!(
        sign_with(
            &fleet.combiner,
            &body,
            &[
                ("authorization", &signed_elsewhere),
                ("odis-key-version", "4")
            ]
        )
        .await,
        (
            StatusCode::BAD_REQUEST,
            failure(INVALID_KEY_VERSION_REQUEST)
        )
    );
}

#[tokio::test]
async fn key_version_4294967296_is_400_warn_12() {
    let fleet = fleet().await;
    let body = sign_body(&query());
    let authorization = wallet_auth(&body).await;

    assert_eq!(
        sign_with(
            &fleet.combiner,
            &body,
            &[
                ("authorization", &authorization),
                ("odis-key-version", "4294967296")
            ]
        )
        .await,
        (
            StatusCode::BAD_REQUEST,
            failure(INVALID_KEY_VERSION_REQUEST)
        )
    );
}

/// viem's strict `isAddress` refuses these spellings, which the Rust signer would take, and TS
/// refuses them before it looks up a DEK.
#[tokio::test]
async fn upper_case_account_is_401_before_any_dek_lookup() {
    let fleet = fleet().await;
    let query = query();
    let hex = &ACCOUNT[2..];
    let upper = format!("0x{}", hex.to_uppercase());
    let capital_x = format!("0X{hex}");

    for account in [upper.as_str(), capital_x.as_str(), hex] {
        let wallet_body =
            format!(r#"{{"account":"{account}","blindedQueryPhoneNumber":"{query}"}}"#);
        let dek_body = format!(
            r#"{{"account":"{account}","blindedQueryPhoneNumber":"{query}","authenticationMethod":"encryption_key"}}"#
        );
        for (body, authorization) in [
            (&wallet_body, wallet_auth(&wallet_body).await),
            (&dek_body, dek_auth(&dek_body)),
        ] {
            assert_eq!(
                sign(&fleet.combiner, body, &authorization).await,
                (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER)),
                "{body}"
            );
        }
    }
    assert_eq!(fleet.dek.calls(), 0);
}

/// The test signers answer after 200 ms, so every signer has received the valid request before
/// a combination ends the calls.
#[tokio::test]
async fn account_as_an_array_of_numbers_is_400_and_reaches_no_signer() {
    let mut signers = Vec::new();
    for share in 0..3 {
        signers.push(partial_signer(share, Duration::from_millis(200)).await);
    }
    let urls: Vec<&str> = signers.iter().map(|(url, _)| url.as_str()).collect();
    let combiner = combiner_over(&urls).await;
    let received = || {
        signers
            .iter()
            .map(|(_, count)| count.load(Ordering::SeqCst))
            .collect::<Vec<_>>()
    };
    let numbers: Vec<u8> = hex::decode(&ACCOUNT[2..]).unwrap();
    let body = format!(
        r#"{{"account":{},"blindedQueryPhoneNumber":"{}"}}"#,
        serde_json::to_string(&numbers).unwrap(),
        query()
    );

    assert_eq!(
        sign(&combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::BAD_REQUEST, failure(INVALID_INPUT))
    );
    assert_eq!(received(), [0, 0, 0]);

    // The same signers receive a valid request.
    assert_eq!(sign_query(&combiner).await, first_success());
    assert_eq!(received(), [1, 1, 1]);
}

#[tokio::test]
async fn slower_signer_still_charges_after_an_early_success() {
    let setups = vec![
        SignerSetup::share(0),
        SignerSetup::share(1),
        SignerSetup {
            key_delay: Duration::from_millis(500),
            ..SignerSetup::share(2)
        },
    ];
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;

    assert_eq!(sign_query(&fleet.combiner).await, first_success());
    wait_for_used_quota(&fleet.signers[2], 1).await;
}

/// The client writes a whole request and closes the connection without reading the answer.
#[tokio::test]
async fn client_disconnect_still_charges_every_signer() {
    let fleet = fleet().await;
    let body = sign_body(&query());
    let authorization = wallet_auth(&body).await;
    let address = fleet.combiner.url.strip_prefix("http://").unwrap();
    let request = format!(
        "POST /sign HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAuthorization: {authorization}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );

    let mut connection = TcpStream::connect(address).await.unwrap();
    connection.write_all(request.as_bytes()).await.unwrap();
    drop(connection);

    for signer in &fleet.signers {
        wait_for_used_quota(signer, 1).await;
    }
}

#[tokio::test]
async fn early_stop_closes_the_slower_call() {
    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(1)).await;
    let (silent, dropped) = silent_signer().await;
    let combiner = combiner_over(&[&signer1.url, &signer2.url, &silent]).await;

    let started = Instant::now();
    assert_eq!(sign_query(&combiner).await, first_success());
    // Well before the 5 second deadline, which would also close the call.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    tokio::time::timeout(Duration::from_secs(1), dropped)
        .await
        .expect("the silent signer's request is closed within 1 s")
        .unwrap();
}
