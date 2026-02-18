use std::sync::Arc;

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use k256::ecdsa::{SigningKey, signature::Signer as _};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;

use odis_signer::account_service::MockAccountService;
use odis_signer::config::{Config, KeystoreType};
use odis_signer::key_management::MockKeyProvider;
use odis_signer::server::{build_router, build_router_with_services};

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
    test_config_with_db(":memory:")
}

fn test_config_with_db(db_path: &str) -> Config {
    Config {
        server_port: 8080,
        pnp_api_enabled: true,
        keystore_type: KeystoreType::Mock,
        pnp_key_name_base: "phoneNumberPrivacy".to_string(),
        pnp_latest_key_version: 1,
        db_path: db_path.to_string(),
        blockchain_provider: None,
        chain_id: 44787,
        mock_dek: None,
        mock_total_quota: 10,
        accounts_contract_address: None,
        odis_payments_contract_address: None,
        full_node_retry_count: 5,
        full_node_retry_delay_ms: 100,
        timeout_ms: 5000,
        query_price_per_cusd: 0.001,
        google_project_id: None,
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
        let app = build_router(test_config()).await.unwrap();

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
    let app = build_router(test_config()).await.unwrap();
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
    let app = build_router(test_config()).await.unwrap();
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
    let app = build_router(config).await.unwrap();

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
    let app = build_router(test_config()).await.unwrap();
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
    let app = build_router(test_config()).await.unwrap();

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
    let app = build_router(test_config()).await.unwrap();

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
    let app = build_router(config).await.unwrap();

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

#[tokio::test]
async fn sqlite_sign_and_quota_full_stack() {
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_str().unwrap();
    let config = test_config_with_db(db_path);
    let app = build_router(config).await.unwrap();

    // Initially zero quota
    let response = app.clone().oneshot(quota_request(ACCOUNT)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 0);

    // Sign a request
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
    assert_eq!(json["performedQueryCount"], 1);

    // Duplicate returns cached signature without incrementing quota
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
    assert_eq!(json["performedQueryCount"], 1);
    assert!(
        json["warnings"][0]
            .as_str()
            .unwrap()
            .contains("CELO_ODIS_WARN_04")
    );

    // Quota reflects the sign
    let response = app.oneshot(quota_request(ACCOUNT)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 1);
    assert_eq!(json["totalQuota"], 10);
}

// --- Authentication-enabled tests ---

/// Build a router with auth enforcement enabled.
/// Sets a dummy blockchain_provider to enable auth, but uses MockAccountService
/// with empty DEK so only wallet key auth works.
async fn build_auth_router() -> Router {
    let config = Config {
        blockchain_provider: Some("http://localhost:8545".to_string()),
        ..test_config()
    };
    let account_service = Arc::new(MockAccountService::new(None, 10));
    build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap()
}

fn sign_request_with_auth(body: &str, auth: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sign")
        .header("content-type", "application/json")
        .header("Authorization", auth)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn quota_request_with_auth(body: &str, auth: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/quotaStatus")
        .header("content-type", "application/json")
        .header("Authorization", auth)
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Sign a body with a wallet key and return the hex-encoded signature.
async fn wallet_sign(signer: &PrivateKeySigner, body: &str) -> String {
    let sig = signer.sign_message(body.as_bytes()).await.unwrap();
    hex::encode(sig.as_bytes())
}

#[tokio::test]
async fn sign_returns_401_without_authorization_header() {
    let app = build_auth_router().await;
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    let response = app.oneshot(sign_request(&body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = response_json(response).await;
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("CELO_ODIS_WARN_02")
    );
}

#[tokio::test]
async fn quota_returns_401_without_authorization_header() {
    let app = build_auth_router().await;

    let response = app.oneshot(quota_request(ACCOUNT)).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sign_succeeds_with_valid_wallet_key_signature() {
    let signer = PrivateKeySigner::random();
    let body = sign_body(&format!("{}", signer.address()), BLINDED_PHONE_NUMBER);
    let sig_hex = wallet_sign(&signer, &body).await;

    let app = build_auth_router().await;
    let response = app
        .oneshot(sign_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["success"], true);
}

#[tokio::test]
async fn sign_returns_401_with_wrong_wallet_key() {
    let signer = PrivateKeySigner::random();
    let wrong_signer = PrivateKeySigner::random();

    // Sign with wrong_signer but claim to be signer
    let body = sign_body(&format!("{}", signer.address()), BLINDED_PHONE_NUMBER);
    let sig_hex = wallet_sign(&wrong_signer, &body).await;

    let app = build_auth_router().await;
    let response = app
        .oneshot(sign_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn quota_succeeds_with_valid_wallet_key_signature() {
    let signer = PrivateKeySigner::random();
    let body = serde_json::json!({ "account": format!("{}", signer.address()) }).to_string();
    let sig_hex = wallet_sign(&signer, &body).await;

    let app = build_auth_router().await;
    let response = app
        .oneshot(quota_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["success"], true);
}

// --- DEK authentication tests ---

/// Produce a DEK signature matching the TS `signWithRawKey` function.
/// Returns a JSON-encoded DER byte array for the Authorization header.
fn dek_sign(body: &str, signing_key: &SigningKey) -> String {
    let double_stringified = serde_json::to_string(body).unwrap();
    let digest = Sha256::digest(double_stringified.as_bytes());
    let digest_hex = hex::encode(digest);
    let sig: k256::ecdsa::Signature = signing_key.sign(digest_hex.as_bytes());
    serde_json::to_string(&sig.to_der().as_bytes()).unwrap()
}

fn dek_public_key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_sec1_bytes())
}

/// Build a router with auth enabled and a DEK registered for the account service.
async fn build_dek_auth_router(dek_public_key: &str) -> Router {
    let config = Config {
        blockchain_provider: Some("http://localhost:8545".to_string()),
        ..test_config()
    };
    let account_service = Arc::new(MockAccountService::new(
        Some(dek_public_key.to_string()),
        10,
    ));
    build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap()
}

#[tokio::test]
async fn sign_succeeds_with_valid_dek_signature() {
    let dek_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let dek_pub = dek_public_key_hex(&dek_key);

    let app = build_dek_auth_router(&dek_pub).await;

    let body = serde_json::json!({
        "account": ACCOUNT,
        "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER,
        "authenticationMethod": "encryption_key",
    })
    .to_string();
    let auth = dek_sign(&body, &dek_key);

    let response = app
        .oneshot(sign_request_with_auth(&body, &auth))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
}

#[tokio::test]
async fn sign_returns_401_with_wrong_dek() {
    let dek_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let wrong_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let dek_pub = dek_public_key_hex(&dek_key);

    let app = build_dek_auth_router(&dek_pub).await;

    // Sign with wrong_key but account service has dek_key registered
    let body = serde_json::json!({
        "account": ACCOUNT,
        "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER,
        "authenticationMethod": "encryption_key",
    })
    .to_string();
    let auth = dek_sign(&body, &wrong_key);

    let response = app
        .oneshot(sign_request_with_auth(&body, &auth))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn quota_succeeds_with_valid_dek_signature() {
    let dek_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let dek_pub = dek_public_key_hex(&dek_key);

    let app = build_dek_auth_router(&dek_pub).await;

    let body = serde_json::json!({
        "account": ACCOUNT,
        "authenticationMethod": "encryption_key",
    })
    .to_string();
    let auth = dek_sign(&body, &dek_key);

    let response = app
        .oneshot(quota_request_with_auth(&body, &auth))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["success"], true);
}

#[tokio::test]
async fn quota_returns_401_with_wrong_dek() {
    let dek_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let wrong_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let dek_pub = dek_public_key_hex(&dek_key);

    let app = build_dek_auth_router(&dek_pub).await;

    let body = serde_json::json!({
        "account": ACCOUNT,
        "authenticationMethod": "encryption_key",
    })
    .to_string();
    let auth = dek_sign(&body, &wrong_key);

    let response = app
        .oneshot(quota_request_with_auth(&body, &auth))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// --- Per-account quota tests ---

#[tokio::test]
async fn sign_returns_403_when_account_has_zero_quota() {
    let signer = PrivateKeySigner::random();
    let config = Config {
        blockchain_provider: Some("http://localhost:8545".to_string()),
        ..test_config()
    };
    let account_service = Arc::new(MockAccountService::new(None, 0));
    let app = build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap();

    let body = sign_body(&format!("{}", signer.address()), BLINDED_PHONE_NUMBER);
    let sig_hex = wallet_sign(&signer, &body).await;

    let response = app
        .oneshot(sign_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn quota_reflects_account_service_total_quota() {
    let signer = PrivateKeySigner::random();
    let config = Config {
        blockchain_provider: Some("http://localhost:8545".to_string()),
        ..test_config()
    };
    let account_service = Arc::new(MockAccountService::new(None, 42));
    let app = build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap();

    let body = serde_json::json!({ "account": format!("{}", signer.address()) }).to_string();
    let sig_hex = wallet_sign(&signer, &body).await;

    let response = app
        .oneshot(quota_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["totalQuota"], 42);
}
