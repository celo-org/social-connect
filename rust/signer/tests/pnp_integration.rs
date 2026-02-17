use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use odis_signer::config::{Config, KeystoreType};
use odis_signer::server::build_router;

const BLINDED_PHONE_NUMBER: &str =
    "n/I9srniwEHm5o6t3y0tTUB5fn7xjxRrLP1F/i8ORCdqV++WWiaAzUo3GA2UNHiB";

const ACCOUNT: &str = "0x0000000000000000000000000000000000007E57";

// Expected signatures for each key version (from values.ts)
const EXPECTED_SIG_V1: &str =
    "MAAAAAAAAACEVdw1ULDwAiTcZuPnZxHHh38PNa+/g997JgV10QnEq9yeuLxbM9l7vk0EAicV7IAAAAAA";
const EXPECTED_SIG_V2: &str =
    "MAAAAAAAAAAmUJY0s9p7fMfs7GIoSiGJoObAN8ZpA7kRqeC9j/Q23TBrG3Jtxc8xWibhNVZhbYEAAAAA";
const EXPECTED_SIG_V3: &str =
    "MAAAAAAAAAC4aBbzhHvt6l/b+8F7cILmWxZZ5Q7S6R4RZ/IgZR7Pfb9B1Wg9fsDybgxVTSv5BYEAAAAA";

fn test_config() -> Config {
    Config {
        server_port: 8080,
        pnp_api_enabled: true,
        keystore_type: KeystoreType::Mock,
        pnp_key_name_base: "phoneNumberPrivacy".to_string(),
        pnp_latest_key_version: 1,
        db_path: ":memory:".to_string(),
        blockchain_provider: None,
        chain_id: 44787,
        should_mock_account_service: true,
        mock_dek: None,
        mock_total_quota: 10,
        timeout_ms: 5000,
        query_price_per_cusd: 0.001,
    }
}

fn sign_body(account: &str, blinded_query: &str) -> String {
    serde_json::json!({
        "account": account,
        "blindedQueryPhoneNumber": blinded_query,
    })
    .to_string()
}

fn sign_request(body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sign")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn sign_request_with_key_version(body: &str, version: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sign")
        .header("content-type", "application/json")
        .header("odis-key-version", version)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn quota_request(account: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/quotaStatus")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "account": account }).to_string(),
        ))
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn sign_returns_correct_signature_for_all_key_versions() {
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let expected = [
        ("1", EXPECTED_SIG_V1),
        ("2", EXPECTED_SIG_V2),
        ("3", EXPECTED_SIG_V3),
    ];

    for (version, expected_sig) in expected {
        let app = build_router(test_config());

        let response = app
            .oneshot(sign_request_with_key_version(&body, version))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "version {version}");
        assert_eq!(
            response.headers().get("odis-key-version").unwrap(),
            version,
            "response header should echo key version {version}"
        );

        let json = response_json(response).await;
        assert_eq!(json["success"], true);
        assert_eq!(json["signature"], expected_sig, "version {version}");
        assert_eq!(json["performedQueryCount"], 1);
    }
}

#[tokio::test]
async fn sign_uses_default_key_version_when_header_absent() {
    let app = build_router(test_config());
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    let response = app.oneshot(sign_request(&body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // Default key version is 1
    assert_eq!(response.headers().get("odis-key-version").unwrap(), "1");

    let json = response_json(response).await;
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
}

#[tokio::test]
async fn sign_duplicate_returns_cached_signature_without_incrementing_quota() {
    let app = build_router(test_config());
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    // First request
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 1);
    assert!(json.get("warnings").is_none());

    // Duplicate request
    let response = app.oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response).await;
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
    assert_eq!(json["performedQueryCount"], 1, "quota should not increment");

    let warnings = json["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("CELO_ODIS_WARN_04"));
}

#[tokio::test]
async fn sign_returns_403_when_quota_exactly_depleted() {
    let mut config = test_config();
    config.mock_total_quota = 1;
    let app = build_router(config);

    let body1 = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    // First request uses the one available quota unit
    let response = app.clone().oneshot(sign_request(&body1)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Second request with a different query should be rejected
    let body2 = sign_body(
        ACCOUNT,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );
    let response = app.oneshot(sign_request(&body2)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sign_returns_500_for_unsupported_key_version() {
    let app = build_router(test_config());
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    let response = app
        .oneshot(sign_request_with_key_version(&body, "99"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = response_json(response).await;
    assert!(json["error"].as_str().unwrap().contains("CELO_ODIS_ERR_04"));
}

#[tokio::test]
async fn sign_then_quota_reflects_updated_count() {
    let app = build_router(test_config());

    // Initially quota is 0
    let response = app.clone().oneshot(quota_request(ACCOUNT)).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 0);

    // Sign a request
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Quota should now reflect the sign
    let response = app.oneshot(quota_request(ACCOUNT)).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 1);
    assert_eq!(json["totalQuota"], 10);
}

#[tokio::test]
async fn sign_returns_500_for_invalid_blinded_message() {
    let app = build_router(test_config());

    // Valid base64, 64 chars, but not a valid BLS G1 point
    let body = sign_body(
        ACCOUNT,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    );

    let response = app.oneshot(sign_request(&body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let json = response_json(response).await;
    assert!(json["error"].as_str().unwrap().contains("CELO_ODIS_ERR_05"));
}

#[tokio::test]
async fn quota_returns_200_even_when_over_quota() {
    let mut config = test_config();
    config.mock_total_quota = 1;
    let app = build_router(config);

    // Sign once to use up quota
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Quota endpoint still returns 200 even though performedQueryCount >= totalQuota
    let response = app.oneshot(quota_request(ACCOUNT)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 1);
    assert_eq!(json["totalQuota"], 1);
    assert_eq!(json["success"], true);
}
