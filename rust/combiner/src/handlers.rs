//! Handlers: status, the domain endpoints, and `/quotaStatus`.

use ::metrics::counter;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{MatchedPath, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::auth::authenticate;
use crate::body::CanonicalBody;
use crate::errors::{
    API_UNAVAILABLE, Failure, INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS, INVALID_INPUT,
    INVALID_KEY_VERSION_REQUEST, THRESHOLD_PNP_QUOTA_STATUS_FAILURE, UNAUTHENTICATED_USER,
    UNKNOWN_ERROR,
};
use crate::metrics::WARNINGS;
use crate::quota::{SignerQuota, discrepancy_warnings, quota_state};
use crate::request::{parse_quota_request, resolve_key_version};
use crate::server::AppState;
use crate::signers::{Flow, SignerEndpoint, ThresholdCall, call_signers};

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

/// The success body of `/quotaStatus`, fields in the TS order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaStatusAnswer {
    success: bool,
    version: &'static str,
    performed_query_count: u64,
    total_quota: u64,
    warnings: Vec<String>,
}

/// `/quotaStatus`. The request runs in a task of its own, so that it runs to its end when the
/// client disconnects, as in TS, instead of being dropped with its signer calls. A panic in the
/// task is raised again here, for the panic layer to answer and count.
pub async fn pnp_quota_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match tokio::spawn(quota_status(state, headers, body)).await {
        Ok(response) => response,
        Err(join_error) => match join_error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(_) => Failure {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error: UNKNOWN_ERROR,
            }
            .into_response(),
        },
    }
}

/// Checks the request, authenticates it, then resolves the key version, in the TS order
/// (`apps/combiner/src/pnp/endpoints/quota/action.ts:37-73`), and asks every signer.
async fn quota_status(state: AppState, headers: HeaderMap, raw: Bytes) -> Response {
    const ENDPOINT: SignerEndpoint = SignerEndpoint::QuotaStatus;

    let parsed = CanonicalBody::parse(headers.get(CONTENT_TYPE), &raw)
        .map_err(|error| error.to_string())
        .and_then(|body| match parse_quota_request(&body) {
            Ok(request) => Ok((body, request)),
            Err(error) => Err(error.to_string()),
        });
    let (body, request) = match parsed {
        Ok(parsed) => parsed,
        Err(reason) => {
            info!(%reason, "{INVALID_INPUT}");
            return warning_failure(ENDPOINT, StatusCode::BAD_REQUEST, INVALID_INPUT);
        }
    };

    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if let Err(failure) = authenticate(state.dek.as_ref(), &body, &request, authorization).await {
        info!(?failure, "{UNAUTHENTICATED_USER}");
        return warning_failure(ENDPOINT, StatusCode::UNAUTHORIZED, UNAUTHENTICATED_USER);
    }

    let config = &state.config;
    let Some(key) = resolve_key_version(&headers, config.current_key_version)
        .ok()
        .and_then(|version| config.key_version(version))
    else {
        return warning_failure(
            ENDPOINT,
            StatusCode::BAD_REQUEST,
            INVALID_KEY_VERSION_REQUEST,
        );
    };

    let call = ThresholdCall {
        client: &state.client,
        signers: &config.signers,
        endpoint: ENDPOINT,
        key_version: key.key_version,
        threshold: key.threshold,
        // `authenticate` refused a request without one.
        authorization: authorization.unwrap_or_default(),
        body: Bytes::from(body.as_str().to_owned()),
        deadline: config.signer_timeout,
    };
    let outcome = call_signers(call, |_, _| Flow::Continue).await;

    let quotas: Vec<SignerQuota> = outcome
        .successes
        .into_iter()
        .map(|(_, success)| success.quota)
        .collect();
    // Counted whether or not the request succeeds, as in TS.
    let mut warnings = discrepancy_warnings(&quotas);
    for warning in &warnings {
        warn!(?quotas, "{}", warning.text);
        count_warning(ENDPOINT, warning.code);
    }

    let Some(quota) = quota_state(key.threshold, &quotas) else {
        if quotas.len() >= key.threshold {
            error!(
                error = INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
                "Error combining signer quota status responses"
            );
        }
        let status = outcome
            .majority_status
            .and_then(|status| StatusCode::from_u16(status).ok())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return Failure {
            status,
            error: THRESHOLD_PNP_QUOTA_STATUS_FAILURE,
        }
        .into_response();
    };
    if let Some(warning) = quota.warning {
        count_warning(ENDPOINT, warning.code);
        warnings.push(warning);
    }
    Json(QuotaStatusAnswer {
        success: true,
        version: env!("CARGO_PKG_VERSION"),
        performed_query_count: quota.performed_query_count,
        total_quota: quota.total_quota,
        warnings: warnings.into_iter().map(|warning| warning.text).collect(),
    })
    .into_response()
}

fn count_warning(endpoint: SignerEndpoint, warning: &'static str) {
    counter!(WARNINGS, "endpoint" => endpoint.path(), "warning_type" => warning).increment(1);
}

/// Counts `warning` and answers it with `status`.
fn warning_failure(
    endpoint: SignerEndpoint,
    status: StatusCode,
    warning: &'static str,
) -> Response {
    count_warning(endpoint, warning);
    Failure {
        status,
        error: warning,
    }
    .into_response()
}
