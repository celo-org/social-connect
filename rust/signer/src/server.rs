use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use ::metrics::counter;
use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::account_service::{
    AccountService, CachingAccountService, ClientAccountService, MeteredAccountService,
    MockAccountService,
};
use crate::config::{Config, DatabaseConfig, KeystoreType};
use crate::errors::OdisError;
use crate::handlers::{
    domain_disable_handler, domain_quota_handler, domain_sign_handler, pnp_quota_handler,
    pnp_sign_handler, status_handler,
};
use crate::key_management::{GoogleSecretManagerKeyProvider, KeyProvider, MockKeyProvider};
use crate::metrics;
use crate::request_service::{
    MeteredPnpRequestService, PnpRequestService, PostgresPnpRequestService, SqlitePnpRequestService,
};

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub account_service: Arc<dyn AccountService>,
    pub request_service: Arc<dyn PnpRequestService>,
    pub key_provider: Arc<dyn KeyProvider>,
}

/// Largest request body read from the network, matching the TS signer's
/// `express.json({ limit: '0.2mb' })`. The handlers apply the much lower limit
/// on a request's length, so that a body over that limit gets the ODIS error
/// response and is counted in the metrics.
const TRANSPORT_BODY_LIMIT_BYTES: usize = 209_715;

/// Build the axum router, constructing the appropriate AccountService from config.
///
/// When `blockchain_provider` is set, uses `ClientAccountService` for on-chain lookups.
/// Otherwise, uses `MockAccountService` — but only allowed with `KeystoreType::Mock`
/// to prevent accidentally running production keys without on-chain auth and quota.
pub async fn build_router(config: Config) -> Result<Router, OdisError> {
    if config.keystore_type != KeystoreType::Mock
        && matches!(&config.database, DatabaseConfig::Sqlite { path } if path == ":memory:")
    {
        tracing::error!(
            "a persistent database is required when keystore_type is not Mock; \
             set DATABASE_URL. An in-memory database forgets all used quota on restart."
        );
        return Err(OdisError::DatabaseError);
    }

    let account_service: Arc<dyn AccountService> = if config.blockchain_provider.is_some() {
        let client = Arc::new(ClientAccountService::new(&config)?);
        client.verify_chain_id().await?;
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
        tracing::warn!(
            "BLOCKCHAIN_PROVIDER is not set: requests are not authenticated and every account \
             gets the mock quota. This mode is for development with the mock key store only."
        );
        let mock_dek = std::env::var("MOCK_DEK").ok();
        let mock_total_quota = std::env::var("MOCK_TOTAL_QUOTA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        Arc::new(MockAccountService::new(mock_dek, mock_total_quota))
    };

    let key_provider: Arc<dyn KeyProvider> = match config.keystore_type {
        KeystoreType::Mock => Arc::new(MockKeyProvider::new()),
        KeystoreType::GoogleSecretManager => {
            let project_id = config.google_project_id.as_deref().ok_or_else(|| {
                tracing::error!(
                    "KEYSTORE_GOOGLE_PROJECT_ID is required when keystore_type is GoogleSecretManager"
                );
                OdisError::KeyFetchError
            })?;
            Arc::new(GoogleSecretManagerKeyProvider::new(project_id).await?)
        }
    };

    // Fetch the latest key now, as the TS signer does for every key store, so
    // that a wrong key name or version stops the start. Otherwise the signer
    // would look healthy and fail every sign request.
    key_provider
        .get_key(&config.pnp_key_name_base, config.pnp_latest_key_version)
        .await
        .inspect_err(|_| {
            tracing::error!(
                "the key store has no key {}-{}; check PHONE_NUMBER_PRIVACY_KEY_NAME_BASE and \
                 PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION",
                config.pnp_key_name_base,
                config.pnp_latest_key_version
            );
        })?;

    build_router_with_services(config, account_service, key_provider).await
}

/// Build the axum router with explicit services.
/// Integration tests use this to inject mocks while still exercising real auth.
pub async fn build_router_with_services(
    config: Config,
    account_service: Arc<dyn AccountService>,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Router, OdisError> {
    let db_timeout = Duration::from_millis(config.db_timeout_ms);
    let inner_request_service: Arc<dyn PnpRequestService> = match &config.database {
        DatabaseConfig::Sqlite { path } => {
            Arc::new(SqlitePnpRequestService::new(path, db_timeout).await?)
        }
        DatabaseConfig::Postgres { url } => {
            let service = PostgresPnpRequestService::new(url, db_timeout).await?;
            if config.migrate_legacy_data {
                service.run_legacy_migration().await?;
            } else {
                service.ensure_legacy_data_imported().await?;
            }
            Arc::new(service)
        }
    };
    let request_service: Arc<dyn PnpRequestService> =
        Arc::new(MeteredPnpRequestService::new(inner_request_service));

    let metrics_handle = metrics::install_recorder();
    metrics::spawn_upkeep_task(metrics_handle.clone());

    let state = AppState {
        config: Arc::new(config),
        account_service,
        request_service,
        key_provider,
    };

    spawn_pruning_task(state.config.clone(), state.request_service.clone());

    Ok(Router::new()
        .route("/status", get(status_handler))
        .route("/sign", post(pnp_sign_handler))
        .route("/quotaStatus", post(pnp_quota_handler))
        .route("/domain/sign", post(domain_sign_handler))
        .route("/domain/quotaStatus", post(domain_quota_handler))
        .route("/domain/disable", post(domain_disable_handler))
        .route(
            "/metrics",
            get(move || std::future::ready(metrics_handle.render())),
        )
        // Layers run from the bottom up, so the panic and timeout layers sit
        // inside the metrics layer, and a request that panics or times out is
        // counted like any other response.
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            request_timeout_layer,
        ))
        .layer(middleware::from_fn(metrics::http_metrics_layer))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(TRANSPORT_BODY_LIMIT_BYTES))
        .with_state(state))
}

