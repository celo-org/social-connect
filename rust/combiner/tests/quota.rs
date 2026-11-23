//! `/quotaStatus` through the combiner, in front of real Rust signers. The cases of the TS
//! combiner's suite (`apps/combiner/test/integration/pnp.test.ts`) come first, with the same
//! expected bodies, then cases the TS suite does not cover or where the Rust combiner differs
//! from TS (CHANGELOG.md).

mod common;

use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use alloy_signer_local::PrivateKeySigner;
use axum::http::StatusCode;
use odis_combiner::errors::{
    INCONSISTENT_SIGNER_QUERY_MEASUREMENTS, INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS, INVALID_INPUT,
    INVALID_KEY_VERSION_REQUEST, SIGNER_RESPONSE_DISCREPANCIES, THRESHOLD_PNP_QUOTA_STATUS_FAILURE,
    TIMEOUT_FROM_SIGNER, UNAUTHENTICATED_USER, UNKNOWN_ERROR,
};

use common::*;

// -- Requests and answers --

async fn quota_status(
    combiner: &RunningCombiner,
    body: &str,
    authorization: &str,
) -> (StatusCode, String) {
    quota_status_with(combiner, body, &[("authorization", authorization)]).await
}

async fn quota_status_with(
    combiner: &RunningCombiner,
    body: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let (status, _, answer) = post(&combiner.url, "/quotaStatus", body.to_string(), headers).await;
    (status, answer)
}

