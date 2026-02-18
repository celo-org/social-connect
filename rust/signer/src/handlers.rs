use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::auth::{Authorization, authenticate_user};
use crate::errors::OdisError;
use crate::server::AppState;
use crate::types::{
    AuthenticationMethod, KEY_VERSION_HEADER, KeyVersion, PnpQuotaRequest, PnpQuotaResponseSuccess,
    SignMessageRequest, SignMessageResponseSuccess,
};

/// Duplicate request warning, matching TS `WarningMessage.DUPLICATE_REQUEST_TO_GET_PARTIAL_SIG`.
const DUPLICATE_REQUEST_WARNING: &str =
    "CELO_ODIS_WARN_04 BAD_INPUT Attempt to replay partial signature request";

pub async fn status_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Check authentication. Returns `Err(401)` on failure.
fn check_auth(
    body: &[u8],
    authorization: &Authorization,
    account: alloy::primitives::Address,
    authentication_method: Option<AuthenticationMethod>,
    dek: &str,
) -> Result<(), OdisError> {
    if !authenticate_user(
        body,
        authorization.0.as_deref(),
        account,
        authentication_method,
        Some(dek).filter(|s| !s.is_empty()),
    ) {
        return Err(OdisError::UnauthenticatedUser);
    }
    Ok(())
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

    let account = state.account_service.get_account(request.account).await?;

    if state.config.blockchain_provider.is_some() {
        check_auth(
            &body,
            &authorization,
            request.account,
            request.authentication_method,
            &account.dek,
        )?;
    }

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
fn compute_signature(state: &AppState, key_version: u32, blinded_query: &str) -> Option<String> {
    let hex_key = state
        .key_provider
        .get_key(&state.config.pnp_key_name_base, key_version)
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
        .inspect_err(|e| tracing::error!("signature computation failed: {e}"))
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

    let request: SignMessageRequest =
        serde_json::from_slice(&body).map_err(|_| OdisError::InvalidInput)?;
    request.validate()?;

    let account = state.account_service.get_account(request.account).await?;

    if state.config.blockchain_provider.is_some() {
        check_auth(
            &body,
            &authorization,
            request.account,
            request.authentication_method,
            &account.dek,
        )?;
    }

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

    // If not a duplicate, check quota
    if duplicate_sig.is_none() && used_quota >= total_quota {
        return Err(OdisError::ExceededQuota {
            total_quota,
            performed_query_count: used_quota,
        });
    }

    let key_version = requested_key_version.unwrap_or(state.config.pnp_latest_key_version);

    let (signature, performed_query_count, warnings) = if let Some(sig) = duplicate_sig {
        (sig, used_quota, vec![DUPLICATE_REQUEST_WARNING.to_string()])
    } else {
        let signature = compute_signature(&state, key_version, &request.blinded_query_phone_number)
            .ok_or(OdisError::SignatureComputationFailure {
                total_quota,
                performed_query_count: used_quota,
            })?;

        // Record the request
        state
            .request_service
            .record_request(
                request.account,
                &request.blinded_query_phone_number,
                &signature,
            )
            .await
            .map_err(|_| OdisError::DatabaseInsertFailure)?;

        (signature, used_quota.saturating_add(1), vec![])
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
