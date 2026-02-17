use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::errors::OdisError;
use crate::server::AppState;
use crate::types::{PnpQuotaRequest, PnpQuotaResponseSuccess};

pub async fn status_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn pnp_sign_stub(State(state): State<AppState>) -> impl IntoResponse {
    if !state.config.pnp_api_enabled {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    StatusCode::NOT_IMPLEMENTED.into_response()
}

pub async fn pnp_quota_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<impl IntoResponse, OdisError> {
    if !state.config.pnp_api_enabled {
        return Err(OdisError::ApiUnavailable);
    }

    let request: PnpQuotaRequest =
        serde_json::from_slice(&body).map_err(|_| OdisError::InvalidInput)?;

    // TODO: authenticate user (mock: always pass)

    let used_quota = state.request_service.get_used_quota(request.account)?;
    let total_quota = state.config.mock_total_quota;

    Ok(Json(PnpQuotaResponseSuccess {
        success: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        performed_query_count: used_quota,
        total_quota,
        warnings: vec![],
    }))
}
