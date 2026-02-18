use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Error/warning codes matching the TS `ErrorMessage` and `WarningMessage` enums.
/// Only PNP-relevant codes are included for now.
#[derive(Debug, Clone, PartialEq)]
pub enum OdisError {
    /// 400 — invalid request body or parameters
    InvalidInput,
    /// 400 — invalid key version header
    InvalidKeyVersion,
    /// 401 — missing or invalid authentication
    UnauthenticatedUser,
    /// 403 — exceeded service query quota
    ExceededQuota,
    /// 500 — BLS signature computation failed
    SignatureComputationFailure,
    /// 500 — database read/write failed
    DatabaseError,
    /// 500 — failed to retrieve key from keystore
    KeyFetchError,
    /// 500 — generic unknown error
    Unknown,
    /// 500 — timeout
    Timeout,
    /// 500 — failed to read on-chain state
    FullNodeError,
    /// 500 — failed to read on-chain state to calculate total quota
    FailureToGetTotalQuota,
    /// 500 — failed to read user's DEK from full-node
    FailureToGetDek,
    /// 503 — API is unavailable (disabled)
    ApiUnavailable,
}

impl OdisError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::InvalidInput | Self::InvalidKeyVersion => StatusCode::BAD_REQUEST,
            Self::UnauthenticatedUser => StatusCode::UNAUTHORIZED,
            Self::ExceededQuota => StatusCode::FORBIDDEN,
            Self::SignatureComputationFailure
            | Self::DatabaseError
            | Self::KeyFetchError
            | Self::Unknown
            | Self::Timeout
            | Self::FullNodeError
            | Self::FailureToGetTotalQuota
            | Self::FailureToGetDek => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ApiUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// The ODIS error/warning string sent in JSON responses, matching the TS enum values.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "CELO_ODIS_WARN_01 BAD_INPUT Invalid input parameters",
            Self::InvalidKeyVersion => {
                "CELO_ODIS_WARN_12 BAD_INPUT Request key version header is invalid"
            }
            Self::UnauthenticatedUser => {
                "CELO_ODIS_WARN_02 BAD_INPUT Missing or invalid authentication"
            }
            Self::ExceededQuota => "CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota",
            Self::SignatureComputationFailure => {
                "CELO_ODIS_ERR_05 SIG_ERR Failed to compute BLS signature"
            }
            Self::DatabaseError => "CELO_ODIS_ERR_03 DB_ERR Failed to get database entry",
            Self::KeyFetchError => "CELO_ODIS_ERR_04 INIT_ERR Failed to retrieve key from keystore",
            Self::Unknown => "CELO_ODIS_ERR_00 Something went wrong",
            Self::Timeout => "CELO_ODIS_ERR_10 SIG_ERR Timeout from signer",
            Self::FullNodeError => "CELO_ODIS_ERR_11 NODE_ERR Failed to read on-chain state",
            Self::FailureToGetTotalQuota => {
                "CELO_ODIS_ERR_25 NODE_ERR Failed to read on-chain state to calculate total quota"
            }
            Self::FailureToGetDek => {
                "CELO_ODIS_ERR_27 NODE_ERR Failed to read user's DEK from full-node"
            }
            Self::ApiUnavailable => "CELO_ODIS_WARN_13 BAD_INPUT API is unavailable",
        }
    }
}

impl std::fmt::Display for OdisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error_code())
    }
}

impl std::error::Error for OdisError {}

/// JSON error response: `{ success: false, version, error }`.
impl IntoResponse for OdisError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "success": false,
            "version": env!("CARGO_PKG_VERSION"),
            "error": self.error_code(),
        });
        (self.status_code(), Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_expected() {
        assert_eq!(
            OdisError::InvalidInput.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            OdisError::InvalidKeyVersion.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            OdisError::UnauthenticatedUser.status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            OdisError::ExceededQuota.status_code(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            OdisError::SignatureComputationFailure.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::DatabaseError.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::KeyFetchError.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::Unknown.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::Timeout.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::FullNodeError.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::FailureToGetTotalQuota.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::FailureToGetDek.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            OdisError::ApiUnavailable.status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn error_codes_match_ts_enum_values() {
        assert!(
            OdisError::InvalidInput
                .error_code()
                .starts_with("CELO_ODIS_WARN_01")
        );
        assert!(
            OdisError::InvalidKeyVersion
                .error_code()
                .starts_with("CELO_ODIS_WARN_12")
        );
        assert!(
            OdisError::UnauthenticatedUser
                .error_code()
                .starts_with("CELO_ODIS_WARN_02")
        );
        assert!(
            OdisError::ExceededQuota
                .error_code()
                .starts_with("CELO_ODIS_WARN_03")
        );
        assert!(
            OdisError::SignatureComputationFailure
                .error_code()
                .starts_with("CELO_ODIS_ERR_05")
        );
        assert!(
            OdisError::DatabaseError
                .error_code()
                .starts_with("CELO_ODIS_ERR_03")
        );
        assert!(
            OdisError::KeyFetchError
                .error_code()
                .starts_with("CELO_ODIS_ERR_04")
        );
        assert!(
            OdisError::Unknown
                .error_code()
                .starts_with("CELO_ODIS_ERR_00")
        );
        assert!(
            OdisError::Timeout
                .error_code()
                .starts_with("CELO_ODIS_ERR_10")
        );
        assert!(
            OdisError::FullNodeError
                .error_code()
                .starts_with("CELO_ODIS_ERR_11")
        );
        assert!(
            OdisError::FailureToGetTotalQuota
                .error_code()
                .starts_with("CELO_ODIS_ERR_25")
        );
        assert!(
            OdisError::FailureToGetDek
                .error_code()
                .starts_with("CELO_ODIS_ERR_27")
        );
        assert!(
            OdisError::ApiUnavailable
                .error_code()
                .starts_with("CELO_ODIS_WARN_13")
        );
    }

    #[test]
    fn display_matches_error_code() {
        let err = OdisError::InvalidInput;
        assert_eq!(err.to_string(), err.error_code());
    }

    #[tokio::test]
    async fn into_response_produces_json_error() {
        use http_body_util::BodyExt;

        let response = OdisError::ExceededQuota.into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["success"], false);
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            json["error"],
            "CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota"
        );
    }
}
