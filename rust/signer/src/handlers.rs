use ::metrics::{counter, histogram};
use alloy::primitives::Address;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::account_service::PnpAccount;
use crate::auth::{Authorization, authenticate_user};
use crate::errors::OdisError;
use crate::metrics as metric_names;
use crate::request_service::RecordOutcome;
use crate::server::AppState;
use crate::types::{
    AuthenticationMethod, KEY_VERSION_HEADER, KeyVersion, PnpQuotaRequest, PnpQuotaResponseSuccess,
    SignMessageRequest, SignMessageResponseSuccess,
};

/// Duplicate request warning, matching TS `WarningMessage.DUPLICATE_REQUEST_TO_GET_PARTIAL_SIG`.
const DUPLICATE_REQUEST_WARNING: &str =
    "CELO_ODIS_WARN_04 BAD_INPUT Attempt to replay partial signature request";

/// Domain endpoints are not yet implemented in the Rust port.
/// Return 503 ApiUnavailable, matching the TS signer's disabled-API behavior.
pub async fn domain_sign_handler() -> OdisError {
    OdisError::ApiUnavailable
}

pub async fn domain_quota_handler() -> OdisError {
    OdisError::ApiUnavailable
}

pub async fn domain_disable_handler() -> OdisError {
    OdisError::ApiUnavailable
}

pub async fn status_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Looks up the account and authenticates the request against it. Both PNP
/// endpoints go through here, so they count and verify requests the same way.
///
/// Like the TS signer, a request that names `wallet_key` is counted before its
/// signature is checked. The signature is only checked when a full node is
/// configured, because the DEK comes from chain. Returns `Err(401)` when the
/// check fails.
async fn authenticated_account(
    state: &AppState,
    body: &[u8],
    authorization: &Authorization,
    address: Address,
    authentication_method: Option<AuthenticationMethod>,
) -> Result<PnpAccount, OdisError> {
    let account = state.account_service.get_account(address).await?;

    if authentication_method == Some(AuthenticationMethod::WalletKey) {
        counter!(metric_names::REQUESTS_WITH_WALLET_ADDRESS).increment(1);
    }

    if state.config.blockchain_provider.is_some()
        && !authenticate_user(
            body,
            authorization.0.as_deref(),
            address,
            authentication_method,
            Some(account.dek.as_str()).filter(|s| !s.is_empty()),
        )
    {
        return Err(OdisError::UnauthenticatedUser);
    }
    Ok(account)
}

pub async fn pnp_quota_handler(
    State(state): State<AppState>,
    authorization: Authorization,
    body: Bytes,
) -> Result<impl IntoResponse, OdisError> {
    if !state.config.pnp_api_enabled {
        return Err(OdisError::ApiUnavailable);
    }

    let request: PnpQuotaRequest =
        serde_json::from_slice(&body).map_err(|_| OdisError::InvalidInput)?;

    let account = authenticated_account(
        &state,
        &body,
        &authorization,
        request.account,
        request.authentication_method,
    )
    .await?;

    let used_quota = state
        .request_service
        .get_used_quota(request.account)
        .await
        .map_err(|_| OdisError::FailureToGetPerformedQueryCount)?;
    let total_quota = account.pnp_total_quota;

    Ok(Json(PnpQuotaResponseSuccess {
        success: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        performed_query_count: used_quota,
        total_quota,
        warnings: vec![],
    }))
}

/// Fetches the signing key and computes the blind partial signature, base64
/// encoded. Returns `None` when the key cannot be fetched or the computation
/// fails, after logging the cause: like the TS signer, the caller reports both
/// as a signature computation failure.
async fn compute_signature(
    state: &AppState,
    key_version: u32,
    blinded_query: &str,
) -> Option<String> {
    let hex_key = state
        .key_provider
        .get_key(&state.config.pnp_key_name_base, key_version)
        .await
        .inspect_err(|e| tracing::error!("failed to fetch key version {key_version}: {e}"))
        .ok()?;
    let key_bytes = hex::decode(&hex_key)
        .inspect_err(|e| tracing::error!("key version {key_version} is not valid hex: {e}"))
        .ok()?;
    let blinded_msg = BASE64
        .decode(blinded_query)
        .inspect_err(|e| tracing::error!("blinded message is not valid base64: {e}"))
        .ok()?;
    let sig_bytes = crate::crypto::compute_blinded_signature(&blinded_msg, &key_bytes)
        .inspect_err(|e| {
            counter!(metric_names::SIGNATURE_COMPUTATION_ERRORS).increment(1);
            tracing::error!("signature computation failed: {e}");
        })
        .ok()?;
    Some(BASE64.encode(&sig_bytes))
}

