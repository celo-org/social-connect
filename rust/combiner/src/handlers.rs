//! Handlers: status, the domain endpoints, `/sign` and `/quotaStatus`.

use std::time::Instant;

use ::metrics::{counter, histogram};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{MatchedPath, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::auth::authenticate;
use crate::body::CanonicalBody;
use crate::config::{Config, SignerEntry};
use crate::errors::{
    API_UNAVAILABLE, EXCEEDED_QUOTA, Failure, INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
    INVALID_INPUT, INVALID_KEY_VERSION_REQUEST, NOT_ENOUGH_PARTIAL_SIGNATURES,
    THRESHOLD_PNP_QUOTA_STATUS_FAILURE, UNAUTHENTICATED_USER, UNKNOWN_ERROR,
    VERIFY_PARTIAL_SIGNATURE_ERROR,
};
use crate::metrics::{
    BLS_COMPUTE_ERRORS, ERRORS, NOT_ENOUGH_SIG_ERRORS, SIGNATURE_AGGREGATION_LATENCY,
    SIGNER_TAIL_LATENCY, WARNINGS,
};
use crate::quota::{SignerQuota, Warning, discrepancy_warnings, quota_state};
use crate::request::{PnpRequest, parse_quota_request, parse_sign_request, resolve_key_version};
use crate::server::AppState;
use crate::signature::{CombineAttempt, KeyVersion, PartialSignatures, Rejection};
use crate::signers::{Flow, SignerEndpoint, SignerSuccess, ThresholdCall, call_signers};

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

/// The success body of `/sign`, fields in the TS order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignAnswer {
    success: bool,
    version: &'static str,
    /// The combined signature, base64.
    signature: String,
    performed_query_count: u64,
    total_quota: u64,
    warnings: Vec<String>,
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

/// `/sign`, run to its end in a task of its own (see `run_to_end`).
pub async fn pnp_sign_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    run_to_end(sign(state, headers, body)).await
}

/// `/quotaStatus`, run to its end in a task of its own (see `run_to_end`).
pub async fn pnp_quota_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    run_to_end(quota_status(state, headers, body)).await
}

