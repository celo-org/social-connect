use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{get, post};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::account_service::{
    AccountService, CachingAccountService, ClientAccountService, MeteredAccountService,
    MockAccountService,
};
use crate::config::{Config, KeystoreType};
use crate::errors::OdisError;
use crate::handlers::{pnp_quota_handler, pnp_sign_handler, status_handler};
use crate::key_management::{KeyProvider, MockKeyProvider};
use crate::metrics;
use crate::request_service::{
    MeteredPnpRequestService, PnpRequestService, SqlitePnpRequestService,
};

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub account_service: Arc<dyn AccountService>,
    pub request_service: Arc<dyn PnpRequestService>,
    pub key_provider: Arc<dyn KeyProvider>,
}

/// Build the axum router, constructing the appropriate AccountService from config.
///
/// When `blockchain_provider` is set, uses `ClientAccountService` for on-chain lookups.
/// Otherwise, uses `MockAccountService` — but only allowed with `KeystoreType::Mock`
/// to prevent accidentally running production keys without on-chain auth and quota.
pub async fn build_router(config: Config) -> Result<Router, OdisError> {
    let account_service: Arc<dyn AccountService> = if config.blockchain_provider.is_some() {
        let client = Arc::new(ClientAccountService::new(&config)?);
        let metered = Arc::new(MeteredAccountService::new(client));
        Arc::new(CachingAccountService::new(metered))
    } else {
        if config.keystore_type != KeystoreType::Mock {
            tracing::error!(
                "BLOCKCHAIN_PROVIDER is required when keystore_type is not Mock. \
                 Without it, authentication and quota are disabled."
            );
            return Err(OdisError::FullNodeError);
        }
        Arc::new(MockAccountService::new(
            config.mock_dek.clone(),
            config.mock_total_quota,
        ))
    };

    build_router_with_services(config, account_service).await
}

/// Build the axum router with an explicit AccountService.
/// Integration tests use this to inject a mock while still exercising real auth.
pub async fn build_router_with_services(
    config: Config,
    account_service: Arc<dyn AccountService>,
) -> Result<Router, OdisError> {
    let inner_request_service: Arc<dyn PnpRequestService> =
        Arc::new(SqlitePnpRequestService::new(&config.db_path).await?);
    let request_service: Arc<dyn PnpRequestService> =
        Arc::new(MeteredPnpRequestService::new(inner_request_service));

    let metrics_handle = metrics::install_recorder();

    let state = AppState {
        config: Arc::new(config),
        account_service,
        request_service,
        key_provider: Arc::new(MockKeyProvider::new()),
    };

    let timeout = Duration::from_millis(state.config.timeout_ms);

    Ok(Router::new()
        .route("/status", get(status_handler))
        .route("/sign", post(pnp_sign_handler))
        .route("/quotaStatus", post(pnp_quota_handler))
        .route(
            "/metrics",
            get(move || std::future::ready(metrics_handle.render())),
        )
        .layer(middleware::from_fn(metrics::http_metrics_layer))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(RequestBodyLimitLayer::new(16 * 1024)) // 16 KB, matches TS REASONABLE_BODY_CHAR_LIMIT
        .with_state(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_config(pnp_enabled: bool) -> Config {
        Config {
            server_port: 8080,
            pnp_api_enabled: pnp_enabled,
            keystore_type: KeystoreType::Mock,
            pnp_key_name_base: "phoneNumberPrivacy".to_string(),
            pnp_latest_key_version: 1,
            db_path: ":memory:".to_string(),
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
        }
    }

    #[tokio::test]
    async fn status_returns_version() {
        let app = build_router(test_config(false)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    const VALID_SIGN_BODY: &str = r#"{
        "account": "0x0000000000000000000000000000000000007E57",
        "blindedQueryPhoneNumber": "n/I9srniwEHm5o6t3y0tTUB5fn7xjxRrLP1F/i8ORCdqV++WWiaAzUo3GA2UNHiB"
    }"#;

    // Expected signature for key version 1 (from values.ts)
    const EXPECTED_SIG_V1: &str =
        "MAAAAAAAAACEVdw1ULDwAiTcZuPnZxHHh38PNa+/g997JgV10QnEq9yeuLxbM9l7vk0EAicV7IAAAAAA";

    #[tokio::test]
    async fn sign_returns_200_with_signature() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(VALID_SIGN_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("odis-key-version").unwrap(), "1");

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["signature"], EXPECTED_SIG_V1);
        assert_eq!(json["performedQueryCount"], 1);
        assert_eq!(json["totalQuota"], 10);
    }

    #[tokio::test]
    async fn sign_returns_400_for_invalid_blinded_query() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"account": "0x0000000000000000000000000000000000007E57", "blindedQueryPhoneNumber": "short"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_returns_400_for_invalid_key_version() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .header("odis-key-version", "abc")
                    .body(Body::from(VALID_SIGN_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_duplicate_returns_cached_signature() {
        let app = build_router(test_config(true)).await.unwrap();

        // First request
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(VALID_SIGN_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Duplicate request
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(VALID_SIGN_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["signature"], EXPECTED_SIG_V1);
        // Quota should not have increased
        assert_eq!(json["performedQueryCount"], 1);
        // Should have duplicate warning
        let warnings = json["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].as_str().unwrap().contains("CELO_ODIS_WARN_04"));
    }

    #[tokio::test]
    async fn sign_returns_403_when_quota_exceeded() {
        let mut config = test_config(true);
        config.mock_total_quota = 0;
        let app = build_router(config).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from(VALID_SIGN_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn quota_returns_200_with_quota_info() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/quotaStatus")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"account": "0x0000000000000000000000000000000000007E57"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["performedQueryCount"], 0);
        assert_eq!(json["totalQuota"], 10);
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn quota_returns_400_for_invalid_account() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/quotaStatus")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"account": "not-an-address"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn quota_returns_400_for_malformed_json() {
        let app = build_router(test_config(true)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/quotaStatus")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sign_returns_503_when_pnp_disabled() {
        let app = build_router(test_config(false)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sign")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn quota_returns_503_when_pnp_disabled() {
        let app = build_router(test_config(false)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/quotaStatus")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text() {
        let app = build_router(test_config(false)).await.unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        // Prometheus text format uses "# TYPE" and "# HELP" lines
        // At minimum, the output should be valid (possibly empty if no metrics recorded yet)
        assert!(
            text.is_empty() || text.contains("# TYPE") || text.contains("# HELP"),
            "expected Prometheus text format"
        );
    }

    #[tokio::test]
    async fn build_router_fails_for_private_key_keystore_without_blockchain_provider() {
        let config = Config {
            keystore_type: KeystoreType::PrivateKey,
            ..test_config(true)
        };

        let result = build_router(config).await;
        assert!(result.is_err());
    }
}
