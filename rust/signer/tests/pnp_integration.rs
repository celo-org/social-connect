use std::sync::Arc;
use std::time::Duration;

use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tower::ServiceExt;

use odis_signer::account_service::MockAccountService;
use odis_signer::config::{Config, DatabaseConfig, KeystoreType, LogFormat};
use odis_signer::errors::OdisError;
use odis_signer::key_management::{KeyProvider, MockKeyProvider};
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
        database: DatabaseConfig::Sqlite {
            path: db_path.to_string(),
        },
        migrate_legacy_data: false,
        blockchain_provider: None,
        blockchain_api_key: None,
        chain_id: 11142220,
        accounts_contract_address: None,
        odis_payments_contract_address: None,
        full_node_retry_count: 5,
        full_node_retry_delay_ms: 100,
        full_node_timeout_ms: 1000,
        timeout_ms: 5000,
        query_price_per_cusd: 0.001,
        google_project_id: None,
        request_pruning_days: 7,
        request_pruning_interval_secs: 86400,
        log_format: LogFormat::Text,
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
    assert_eq!(json["warnings"], serde_json::json!([]));

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
    let config = test_config();
    let account_service = Arc::new(MockAccountService::new(None, 1));
    let app = build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap();

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
    assert_eq!(
        response_json(response).await,
        serde_json::json!({
            "success": false,
            "version": env!("CARGO_PKG_VERSION"),
            "error": "CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota",
            "performedQueryCount": 1,
            "totalQuota": 1,
        })
    );
}