/// Runs a request in a task of its own, so that it runs to its end when the client
/// disconnects, as in TS, instead of being dropped with its signer calls. A panic in the task
/// is raised again here, for the panic layer to answer and count.
async fn run_to_end(
    request: impl Future<Output = Result<Response, Failure>> + Send + 'static,
) -> Response {
    match tokio::spawn(request).await {
        Ok(answer) => answer.into_response(),
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

/// Checks the request, resolves the key version, then authenticates, in the TS order
/// (`odis-combiner-4.0.5:apps/combiner/src/pnp/endpoints/sign/action.ts:41-57`). Asks every signer
/// for a partial signature and combines them as they arrive, stopping the calls once a combination
/// verifies. A failure is an `Err`, so each check can end the request with `?`.
async fn sign(state: AppState, headers: HeaderMap, raw: Bytes) -> Result<Response, Failure> {
    const ENDPOINT: SignerEndpoint = SignerEndpoint::Sign;

    let (body, request, blinded_message) = CanonicalBody::parse(&headers, &raw)
        .map_err(|error| error.to_string())
        .and_then(|body| {
            let request = parse_sign_request(&body).map_err(|error| error.to_string())?;
            // `parse_sign_request` checked that the query is base64.
            let query = request.blinded_query_phone_number.as_deref();
            let blinded_message = BASE64
                .decode(query.unwrap_or_default())
                .map_err(|error| error.to_string())?;
            Ok((body, request, blinded_message))
        })
        .map_err(|reason| invalid_input(ENDPOINT, &reason))?;
    let key = requested_key(ENDPOINT, &headers, &state.config)?;
    let authorization = authenticated(ENDPOINT, &state, &headers, &body, &request).await?;

    let call = threshold_call(&state, ENDPOINT, key, authorization, &body);
    let mut combiner = Combiner::new(&state.config.signers, key, blinded_message);
    let outcome = call_signers(call, |signer, success| match success.signature.as_deref() {
        Some(partial) if combiner.add(signer, partial) => Flow::Stop,
        _ => Flow::Continue,
    })
    .await;
    let signature = combiner.finish();

    let quotas = signer_quotas(outcome.successes);
    let discrepancies = counted_discrepancies(ENDPOINT, &quotas);
    if let Some(signature) = signature {
        if let Some(quota) = quota_state(key.threshold, &quotas) {
            return Ok(Json(SignAnswer {
                success: true,
                version: env!("CARGO_PKG_VERSION"),
                signature: BASE64.encode(signature),
                performed_query_count: quota.performed_query_count,
                total_quota: quota.total_quota,
                warnings: answer_warnings(ENDPOINT, discrepancies, quota.warning),
            })
            .into_response());
        }
        error!(?quotas, "{INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS}");
    }

    let status = failure_status(outcome.majority_status);
    if status == StatusCode::FORBIDDEN {
        return Err(warning_failure(ENDPOINT, status, EXCEEDED_QUOTA));
    }
    counter!(
        ERRORS,
        "endpoint" => ENDPOINT.path(),
        "error_type" => NOT_ENOUGH_PARTIAL_SIGNATURES,
    )
    .increment(1);
    counter!(NOT_ENOUGH_SIG_ERRORS, "endpoint" => ENDPOINT.path()).increment(1);
    Err(Failure {
        status,
        error: NOT_ENOUGH_PARTIAL_SIGNATURES,
    })
}

/// Collects the signers' partial signatures for one `/sign` request and combines them once
/// there are enough, as the TS combiner's `BLSCryptographyClient` does. Every dropped partial
/// is logged with its signer's URL and counted.
struct Combiner<'a> {
    signers: &'a [SignerEntry],
    partials: PartialSignatures<'a>,
    combined: Option<Vec<u8>>,
    tail_latency: TailLatency,
}

/// `SIGNER_TAIL_LATENCY` runs from the first kept partial to the first combination attempt,
/// and is recorded once.
enum TailLatency {
    NotStarted,
    Since(Instant),
    Recorded,
}

impl<'a> Combiner<'a> {
    fn new(signers: &'a [SignerEntry], key: &'a KeyVersion, blinded_message: Vec<u8>) -> Self {
        Self {
            signers,
            partials: PartialSignatures::new(key, blinded_message),
            combined: None,
            tail_latency: TailLatency::NotStarted,
        }
    }

    /// Checks and keeps the partial of the signer at index `signer`, then combines if there
    /// are enough. True once a combination has verified.
    fn add(&mut self, signer: usize, partial: &str) -> bool {
        info!(signer = %self.signers[signer].url, partial, "Received a partial signature");
        // Each signer answers once, so a signer cannot add two partials.
        if let Err(rejection) = self
            .partials
            .add(signer, self.signers[signer].share_index, partial)
        {
            self.drop_partial(signer, &rejection);
            return false;
        }
        if let TailLatency::NotStarted = self.tail_latency {
            self.tail_latency = TailLatency::Since(Instant::now());
        }
        self.combine()
    }

    /// Combines the kept partials if there are enough. A failed combination drops the
    /// partials that fail verification.
    fn combine(&mut self) -> bool {
        let started = Instant::now();
        match self.partials.try_combine() {
            CombineAttempt::NotEnough => false,
            CombineAttempt::Combined(signature) => {
                self.record_attempt(started);
                self.combined = Some(signature);
                true
            }
            CombineAttempt::Failed { rejected } => {
                self.record_attempt(started);
                warn!(
                    dropped = rejected.len(),
                    "Combining the partial signatures failed, so each was verified on its own"
                );
                for signer in rejected {
                    self.drop_partial(signer, &Rejection::FailedVerification);
                }
                false
            }
        }
    }

    /// Records one combination attempt that began at `started`, and the tail latency up to
    /// the first attempt.
    fn record_attempt(&mut self, started: Instant) {
        let endpoint = SignerEndpoint::Sign.path();
        histogram!(SIGNATURE_AGGREGATION_LATENCY, "endpoint" => endpoint)
            .record(started.elapsed().as_secs_f64());
        if let TailLatency::Since(first) = self.tail_latency {
            histogram!(SIGNER_TAIL_LATENCY, "endpoint" => endpoint)
                .record(started.duration_since(first).as_secs_f64());
            self.tail_latency = TailLatency::Recorded;
        }
    }

    /// The combined signature, combining once more if none has verified yet.
    fn finish(mut self) -> Option<Vec<u8>> {
        if self.combined.is_none() {
            self.combine();
        }
        self.combined
    }

    fn drop_partial(&self, signer: usize, rejection: &Rejection) {
        let endpoint = SignerEndpoint::Sign.path();
        let url = &self.signers[signer].url;
        error!(signer = %url, ?rejection, "{VERIFY_PARTIAL_SIGNATURE_ERROR}");
        counter!(BLS_COMPUTE_ERRORS, "endpoint" => endpoint, "signer" => url.clone()).increment(1);
        counter!(
            ERRORS,
            "endpoint" => endpoint,
            "error_type" => VERIFY_PARTIAL_SIGNATURE_ERROR,
        )
        .increment(1);
    }
}

/// Checks the request, authenticates it, then resolves the key version, in the TS order
/// (`odis-combiner-4.0.5:apps/combiner/src/pnp/endpoints/quota/action.ts:37-73`), and asks every
/// signer. A failure is an `Err`, so each check can end the request with `?`.
async fn quota_status(
    state: AppState,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<Response, Failure> {
    const ENDPOINT: SignerEndpoint = SignerEndpoint::QuotaStatus;

    let (body, request) = CanonicalBody::parse(&headers, &raw)
        .map_err(|error| error.to_string())
        .and_then(|body| {
            let request = parse_quota_request(&body).map_err(|error| error.to_string())?;
            Ok((body, request))
        })
        .map_err(|reason| invalid_input(ENDPOINT, &reason))?;
    let authorization = authenticated(ENDPOINT, &state, &headers, &body, &request).await?;
    let key = requested_key(ENDPOINT, &headers, &state.config)?;

    let call = threshold_call(&state, ENDPOINT, key, authorization, &body);
    let outcome = call_signers(call, |_, _| Flow::Continue).await;

    let quotas = signer_quotas(outcome.successes);
    let discrepancies = counted_discrepancies(ENDPOINT, &quotas);
    let Some(quota) = quota_state(key.threshold, &quotas) else {
        if quotas.len() >= key.threshold {
            error!(
                error = INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
                "Error combining signer quota status responses"
            );
        }
        return Err(Failure {
            status: failure_status(outcome.majority_status),
            error: THRESHOLD_PNP_QUOTA_STATUS_FAILURE,
        });
    };
    Ok(Json(QuotaStatusAnswer {
        success: true,
        version: env!("CARGO_PKG_VERSION"),
        performed_query_count: quota.performed_query_count,
        total_quota: quota.total_quota,
        warnings: answer_warnings(ENDPOINT, discrepancies, quota.warning),
    })
    .into_response())
}

/// Logs why the body was refused and answers 400 `WARN_01`.
fn invalid_input(endpoint: SignerEndpoint, reason: &str) -> Failure {
    info!(%reason, "{INVALID_INPUT}");
    warning_failure(endpoint, StatusCode::BAD_REQUEST, INVALID_INPUT)
}

/// The configured key version that the request's `odis-key-version` header names, or the
/// current one without a header; 400 `WARN_12` when it is not configured.
fn requested_key<'c>(
    endpoint: SignerEndpoint,
    headers: &HeaderMap,
    config: &'c Config,
) -> Result<&'c KeyVersion, Failure> {
    resolve_key_version(headers, config.current_key_version)
        .ok()
        .and_then(|version| config.key_version(version))
        .ok_or_else(|| {
            warning_failure(
                endpoint,
                StatusCode::BAD_REQUEST,
                INVALID_KEY_VERSION_REQUEST,
            )
        })
}