pub async fn pnp_sign_handler(
    State(state): State<AppState>,
    KeyVersion(requested_key_version): KeyVersion,
    authorization: Authorization,
    body: Bytes,
) -> Result<impl IntoResponse, OdisError> {
    if !state.config.pnp_api_enabled {
        return Err(OdisError::ApiUnavailable);
    }

    // The request runs in its own task so that it is finished and charged even
    // when this handler is dropped, which happens when the caller disconnects or
    // the request times out. The combiner disconnects from the slower signers as
    // soon as it has enough partial signatures; if those signers then skipped the
    // charge, their counts for the account would fall behind the others. The TS
    // signer finishes such requests too. A retry is served the stored signature.
    let task = tokio::spawn(sign(state, requested_key_version, authorization, body));
    match task.await {
        Ok(result) => result,
        Err(join_error) => match join_error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(_) => Err(OdisError::Unknown),
        },
    }
}

/// Authenticates, signs and records one PNP sign request.
async fn sign(
    state: AppState,
    requested_key_version: Option<u32>,
    authorization: Authorization,
    body: Bytes,
) -> Result<(HeaderMap, Json<SignMessageResponseSuccess>), OdisError> {
    let request: SignMessageRequest =
        serde_json::from_slice(&body).map_err(|_| OdisError::InvalidInput)?;
    request.validate()?;

    let account = authenticated_account(
        &state,
        &body,
        &authorization,
        request.account,
        request.authentication_method,
    )
    .await?;

    let duplicate_sig = state
        .request_service
        .get_duplicate_request(request.account, &request.blinded_query_phone_number)
        .await?;

    let used_quota = state
        .request_service
        .get_used_quota(request.account)
        .await
        .map_err(|_| OdisError::FailureToGetPerformedQueryCount)?;
    let total_quota = account.pnp_total_quota;

    // Recorded before the quota check, as the TS signer does, so that requests
    // refused for lack of quota are included.
    histogram!(metric_names::USER_REMAINING_QUOTA, "endpoint" => "/sign")
        .record(f64::from(total_quota) - f64::from(used_quota));

    // If not a duplicate, check quota
    if duplicate_sig.is_none() && used_quota >= total_quota {
        return Err(OdisError::ExceededQuota {
            total_quota,
            performed_query_count: used_quota,
        });
    }

    let key_version = requested_key_version.unwrap_or(state.config.pnp_latest_key_version);

    let (signature, performed_query_count, warnings) = if let Some(sig) = duplicate_sig {
        counter!(metric_names::DUPLICATE_REQUESTS).increment(1);
        (sig, used_quota, vec![DUPLICATE_REQUEST_WARNING.to_string()])
    } else {
        // The reads above are a fast path that spares the keystore round trip
        // and the signing work. Duplicate detection and the quota bound are
        // decided authoritatively by `record_request` below.
        let signature = compute_signature(&state, key_version, &request.blinded_query_phone_number)
            .await
            .ok_or(OdisError::SignatureComputationFailure {
                total_quota,
                performed_query_count: used_quota,
            })?;

        // Record the request
        match state
            .request_service
            .record_request(
                request.account,
                &request.blinded_query_phone_number,
                &signature,
                total_quota,
            )
            .await
            .map_err(|_| OdisError::DatabaseInsertFailure)?
        {
            RecordOutcome::Recorded {
                performed_query_count,
            } => (signature, performed_query_count, vec![]),
            // A concurrent identical request won the race. Serve its signature
            // rather than charging this account a second time.
            RecordOutcome::Duplicate {
                signature,
                performed_query_count,
            } => {
                counter!(metric_names::DUPLICATE_REQUESTS).increment(1);
                (
                    signature,
                    performed_query_count,
                    vec![DUPLICATE_REQUEST_WARNING.to_string()],
                )
            }
            // Concurrent requests drained the remaining quota after the check
            // above passed.
            RecordOutcome::QuotaExceeded {
                performed_query_count,
            } => {
                return Err(OdisError::ExceededQuota {
                    total_quota,
                    performed_query_count,
                });
            }
        }
    };

    let mut response_headers = HeaderMap::new();
    response_headers.insert(KEY_VERSION_HEADER, key_version.to_string().parse().unwrap());

    Ok((
        response_headers,
        Json(SignMessageResponseSuccess {
            success: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            signature,
            performed_query_count,
            total_quota,
            warnings,
        }),
    ))
}