/// Answers a handler panic with the ODIS error for an unknown failure, as the
/// TS signer answers an error that escapes its handlers.
fn panic_response(panic: Box<dyn Any + Send + 'static>) -> Response {
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("no message");
    tracing::error!(message, "request handler panicked");
    counter!(metrics::ERRORS_CAUGHT_IN_ENDPOINT_HANDLER).increment(1);
    OdisError::Unknown.into_response()
}

/// Answers with the ODIS timeout error when a request takes longer than
/// `ODIS_SIGNER_TIMEOUT`, as the TS signer does. Only the wait ends here: a
/// /sign request that is already under way finishes in its own task.
async fn request_timeout_layer(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let limit = Duration::from_millis(state.config.timeout_ms);
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            counter!(metrics::TIMEOUTS).increment(1);
            OdisError::Timeout.into_response()
        }
    }
}

fn spawn_pruning_task(config: Arc<Config>, request_service: Arc<dyn PnpRequestService>) {
    let days = config.request_pruning_days;
    let interval_secs = config.request_pruning_interval_secs;

    if interval_secs == 0 {
        tracing::info!("request pruning disabled (interval_secs = 0)");
        return;
    }

    tracing::info!(
        days,
        interval_secs,
        "request pruning enabled, first run starting now"
    );

    tokio::spawn(async move {
        loop {
            tracing::debug!(days, "starting request pruning cycle");
            match request_service.delete_old_requests(days).await {
                Ok(deleted) if deleted > 0 => {
                    tracing::info!(deleted, days, "pruned old PNP requests");
                }
                Ok(_) => {
                    tracing::debug!(days, "no old PNP requests to prune");
                }
                Err(e) => {
                    tracing::error!(?e, "failed to prune old PNP requests");
                }
            }
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogFormat;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_config() -> Config {
        Config {
            server_port: 8080,
            keystore_type: KeystoreType::Mock,
            pnp_key_name_base: "phoneNumberPrivacy".to_string(),
            pnp_latest_key_version: 1,
            database: DatabaseConfig::Sqlite {
                path: ":memory:".to_string(),
            },
            migrate_legacy_data: false,
            db_timeout_ms: 1000,
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

    /// A config that passes every startup check for a real key store up to the
    /// one a test removes: a database file, a full node and a project ID.
    fn real_keystore_config(db_file: &tempfile::NamedTempFile) -> Config {
        Config {
            keystore_type: KeystoreType::GoogleSecretManager,
            blockchain_provider: Some("http://localhost:8545".to_string()),
            google_project_id: Some("project".to_string()),
            database: DatabaseConfig::Sqlite {
                path: db_file.path().to_str().unwrap().to_string(),
            },
            ..test_config()
        }
    }

    /// Starts a JSON-RPC server on a free local port that answers every call
    /// with `chain_id`, and returns its URL.
    async fn spawn_full_node(chain_id: u64) -> String {
        let app = Router::new().fallback(
            move |axum::Json(request): axum::Json<serde_json::Value>| async move {
                axum::Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": format!("0x{chain_id:x}"),
                }))
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn status_returns_version() {
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let config = test_config();
        let account_service = Arc::new(MockAccountService::new(None, 0));
        let key_provider: Arc<dyn KeyProvider> = Arc::new(MockKeyProvider::new());
        let app = build_router_with_services(config, account_service, key_provider)
            .await
            .unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
        let app = build_router(test_config()).await.unwrap();

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
    async fn metrics_endpoint_returns_prometheus_text() {
        let app = build_router(test_config()).await.unwrap();

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

    /// Like the TS signer, only the API endpoints are counted in the request
    /// metrics: not scrapes, probes or paths that do not exist.
    #[tokio::test]
    async fn request_metrics_count_only_the_api_endpoints() {
        let app = build_router(test_config()).await.unwrap();
        let get = |uri: &str| Request::builder().uri(uri).body(Body::empty()).unwrap();

        for uri in ["/status", "/metrics", "/no-such-path"] {
            app.clone().oneshot(get(uri)).await.unwrap();
        }
        let invalid_quota_request = Request::builder()
            .method("POST")
            .uri("/quotaStatus")
            .body(Body::from("{}"))
            .unwrap();
        app.clone().oneshot(invalid_quota_request).await.unwrap();

        let response = app.oneshot(get("/metrics")).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();

        assert!(
            text.contains(r#"requests{endpoint="/quotaStatus"}"#),
            "{text}"
        );
        assert!(
            text.contains(r#"responses{endpoint="/quotaStatus",status_code="400"}"#),
            "{text}"
        );
        for uncounted in ["/status", "/metrics", "unknown", "/no-such-path"] {
            assert!(
                !text.contains(&format!(r#"endpoint="{uncounted}""#)),
                "{uncounted} is counted in:\n{text}"
            );
        }
    }

    #[tokio::test]
    async fn build_router_fails_for_google_secret_manager_without_blockchain_provider() {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let config = Config {
            blockchain_provider: None,
            ..real_keystore_config(&db_file)
        };

        assert_eq!(
            build_router(config).await.err(),
            Some(OdisError::FullNodeError)
        );
    }

    /// A real keystore with a database that vanishes on restart would hand every
    /// account its full quota again, so the signer must not start that way.
    #[tokio::test]
    async fn build_router_fails_for_a_real_keystore_with_an_in_memory_database() {
        let config = Config {
            keystore_type: KeystoreType::GoogleSecretManager,
            blockchain_provider: Some("http://localhost:8545".to_string()),
            google_project_id: Some("project".to_string()),
            database: DatabaseConfig::Sqlite {
                path: ":memory:".to_string(),
            },
            ..test_config()
        };

        assert_eq!(
            build_router(config).await.err(),
            Some(OdisError::DatabaseError)
        );
    }

    #[tokio::test]
    async fn build_router_fails_when_the_key_store_lacks_the_latest_key() {
        // The mock key store holds phoneNumberPrivacy-1 to -3.
        assert!(build_router(test_config()).await.is_ok());

        let unknown_version = Config {
            pnp_latest_key_version: 4,
            ..test_config()
        };
        assert_eq!(
            build_router(unknown_version).await.err(),
            Some(OdisError::KeyFetchError)
        );

        let unknown_name = Config {
            pnp_key_name_base: "custom".to_string(),
            ..test_config()
        };
        assert_eq!(
            build_router(unknown_name).await.err(),
            Some(OdisError::KeyFetchError)
        );
    }

    #[tokio::test]
    async fn build_router_fails_for_google_secret_manager_without_project_id() {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let base = real_keystore_config(&db_file);
        let config = Config {
            // The chain ID check comes first and must pass to reach the key store.
            blockchain_provider: Some(spawn_full_node(base.chain_id).await),
            google_project_id: None,
            ..base
        };

        assert_eq!(
            build_router(config).await.err(),
            Some(OdisError::KeyFetchError)
        );
    }
}