fn post_request(uri: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Asserts the whole error body, as the combiner and clients parse it.
async fn assert_error_body(response: axum::response::Response, status: StatusCode, error: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(
        response_json(response).await,
        serde_json::json!({
            "success": false,
            "version": env!("CARGO_PKG_VERSION"),
            "error": error,
        })
    );
}

#[tokio::test]
async fn request_validation_matches_the_ts_signer() {
    const ENDPOINTS: [&str; 2] = ["/sign", "/quotaStatus"];
    let app = build_router(test_config()).await.unwrap();

    let invalid_accounts = [
        serde_json::json!({ "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER }),
        serde_json::json!({ "account": "", "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER }),
        serde_json::json!({
            "account": "0xnotanaddress",
            "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER,
        }),
    ];
    for body in &invalid_accounts {
        for uri in ENDPOINTS {
            let response = app.clone().oneshot(post_request(uri, body)).await.unwrap();
            assert_error_body(
                response,
                StatusCode::BAD_REQUEST,
                "CELO_ODIS_WARN_01 BAD_INPUT Invalid input parameters",
            )
            .await;
        }
    }

    // A field the signer does not know is ignored.
    let extra_field = serde_json::json!({
        "account": ACCOUNT,
        "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER,
        "extraField": "ignored",
    });
    for uri in ENDPOINTS {
        let response = app
            .clone()
            .oneshot(post_request(uri, &extra_field))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }

    // With the API disabled, a valid request is refused too.
    let mut config = test_config();
    config.pnp_api_enabled = false;
    let disabled = build_router(config).await.unwrap();
    for uri in ENDPOINTS {
        let response = disabled
            .clone()
            .oneshot(post_request(uri, &extra_field))
            .await
            .unwrap();
        assert_error_body(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "CELO_ODIS_WARN_13 BAD_INPUT API is unavailable",
        )
        .await;
    }
}

/// Asserts the body the TS signer sends when it cannot fetch the key or compute
/// the signature: `ERR_05` with the account's quota.
async fn assert_signing_failure(response: axum::response::Response, performed_query_count: u64) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(
        json["error"],
        "CELO_ODIS_ERR_05 SIG_ERR Failed to compute BLS signature"
    );
    assert_eq!(json["performedQueryCount"], performed_query_count);
    assert_eq!(json["totalQuota"], 10);
}

async fn assert_used_quota(app: &axum::Router, expected: u64) {
    let response = app.clone().oneshot(quota_request(ACCOUNT)).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], expected);
}

#[tokio::test]
async fn sign_returns_500_for_unsupported_key_version() {
    let app = build_router(test_config()).await.unwrap();
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    let response = app
        .clone()
        .oneshot(sign_request_with_key_version(&body, "99"))
        .await
        .unwrap();

    assert_signing_failure(response, 0).await;
    assert_used_quota(&app, 0).await;
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

    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_signing_failure(response, 0).await;

    // The body reports the quota used so far, and the failure adds nothing to it.
    let valid = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let response = app.clone().oneshot(sign_request(&valid)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_signing_failure(response, 1).await;
    assert_used_quota(&app, 1).await;
}

#[tokio::test]
async fn quota_returns_200_even_when_over_quota() {
    let config = test_config();
    let account_service = Arc::new(MockAccountService::new(None, 1));
    let app = build_router_with_services(config, account_service, Arc::new(MockKeyProvider::new()))
        .await
        .unwrap();

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

/// Runs a statement on the signer's database from a second connection, to make
/// its later queries fail.
async fn break_database(db_path: &str, statement: &str) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{db_path}"))
        .await
        .unwrap();
    sqlx::query(statement).execute(&pool).await.unwrap();
    pool.close().await;
}

async fn assert_server_error(response: axum::response::Response, error: &str) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["success"], false);
    assert_eq!(json["error"], error);
}

/// Each database failure answers with the error string the TS signer sends for
/// it. A failed duplicate lookup refuses the request, where the TS signer
/// carries on as if the request were new.
#[tokio::test]
async fn database_failures_return_the_ts_error_strings() {
    const INSERT_FAILURE: &str = "CELO_ODIS_ERR_02 DB_ERR Failed to insert database entry";
    const GET_FAILURE: &str = "CELO_ODIS_ERR_03 DB_ERR Failed to get database entry";
    const QUOTA_READ_FAILURE: &str =
        "CELO_ODIS_ERR_24 DB_ERR Failed to read performedQueryCount from signer db";

    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_str().unwrap();
    let app = build_router(test_config_with_db(db_path)).await.unwrap();
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    // Recording the request fails: nothing is charged.
    break_database(
        db_path,
        "CREATE TRIGGER refuse_requests BEFORE INSERT ON requests \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .await;
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_server_error(response, INSERT_FAILURE).await;
    assert_used_quota(&app, 0).await;

    // The used-quota read fails on both endpoints.
    break_database(db_path, "ALTER TABLE accounts RENAME TO accounts_moved").await;
    let response = app.clone().oneshot(quota_request(ACCOUNT)).await.unwrap();
    assert_server_error(response, QUOTA_READ_FAILURE).await;
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_server_error(response, QUOTA_READ_FAILURE).await;

    // The duplicate lookup fails.
    break_database(db_path, "DROP TRIGGER refuse_requests").await;
    break_database(db_path, "ALTER TABLE requests RENAME TO requests_moved").await;
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_server_error(response, GET_FAILURE).await;
}

/// Concurrent retries of the same request all clear the duplicate pre-check,
/// so they collide on the write. Each must still be served the signature and
/// charged once between them, rather than failing on the primary key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_identical_sign_requests_are_all_served_and_charged_once() {
    let db_file = NamedTempFile::new().unwrap();
    let db_path = db_file.path().to_str().unwrap();
    let app = build_router(test_config_with_db(db_path)).await.unwrap();

    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let app = app.clone();
            let body = body.clone();
            tokio::spawn(async move { app.oneshot(sign_request(&body)).await.unwrap() })
        })
        .collect();

    for handle in handles {
        let response = handle.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["signature"], EXPECTED_SIG_V1);
        assert_eq!(json["performedQueryCount"], 1);
    }

    let response = app.oneshot(quota_request(ACCOUNT)).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["performedQueryCount"], 1, "quota charged exactly once");
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

