//! Handlers for the routes that do not need the signers: status and the domain endpoints.

use ::metrics::counter;
use axum::Json;
use axum::extract::MatchedPath;
use axum::http::StatusCode;

use crate::errors::{API_UNAVAILABLE, Failure};
use crate::metrics::WARNINGS;

pub async fn status_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
}

/// The domain endpoints are not part of the port. Like the TS combiner with the
/// domains API disabled, they answer 503 and count a warning.
pub async fn domain_handler(path: MatchedPath) -> Failure {
    counter!(
        WARNINGS,
        "endpoint" => path.as_str().to_owned(),
        "warning_type" => API_UNAVAILABLE,
    )
    .increment(1);
    Failure {
        status: StatusCode::SERVICE_UNAVAILABLE,
        error: API_UNAVAILABLE,
    }
}

/// Stands in for the PNP handlers until they are added.
pub async fn pnp_pending_handler() -> Failure {
    Failure {
        status: StatusCode::SERVICE_UNAVAILABLE,
        error: API_UNAVAILABLE,
    }
}
