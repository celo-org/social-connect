use ::metrics::{counter, histogram};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::auth::{Authorization, authenticate_user};
use crate::errors::OdisError;
use crate::metrics as metric_names;
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
        .await?;
    let total_quota = account.pnp_total_quota;

    Ok(Json(PnpQuotaResponseSuccess {
        success: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        performed_query_count: used_quota,
        total_quota,
        warnings: vec![],
    }))
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

    if request.authentication_method != Some(AuthenticationMethod::EncryptionKey) {
        counter!(metric_names::REQUESTS_WITH_WALLET_ADDRESS).increment(1);
    }

    let duplicate_sig = state
        .request_service
        .get_duplicate_request(request.account, &request.blinded_query_phone_number)
        .await?;

    let used_quota = state
        .request_service
        .get_used_quota(request.account)
        .await?;
    let total_quota = account.pnp_total_quota;

    // If not a duplicate, check quota
    if duplicate_sig.is_none() && used_quota >= total_quota {
        return Err(OdisError::ExceededQuota);
    }

    let key_version = requested_key_version.unwrap_or(state.config.pnp_latest_key_version);

    let remaining_quota = total_quota.saturating_sub(used_quota);
    histogram!(metric_names::USER_REMAINING_QUOTA, "endpoint" => "/sign")
        .record(remaining_quota as f64);

    let (signature, performed_query_count, warnings) = if let Some(sig) = duplicate_sig {
        counter!(metric_names::DUPLICATE_REQUESTS).increment(1);
        (sig, used_quota, vec![DUPLICATE_REQUEST_WARNING.to_string()])
    } else {
        // Fetch key and compute signature
        let hex_key = state
            .key_provider
            .get_key(&state.config.pnp_key_name_base, key_version)?;
        let key_bytes = hex::decode(&hex_key).map_err(|_| OdisError::KeyFetchError)?;
        let blinded_msg = BASE64
            .decode(&request.blinded_query_phone_number)
            .map_err(|_| OdisError::InvalidInput)?;
        let sig_bytes = crate::crypto::compute_blinded_signature(&blinded_msg, &key_bytes)
            .map_err(|_| {
                counter!(metric_names::SIGNATURE_COMPUTATION_ERRORS).increment(1);
                OdisError::SignatureComputationFailure
            })?;
        let signature = BASE64.encode(&sig_bytes);

        // Record the request
        state
            .request_service
            .record_request(
                request.account,
                &request.blinded_query_phone_number,
                &signature,
            )
            .await?;

        (signature, used_quota + 1, vec![])
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