/// Sign a body with a wallet key and return the signature as clients send it:
/// hex with a `0x` prefix.
async fn wallet_sign(signer: &PrivateKeySigner, body: &str) -> String {
    let sig = signer.sign_message(body.as_bytes()).await.unwrap();
    format!("0x{}", hex::encode(sig.as_bytes()))
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

/// The TS signer verifies by wallet key whenever the method is not exactly
/// `encryption_key`, so an unknown method needs the account's own signature.
#[tokio::test]
async fn unknown_authentication_method_is_verified_by_wallet_key() {
    let signer = PrivateKeySigner::random();
    let other = PrivateKeySigner::random();
    let app = build_auth_router().await;

    for method in ["WALLET_KEY", "ENCRYPTION_KEY", "foo"] {
        let body = serde_json::json!({
            "account": format!("{}", signer.address()),
            "blindedQueryPhoneNumber": BLINDED_PHONE_NUMBER,
            "authenticationMethod": method,
        })
        .to_string();

        let response = app
            .clone()
            .oneshot(sign_request_with_auth(
                &body,
                &wallet_sign(&signer, &body).await,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method}");

        let response = app
            .clone()
            .oneshot(quota_request_with_auth(
                &body,
                &wallet_sign(&signer, &body).await,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method}");

        let response = app
            .clone()
            .oneshot(sign_request_with_auth(
                &body,
                &wallet_sign(&other, &body).await,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
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
async fn quota_returns_401_with_wrong_wallet_key() {
    let signer = PrivateKeySigner::random();
    let wrong_signer = PrivateKeySigner::random();

    let body = serde_json::json!({ "account": format!("{}", signer.address()) }).to_string();
    let sig_hex = wallet_sign(&wrong_signer, &body).await;

    let app = build_auth_router().await;
    let response = app
        .oneshot(quota_request_with_auth(&body, &sig_hex))
        .await
        .unwrap();

    assert_error_body(
        response,
        StatusCode::UNAUTHORIZED,
        "CELO_ODIS_WARN_02 BAD_INPUT Missing or invalid authentication",
    )
    .await;
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
    let sig: k256::ecdsa::Signature = signing_key.sign_prehash(&digest).unwrap();
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

// ── Aborted and timed-out requests ───────────────────────────────────

/// Key provider that answers after a delay, which keeps a /sign request in
/// flight long enough for the caller to give up or the signer to time out.
struct SlowKeyProvider {
    inner: MockKeyProvider,
    delay: Duration,
}

#[async_trait::async_trait]
impl KeyProvider for SlowKeyProvider {
    async fn get_key(&self, name: &str, version: u32) -> Result<String, OdisError> {
        tokio::time::sleep(self.delay).await;
        self.inner.get_key(name, version).await
    }
}

async fn build_slow_router(key_delay: Duration, signer_timeout: Duration) -> Router {
    let config = Config {
        timeout_ms: signer_timeout.as_millis() as u64,
        ..test_config()
    };
    let key_provider = Arc::new(SlowKeyProvider {
        inner: MockKeyProvider::new(),
        delay: key_delay,
    });
    build_router_with_services(
        config,
        Arc::new(MockAccountService::new(None, 10)),
        key_provider,
    )
    .await
    .unwrap()
}

/// Polls /quotaStatus until the account's used quota reaches `expected`.
async fn wait_for_used_quota(app: &Router, expected: u64) {
    for _ in 0..100 {
        let response = app.clone().oneshot(quota_request(ACCOUNT)).await.unwrap();
        if response_json(response).await["performedQueryCount"] == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("used quota never reached {expected}");
}

async fn metrics_text(app: &Router) -> String {
    let request = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(body.to_vec()).unwrap()
}

/// The combiner drops its connection to the slower signers once it has enough
/// partial signatures. Like the TS signer, the request must still be finished
/// and charged, so that all signers keep the same count for the account.
#[tokio::test]
async fn sign_is_finished_and_charged_when_the_caller_gives_up() {
    let app = build_slow_router(Duration::from_millis(200), Duration::from_secs(5)).await;
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    // Dropping the response future is what the server does to a handler when
    // its connection closes.
    let abandoned = tokio::time::timeout(
        Duration::from_millis(50),
        app.clone().oneshot(sign_request(&body)),
    )
    .await;
    assert!(abandoned.is_err(), "the caller must give up first");

    wait_for_used_quota(&app, 1).await;

    // The retry is a replay: same signature, no second charge.
    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature"], EXPECTED_SIG_V1);
    assert_eq!(json["performedQueryCount"], 1);
    assert_eq!(
        json["warnings"][0],
        "CELO_ODIS_WARN_04 BAD_INPUT Attempt to replay partial signature request"
    );
}

#[tokio::test]
async fn sign_timeout_returns_the_odis_error_and_is_counted_and_charged() {
    let app = build_slow_router(Duration::from_millis(300), Duration::from_millis(50)).await;
    let body = sign_body(ACCOUNT, BLINDED_PHONE_NUMBER);

    let response = app.clone().oneshot(sign_request(&body)).await.unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["success"], false);
    assert_eq!(
        json["error"],
        "CELO_ODIS_ERR_10 SIG_ERR Timeout from signer"
    );
    assert!(json["version"].is_string());

    let metrics = metrics_text(&app).await;
    assert!(
        metrics.lines().any(|l| l.starts_with("timeouts ")),
        "timeouts counter missing in:\n{metrics}"
    );
    assert!(
        metrics.contains(r#"responses{endpoint="/sign",status_code="500"}"#),
        "timed-out response not counted in:\n{metrics}"
    );

    // The work goes on after the 500, as in the TS signer.
    wait_for_used_quota(&app, 1).await;
}
