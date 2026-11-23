//! Router construction: routes, CORS, body limit, tracing, metrics and the panic handler.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use ::metrics::counter;
use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::FutureExt;
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::dek::{CachingDekLookup, DekLookup, FullNodeDekLookup, FullNodeError};
use crate::errors::{Failure, UNKNOWN_ERROR};
use crate::handlers::{domain_handler, pnp_pending_handler, pnp_quota_handler, status_handler};
use crate::metrics::{self, ENDPOINT_HANDLER_ERRORS, ERRORS, UNKNOWN_ERRORS};
use crate::signers::build_client;

/// Largest request body read from the network, as the TS combiner's
/// `express.json({ limit: '0.2mb' })`.
const TRANSPORT_BODY_LIMIT_BYTES: usize = 209_715;

/// The CORS headers every response carries, spelled as the TS combiner's `cors`
/// middleware spells them.
const CORS_HEADERS: [(&str, &str); 3] = [
    ("access-control-allow-origin", "*"),
    ("access-control-allow-methods", "GET, POST, OPTIONS"),
    (
        "access-control-allow-headers",
        "Origin, X-Requested-With, Content-Type, Accept, Authorization, odis-key-version",
    ),
];

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub client: reqwest::Client,
    pub dek: Arc<dyn DekLookup>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    FullNode(#[from] FullNodeError),
    #[error("the HTTP client for signer calls could not be built: {0}")]
    SignerClient(reqwest::Error),
}

/// Builds the router with the full node DEK lookup and its cache. Stops the start
/// when the full node is on another chain than `CHAIN_ID`.
pub async fn build_router(
    config: Config,
    metrics: PrometheusHandle,
) -> Result<Router, StartupError> {
    let full_node = FullNodeDekLookup::new(&config)?;
    full_node.verify_chain_id().await?;
    let dek = Arc::new(CachingDekLookup::new(Arc::new(full_node)));
    build_router_with_services(config, dek, metrics)
}

pub fn build_router_with_services(
    config: Config,
    dek: Arc<dyn DekLookup>,
    metrics: PrometheusHandle,
) -> Result<Router, StartupError> {
    for signer in config.signers.iter().filter(|s| s.share_index.is_none()) {
        tracing::warn!(
            url = signer.url,
            "signer entry has no shareIndex: its partial signatures are not checked against a share index"
        );
    }

    let state = AppState {
        config: Arc::new(config),
        client: build_client().map_err(StartupError::SignerClient)?,
        dek,
    };

    // Layers run from the bottom up, so the panic layer sits inside the metrics
    // layer, and a request that panics is counted like any other response.
    Ok(Router::new()
        .route("/status", get(status_handler))
        .route("/sign", post(pnp_pending_handler))
        .route("/quotaStatus", post(pnp_quota_handler))
        .route("/domain/sign", post(domain_handler))
        .route("/domain/quotaStatus", post(domain_handler))
        .route("/domain/disable", post(domain_handler))
        .route(
            "/metrics",
            get(move || std::future::ready(metrics.render())),
        )
        .layer(middleware::from_fn(panic_layer))
        .layer(middleware::from_fn(metrics::http_metrics_layer))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(TRANSPORT_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn(cors_layer))
        .with_state(state))
}

/// Answers every `OPTIONS` request itself, whatever the path, and adds the CORS
/// headers to every other response, so that a browser's preflight succeeds and
/// error answers stay readable to web clients.
async fn cors_layer(request: Request, next: Next) -> Response {
    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::OK.into_response()
    } else {
        next.run(request).await
    };
    for (name, value) in CORS_HEADERS {
        response.headers_mut().insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}

/// Catches a handler panic and answers the unknown-error failure, counting it
/// under the route as the TS combiner counts an error that escapes a handler.
pub async fn panic_layer(request: Request, next: Next) -> Response {
    let endpoint = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unknown".to_owned(), |p| p.as_str().to_owned());
    match AssertUnwindSafe(next.run(request)).catch_unwind().await {
        Ok(response) => response,
        Err(panic) => {
            tracing::error!(
                panic = panic_message(panic.as_ref()),
                endpoint,
                "request handler panicked"
            );
            counter!(ENDPOINT_HANDLER_ERRORS, "endpoint" => endpoint.clone()).increment(1);
            counter!(UNKNOWN_ERRORS, "endpoint" => endpoint.clone()).increment(1);
            counter!(ERRORS, "endpoint" => endpoint, "error_type" => UNKNOWN_ERROR).increment(1);
            Failure {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error: UNKNOWN_ERROR,
            }
            .into_response()
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("no message")
}
