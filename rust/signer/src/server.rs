use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::handlers::{pnp_quota_handler, pnp_sign_stub, status_handler};
use crate::request_service::{InMemoryPnpRequestService, PnpRequestService};

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub request_service: Arc<dyn PnpRequestService>,
}

/// Build the axum router with all routes and middleware.
pub fn build_router(config: Config) -> Router {
    let state = AppState {
        config: Arc::new(config),
        request_service: Arc::new(InMemoryPnpRequestService::new()),
    };

    let timeout = Duration::from_millis(state.config.timeout_ms);

    Router::new()
        .route("/status", get(status_handler))
        .route("/sign", post(pnp_sign_stub))
        .route("/quotaStatus", post(pnp_quota_handler))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(RequestBodyLimitLayer::new(16 * 1024)) // 16 KB, matches TS REASONABLE_BODY_CHAR_LIMIT
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::config::KeystoreType;

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
            should_mock_account_service: true,
            mock_dek: None,
            mock_total_quota: 10,
            timeout_ms: 5000,
            query_price_per_cusd: 0.001,
        }
    }

    #[tokio::test]
    async fn status_returns_version() {
        let app = build_router(test_config(false));

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

    #[tokio::test]
    async fn sign_stub_returns_501_when_enabled() {
        let app = build_router(test_config(true));

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

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn quota_returns_200_with_quota_info() {
        let app = build_router(test_config(true));

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
        let app = build_router(test_config(true));

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
        let app = build_router(test_config(true));

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
        let app = build_router(test_config(false));

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
        let app = build_router(test_config(false));

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
}