fn account_body() -> String {
    format!(r#"{{"account":"{ACCOUNT}"}}"#)
}

fn dek_body() -> String {
    format!(r#"{{"account":"{ACCOUNT}","authenticationMethod":"encryption_key"}}"#)
}

fn warnings_series(warning: &str) -> String {
    format!(r#"combiner_warnings_total{{endpoint="/quotaStatus",warning_type="{warning}"}}"#)
}

// -- The TS suite --

#[tokio::test]
async fn the_test_values_agree() {
    let signer = PrivateKeySigner::from_str(PRIVATE_KEY1).unwrap();
    assert_eq!(signer.address().to_string().to_lowercase(), ACCOUNT);
    assert_eq!(blinded_query(1).len(), 64);
}

#[tokio::test]
async fn quota_is_the_threshold_signers_count() {
    let table: [([u64; 3], u64, &[&str]); 5] = [
        ([0, 0, 0], 0, &[]),
        // Does not reach the threshold.
        ([1, 0, 0], 0, &[SIGNER_RESPONSE_DISCREPANCIES]),
        // Reaches the threshold.
        ([1, 1, 0], 1, &[SIGNER_RESPONSE_DISCREPANCIES]),
        // The order of the signers does not matter.
        ([0, 1, 1], 1, &[SIGNER_RESPONSE_DISCREPANCIES]),
        (
            [1, 4, 9],
            4,
            &[
                SIGNER_RESPONSE_DISCREPANCIES,
                INCONSISTENT_SIGNER_QUERY_MEASUREMENTS,
            ],
        ),
    ];
    for (queries, expected, warnings) in table {
        let fleet = fleet().await;
        for (signer, count) in fleet.signers.iter().zip(queries) {
            use_queries(signer, 1, count).await;
            assert_eq!(signer_used_quota(signer).await, count, "{queries:?}");
        }

        let body = account_body();
        let answer = quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await;

        assert_eq!(
            answer,
            (StatusCode::OK, quota_success(expected, 1000, warnings)),
            "{queries:?}"
        );
    }
}

#[tokio::test]
async fn repeated_requests_succeed() {
    let fleet = fleet().await;
    let body = account_body();
    let authorization = wallet_auth(&body).await;

    for _ in 0..2 {
        assert_eq!(
            quota_status(&fleet.combiner, &body, &authorization).await,
            (StatusCode::OK, quota_success(0, 1000, &[]))
        );
    }
}

#[tokio::test]
async fn extra_fields_are_accepted() {
    let fleet = fleet().await;
    let body = format!(r#"{{"account":"{ACCOUNT}","extraField":"dummy"}}"#);

    assert_eq!(
        quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::OK, quota_success(0, 1000, &[]))
    );
}

#[tokio::test]
async fn dek_authentication_succeeds() {
    let fleet = fleet().await;
    let body = dek_body();

    assert_eq!(
        quota_status(&fleet.combiner, &body, &dek_auth(&body)).await,
        (StatusCode::OK, quota_success(0, 1000, &[]))
    );
    assert_eq!(fleet.dek.calls(), 1);
}

#[tokio::test]
async fn slight_total_quota_discrepancy_warns() {
    let mut setups: Vec<SignerSetup> = (0..3).map(SignerSetup::share).collect();
    setups[0].total_quota = 1001;
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let series = [
        warnings_series(SIGNER_RESPONSE_DISCREPANCIES),
        warnings_series(INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS),
    ];
    let mut before = Vec::new();
    for s in &series {
        before.push(counter(&fleet.combiner, s).await);
    }
    let body = account_body();

    let answer = quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await;

    let best_guess =
        format!("{INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS}, using threshold signer as best guess");
    assert_eq!(
        answer,
        (
            StatusCode::OK,
            quota_success(0, 1000, &[SIGNER_RESPONSE_DISCREPANCIES, &best_guess])
        )
    );
    for (s, before) in series.iter().zip(before) {
        assert_eq!(counter(&fleet.combiner, s).await, before + 1, "{s}");
    }
}

#[tokio::test]
async fn large_total_quota_discrepancy_fails_with_err_23() {
    let mut setups: Vec<SignerSetup> = (0..3).map(SignerSetup::share).collect();
    setups[0].total_quota = 1015;
    let fleet = fleet_of(setups, FixedDek::with_test_dek()).await;
    let body = account_body();

    assert_eq!(
        quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            failure(THRESHOLD_PNP_QUOTA_STATUS_FAILURE)
        )
    );
}

#[tokio::test]
async fn missing_fields_are_400() {
    let fleet = fleet().await;
    let series = warnings_series(INVALID_INPUT);
    let before = counter(&fleet.combiner, &series).await;

    assert_eq!(
        quota_status(&fleet.combiner, "{}", &wallet_auth("{}").await).await,
        (StatusCode::BAD_REQUEST, failure(INVALID_INPUT))
    );
    assert_eq!(counter(&fleet.combiner, &series).await, before + 1);
}

#[tokio::test]
async fn invalid_address_is_400() {
    let fleet = fleet().await;
    let body = r#"{"account":"not an address"}"#;

    assert_eq!(
        quota_status(&fleet.combiner, body, &wallet_auth(body).await).await,
        (StatusCode::BAD_REQUEST, failure(INVALID_INPUT))
    );
}

#[tokio::test]
async fn failed_wallet_key_auth_is_401() {
    let fleet = fleet().await;
    let series = warnings_series(UNAUTHENTICATED_USER);
    let before = counter(&fleet.combiner, &series).await;
    // From one account, signed by another.
    let body = format!(r#"{{"account":"{}"}}"#, account2());

    assert_eq!(
        quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
    assert_eq!(counter(&fleet.combiner, &series).await, before + 1);
}

#[tokio::test]
async fn failed_dek_auth_is_401() {
    let fleet = fleet().await;
    // The TS case spells the field with a capital A, so it names no method.
    let body = format!(
        r#"{{"account":"{}","AuthenticationMethod":"encryption_key"}}"#,
        account2()
    );

    assert_eq!(
        quota_status(&fleet.combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
}

#[tokio::test]
async fn failure_to_fetch_the_dek_is_401() {
    let fleet = fleet_of(
        (0..3).map(SignerSetup::share).collect(),
        FixedDek::failing(),
    )
    .await;
    let body = dek_body();

    assert_eq!(
        quota_status(&fleet.combiner, &body, &dek_auth(&body)).await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
    assert_eq!(fleet.dek.calls(), 1);
}

#[tokio::test]
async fn insufficient_signer_responses_are_500_err_23() {
    let signer = spawn_signer(SignerSetup::share(2)).await;
    let (down1, down2) = (down_signer().await, down_signer().await);
    let combiner = combiner_over(&[&down1, &down2, &signer.url]).await;
    let body = account_body();

    assert_eq!(
        quota_status(&combiner, &body, &wallet_auth(&body).await).await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            failure(THRESHOLD_PNP_QUOTA_STATUS_FAILURE)
        )
    );
}

#[tokio::test]
async fn two_of_three_signers_down_fails() {
    let signer = spawn_signer(SignerSetup::share(0)).await;
    let (down1, down2) = (disabled_signer().await, disabled_signer().await);
    let combiner = combiner_over(&[&signer.url, &down1, &down2]).await;
    let body = account_body();

    // The majority status of the failures.
    assert_eq!(
        quota_status(&combiner, &body, &wallet_auth(&body).await).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            failure(THRESHOLD_PNP_QUOTA_STATUS_FAILURE)
        )
    );
}

#[tokio::test]
async fn one_of_three_signers_down_succeeds() {
    let signer1 = spawn_signer(SignerSetup::share(0)).await;
    let signer2 = spawn_signer(SignerSetup::share(1)).await;
    let down = disabled_signer().await;
    let combiner = combiner_over(&[&signer1.url, &signer2.url, &down]).await;
    let body = account_body();

    assert_eq!(
        quota_status(&combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::OK, quota_success(0, 1000, &[]))
    );
}

/// The TS suite sets the signers' own timeout to 0. Here the signers answer after the
/// combiner's deadline, which ends the same way, and shows that the deadline is kept.
#[tokio::test]
async fn signer_timeouts_fail() {
    let answer = r#"{"success":true,"version":"1.0.0","performedQueryCount":0,"totalQuota":1000}"#;
    let mut urls = Vec::new();
    for _ in 0..3 {
        urls.push(
            fake_signer(StatusCode::OK, answer, Duration::from_secs(30))
                .await
                .0,
        );
    }
    let url_refs: Vec<&str> = urls.iter().map(String::as_str).collect();
    let combiner = spawn_combiner(CombinerSetup {
        timeout: Duration::from_millis(300),
        ..CombinerSetup::over(&url_refs)
    })
    .await;
    let timeouts = |url: &str| {
        format!(
            r#"combiner_sig_request_errors_total{{signer="{url}",endpoint="/quotaStatus",error_type="{TIMEOUT_FROM_SIGNER}"}}"#
        )
    };
    let mut before = Vec::new();
    for url in &urls {
        before.push(counter(&combiner, &timeouts(url)).await);
    }
    let body = account_body();

    let started = std::time::Instant::now();
    let answer = quota_status(&combiner, &body, &wallet_auth(&body).await).await;

    assert_eq!(
        answer,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            failure(THRESHOLD_PNP_QUOTA_STATUS_FAILURE)
        )
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    for (url, before) in urls.iter().zip(before) {
        assert_eq!(
            counter(&combiner, &timeouts(url)).await,
            before + 1,
            "{url}"
        );
    }
}

// -- Beyond the TS suite --

/// `/quotaStatus` authenticates before it resolves the key version.
#[tokio::test]
async fn bad_signature_and_unsupported_version_is_401() {
    let fleet = fleet().await;
    let body = account_body();
    let signed_elsewhere = wallet_auth("{}").await;

    assert_eq!(
        quota_status_with(
            &fleet.combiner,
            &body,
            &[
                ("authorization", &signed_elsewhere),
                ("odis-key-version", "4")
            ]
        )
        .await,
        (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER))
    );
}

#[tokio::test]
async fn unconfigured_key_version_is_400_warn_12() {
    let fleet = fleet().await;
    let series = warnings_series(INVALID_KEY_VERSION_REQUEST);
    let before = counter(&fleet.combiner, &series).await;
    let body = account_body();
    let authorization = wallet_auth(&body).await;

    assert_eq!(
        quota_status_with(
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

#[tokio::test]
async fn key_version_4294967296_is_400_warn_12() {
    let fleet = fleet().await;
    let body = account_body();
    let authorization = wallet_auth(&body).await;

    assert_eq!(
        quota_status_with(
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

/// Node joins the two values to `4, 5`, which `Number()` reads as NaN, so TS uses the current
/// version. Either value alone names a version that is not configured.
#[tokio::test]
async fn repeated_key_version_header_uses_the_current_version() {
    let fleet = fleet().await;
    let body = account_body();
    let authorization = wallet_auth(&body).await;

    assert_eq!(
        quota_status_with(
            &fleet.combiner,
            &body,
            &[
                ("authorization", &authorization),
                ("odis-key-version", "4"),
                ("odis-key-version", "5")
            ]
        )
        .await,
        (StatusCode::OK, quota_success(0, 1000, &[]))
    );
}

/// viem's strict `isAddress` refuses these spellings, which the Rust signer would take, and TS
/// refuses them before it looks up a DEK.
#[tokio::test]
async fn upper_case_account_is_401_before_any_dek_lookup() {
    let fleet = fleet().await;
    let hex = &ACCOUNT[2..];
    let upper = format!("0x{}", hex.to_uppercase());
    let capital_x = format!("0X{hex}");

    for account in [upper.as_str(), capital_x.as_str(), hex] {
        let wallet_body = format!(r#"{{"account":"{account}"}}"#);
        let dek_body =
            format!(r#"{{"account":"{account}","authenticationMethod":"encryption_key"}}"#);
        for (body, authorization) in [
            (&wallet_body, wallet_auth(&wallet_body).await),
            (&dek_body, dek_auth(&dek_body)),
        ] {
            assert_eq!(
                quota_status(&fleet.combiner, body, &authorization).await,
                (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER)),
                "{body}"
            );
        }
    }
    assert_eq!(fleet.dek.calls(), 0);
}

#[tokio::test]
async fn account_as_an_array_of_numbers_is_400_and_reaches_no_signer() {
    let answer = r#"{"success":true,"version":"1.0.0","performedQueryCount":0,"totalQuota":1000}"#;
    let mut signers = Vec::new();
    for _ in 0..3 {
        signers.push(fake_signer(StatusCode::OK, answer, Duration::ZERO).await);
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
        r#"{{"account":{}}}"#,
        serde_json::to_string(&numbers).unwrap()
    );

    assert_eq!(
        quota_status(&combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::BAD_REQUEST, failure(INVALID_INPUT))
    );
    assert_eq!(received(), [0, 0, 0]);

    // The same signers receive a valid request.
    let body = account_body();
    assert_eq!(
        quota_status(&combiner, &body, &wallet_auth(&body).await).await,
        (StatusCode::OK, quota_success(0, 1000, &[]))
    );
    assert_eq!(received(), [1, 1, 1]);
}

/// The signers check the signature over what they receive, so a 200 shows they received the
/// compact form.
#[tokio::test]
async fn compact_signed_body_sent_with_whitespace_passes() {
    let fleet = fleet().await;
    let wallet_sent = format!("{{\n  \"account\" : \"{ACCOUNT}\"\n}}");
    let dek_sent = format!(
        "{{ \"account\": \"{ACCOUNT}\",\r\n  \"authenticationMethod\": \"encryption_key\" }}"
    );

    for (sent, authorization) in [
        (wallet_sent, wallet_auth(&account_body()).await),
        (dek_sent, dek_auth(&dek_body())),
    ] {
        assert_eq!(
            quota_status(&fleet.combiner, &sent, &authorization).await,
            (StatusCode::OK, quota_success(0, 1000, &[])),
            "{sent}"
        );
    }
}

#[tokio::test]
async fn body_signed_with_whitespace_is_401() {
    let fleet = fleet().await;
    let wallet_body = format!("{{\n  \"account\" : \"{ACCOUNT}\"\n}}");
    let dek_body =
        format!("{{ \"account\": \"{ACCOUNT}\", \"authenticationMethod\": \"encryption_key\" }}");

    for (body, authorization) in [
        (&wallet_body, wallet_auth(&wallet_body).await),
        (&dek_body, dek_auth(&dek_body)),
    ] {
        assert_eq!(
            quota_status(&fleet.combiner, body, &authorization).await,
            (StatusCode::UNAUTHORIZED, failure(UNAUTHENTICATED_USER)),
            "{body}"
        );
    }
}

/// The SDK's bodies (`packages/identity/src/odis/quota.ts:36-41`). With a client version the
/// keys are `account`, `version`, `authenticationMethod`, `sessionID`, which is neither sorted
/// nor the order of a Rust struct. Without one, `JSON.stringify` leaves `version` out and the
/// remaining keys happen to be sorted.
#[tokio::test]
async fn sdk_quota_body_passes_with_wallet_key_and_dek() {
    let fleet = fleet().await;
    let mut bodies = Vec::new();
    for method in ["wallet_key", "encryption_key"] {
        bodies.push(format!(
            r#"{{"account":"{ACCOUNT}","version":"{CLIENT_VERSION}","authenticationMethod":"{method}","sessionID":"{SESSION_ID}"}}"#
        ));
        bodies.push(format!(
            r#"{{"account":"{ACCOUNT}","authenticationMethod":"{method}","sessionID":"{SESSION_ID}"}}"#
        ));
    }

    for body in &bodies {
        let authorization = if body.contains("encryption_key") {
            dek_auth(body)
        } else {
            wallet_auth(body).await
        };
        assert_eq!(
            quota_status(&fleet.combiner, body, &authorization).await,
            (StatusCode::OK, quota_success(0, 1000, &[])),
            "{body}"
        );
    }
}

/// The JSON failure arrives first. The two 502s come later, so the request ends early on the
/// second failure while the first is already counted. Only the first has a JSON body, so it is
/// the majority. A tie goes to the lower status, so only the 503 case would show a 502 that
/// entered the majority; the 403 case is the brief's.
#[tokio::test]
async fn html_and_empty_failures_do_not_enter_the_majority() {
    let late = Duration::from_millis(300);
    for (status, answer) in [
        (StatusCode::FORBIDDEN, OUT_OF_QUOTA_ANSWER),
        (StatusCode::SERVICE_UNAVAILABLE, DISABLED_SIGNER_ANSWER),
    ] {
        let (json, _) = fake_signer(status, answer, Duration::ZERO).await;
        let (html, _) = fake_signer(
            StatusCode::BAD_GATEWAY,
            "<html><body>502 Bad Gateway</body></html>",
            late,
        )
        .await;
        let (empty, _) = fake_signer(StatusCode::BAD_GATEWAY, "", late).await;
        let combiner = combiner_over(&[&json, &html, &empty]).await;
        let body = account_body();

        assert_eq!(
            quota_status(&combiner, &body, &wallet_auth(&body).await).await,
            (status, failure(THRESHOLD_PNP_QUOTA_STATUS_FAILURE))
        );
    }
}

#[tokio::test]
async fn a_panicking_request_answers_err_00_and_is_counted() {
    let fleet = fleet_of(
        (0..3).map(SignerSetup::share).collect(),
        FixedDek::panicking(),
    )
    .await;
    let series = [
        r#"combiner_endpoint_handler_errors_total{endpoint="/quotaStatus"}"#.to_string(),
        r#"combiner_unknown_errors_total{endpoint="/quotaStatus"}"#.to_string(),
        format!(r#"combiner_errors_total{{endpoint="/quotaStatus",error_type="{UNKNOWN_ERROR}"}}"#),
    ];
    let mut before = Vec::new();
    for s in &series {
        before.push(counter(&fleet.combiner, s).await);
    }
    let body = dek_body();

    assert_eq!(
        quota_status(&fleet.combiner, &body, &dek_auth(&body)).await,
        (StatusCode::INTERNAL_SERVER_ERROR, failure(UNKNOWN_ERROR))
    );
    assert_eq!(fleet.dek.calls(), 1);
    for (s, before) in series.iter().zip(before) {
        assert_eq!(counter(&fleet.combiner, s).await, before + 1, "{s}");
    }
}