/// Authenticates the request and returns its `Authorization` header; 401 `WARN_02` when the
/// check fails.
async fn authenticated<'h>(
    endpoint: SignerEndpoint,
    state: &AppState,
    headers: &'h HeaderMap,
    body: &CanonicalBody,
    request: &PnpRequest,
) -> Result<&'h str, Failure> {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match authenticate(state.dek.as_ref(), body, request, authorization).await {
        // `authenticate` refuses a request without one.
        Ok(()) => Ok(authorization.unwrap_or_default()),
        Err(failure) => {
            info!(?failure, "{UNAUTHENTICATED_USER}");
            Err(warning_failure(
                endpoint,
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED_USER,
            ))
        }
    }
}

/// The request to every signer: the canonical body, the client's `Authorization` and the
/// resolved key version.
fn threshold_call<'a>(
    state: &'a AppState,
    endpoint: SignerEndpoint,
    key: &KeyVersion,
    authorization: &'a str,
    body: &CanonicalBody,
) -> ThresholdCall<'a> {
    ThresholdCall {
        client: &state.client,
        signers: &state.config.signers,
        endpoint,
        key_version: key.key_version,
        threshold: key.threshold,
        authorization,
        body: Bytes::from(body.as_str().to_owned()),
        deadline: state.config.signer_timeout,
    }
}

/// The quota numbers of the signers' successes, in arrival order.
fn signer_quotas(successes: Vec<(usize, SignerSuccess)>) -> Vec<SignerQuota> {
    successes
        .into_iter()
        .map(|(_, success)| success.quota)
        .collect()
}

/// The discrepancy warnings over the signers' answers, logged and counted whether or not the
/// request then succeeds, as in TS.
fn counted_discrepancies(endpoint: SignerEndpoint, quotas: &[SignerQuota]) -> Vec<Warning> {
    let warnings = discrepancy_warnings(quotas);
    for warning in &warnings {
        warn!(?quotas, "{}", warning.text);
        count_warning(endpoint, warning.code);
    }
    warnings
}

/// The `warnings` of a success, in the TS order: the discrepancies, then the quota state's
/// best-guess warning, which is counted here because TS counts it only on success.
fn answer_warnings(
    endpoint: SignerEndpoint,
    discrepancies: Vec<Warning>,
    best_guess: Option<Warning>,
) -> Vec<String> {
    if let Some(warning) = &best_guess {
        count_warning(endpoint, warning.code);
    }
    discrepancies
        .into_iter()
        .chain(best_guess)
        .map(|warning| warning.text)
        .collect()
}

/// The status of a failure: the signers' majority status, or 500 without one.
fn failure_status(majority_status: Option<u16>) -> StatusCode {
    majority_status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn count_warning(endpoint: SignerEndpoint, warning: &'static str) {
    counter!(WARNINGS, "endpoint" => endpoint.path(), "warning_type" => warning).increment(1);
}

/// Counts `warning` and answers it with `status`.
fn warning_failure(endpoint: SignerEndpoint, status: StatusCode, warning: &'static str) -> Failure {
    count_warning(endpoint, warning);
    Failure {
        status,
        error: warning,
    }
}
