//! The TS combiner's error and warning strings, byte for byte
//! (`packages/common/src/interfaces/errors.ts`).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub const UNKNOWN_ERROR: &str = "CELO_ODIS_ERR_00 Something went wrong";
pub const VERIFY_PARTIAL_SIGNATURE_ERROR: &str =
    "CELO_ODIS_ERR_06 SIG_ERR BLS partial signature verification Failure";
pub const NOT_ENOUGH_PARTIAL_SIGNATURES: &str =
    "CELO_ODIS_ERR_07 SIG_ERR Not enough partial signatures";
pub const INCONSISTENT_SIGNER_RESPONSES: &str =
    "CELO_ODIS_ERR_08 SIG_ERR Inconsistent responses from signers";
pub const SIGNER_REQUEST_ERROR: &str = "CELO_ODIS_ERR_09 SIG_ERR Failure in signer request";
pub const TIMEOUT_FROM_SIGNER: &str = "CELO_ODIS_ERR_10 SIG_ERR Timeout from signer";
pub const FULL_NODE_ERROR: &str = "CELO_ODIS_ERR_11 NODE_ERR Failed to read on-chain state";
pub const INVALID_KEY_VERSION_RESPONSE: &str =
    "CELO_ODIS_ERR_20 SIG_ERR Signer response key version header is invalid";
pub const INVALID_SIGNER_RESPONSE: &str =
    "CELO_ODIS_ERR_21 SIG_ERR Signer response body is invalid";
pub const SIGNER_RESPONSE_FAILED_WITH_OK_STATUS: &str =
    "CELO_ODIS_ERR_22 SIG_ERR Signer response failed with 200 status";
pub const THRESHOLD_PNP_QUOTA_STATUS_FAILURE: &str =
    "CELO_ODIS_ERR_23 SIG_ERR Failed to get PNP quota status from a threshold of signers";
pub const FAILURE_TO_GET_DEK: &str =
    "CELO_ODIS_ERR_27 NODE_ERR Failed to read user's DEK from full-node";
pub const INVALID_INPUT: &str = "CELO_ODIS_WARN_01 BAD_INPUT Invalid input parameters";
pub const UNAUTHENTICATED_USER: &str =
    "CELO_ODIS_WARN_02 BAD_INPUT Missing or invalid authentication";
pub const EXCEEDED_QUOTA: &str = "CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota";
pub const INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS: &str =
    "CELO_ODIS_WARN_06 SIGNER Discrepancy found in signers quota measurements";
pub const CANCELLED_REQUEST_TO_SIGNER: &str =
    "CELO_ODIS_WARN_08 SIGNER Cancelled request to signer";
pub const INVALID_KEY_VERSION_REQUEST: &str =
    "CELO_ODIS_WARN_12 BAD_INPUT Request key version header is invalid";
pub const API_UNAVAILABLE: &str = "CELO_ODIS_WARN_13 BAD_INPUT API is unavailable";
pub const SIGNER_RESPONSE_DISCREPANCIES: &str =
    "CELO_ODIS_WARN_17 SIGNER Discrepancies detected in signer responses";
pub const INCONSISTENT_SIGNER_QUERY_MEASUREMENTS: &str =
    "CELO_ODIS_WARN_18 SIGNER Discrepancy found in signers performed query count measurements";

/// An error answer: a status and one of the strings above. The body is the TS
/// combiner's failure body; field order is the TS key order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Failure {
    pub status: StatusCode,
    pub error: &'static str,
}

#[derive(Serialize)]
struct FailureBody {
    success: bool,
    version: &'static str,
    error: &'static str,
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = FailureBody {
            success: false,
            version: env!("CARGO_PKG_VERSION"),
            error: self.error,
        };
        (self.status, Json(body)).into_response()
    }
}
