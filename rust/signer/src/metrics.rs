use std::sync::OnceLock;
use std::time::Instant;

use ::metrics::{counter, histogram};
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

// -- Counters --

pub const REQUESTS: &str = "requests";
pub const RESPONSES: &str = "responses";
pub const DATABASE_ERRORS: &str = "database_errors";
pub const BLOCKCHAIN_ERRORS: &str = "blockchain_errors";
pub const SIGNATURE_COMPUTATION_ERRORS: &str = "signature_computation_errors";
pub const DUPLICATE_REQUESTS: &str = "duplicate_requests";
pub const REQUESTS_WITH_WALLET_ADDRESS: &str = "requests_with_wallet_address";

// -- Histograms --

pub const RESPONSE_LATENCY: &str = "signature_endpoint_latency";
pub const FULL_NODE_LATENCY: &str = "full_node_latency";
pub const DB_OPS_LATENCY: &str = "db_ops_instrumentation";
pub const USER_REMAINING_QUOTA: &str = "user_remaining_quota_at_request";

/// Custom histogram buckets matching the TS signer configuration.
/// TS uses: [0.001, 0.01, 0.05, 0.1, 0.3, 0.5, 0.7, 1, 2, 3, 5, 10]
const HISTOGRAM_BUCKETS: &[f64] = &[
    0.001, 0.01, 0.05, 0.1, 0.3, 0.5, 0.7, 1.0, 2.0, 3.0, 5.0, 10.0,
];

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the Prometheus metrics recorder with custom histogram buckets.
/// Returns a handle for rendering the `/metrics` endpoint.
///
/// Safe to call multiple times — the recorder is installed only once.
pub fn install_recorder() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Full(RESPONSE_LATENCY.to_string()),
                    HISTOGRAM_BUCKETS,
                )
                .unwrap()
                .set_buckets_for_metric(
                    Matcher::Full(FULL_NODE_LATENCY.to_string()),
                    HISTOGRAM_BUCKETS,
                )
                .unwrap()
                .set_buckets_for_metric(
                    Matcher::Full(DB_OPS_LATENCY.to_string()),
                    HISTOGRAM_BUCKETS,
                )
                .unwrap()
                .install_recorder()
                .expect("failed to install Prometheus recorder")
        })
        .clone()
}

/// Axum middleware that records per-request metrics:
/// - `requests` counter (label: endpoint)
/// - `responses` counter (labels: endpoint, status_code)
/// - `signature_endpoint_latency` histogram (label: endpoint)
pub async fn http_metrics_layer(request: Request, next: Next) -> Response {
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    counter!(REQUESTS, "endpoint" => path.clone()).increment(1);

    let start = Instant::now();
    let response = next.run(request).await;
    let elapsed = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();
    counter!(RESPONSES, "endpoint" => path.clone(), "status_code" => status).increment(1);
    histogram!(RESPONSE_LATENCY, "endpoint" => path).record(elapsed);

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_name_constants_are_non_empty() {
        let names = [
            REQUESTS,
            RESPONSES,
            DATABASE_ERRORS,
            BLOCKCHAIN_ERRORS,
            SIGNATURE_COMPUTATION_ERRORS,
            DUPLICATE_REQUESTS,
            REQUESTS_WITH_WALLET_ADDRESS,
            RESPONSE_LATENCY,
            FULL_NODE_LATENCY,
            DB_OPS_LATENCY,
            USER_REMAINING_QUOTA,
        ];
        for name in names {
            assert!(!name.is_empty());
        }
    }
}
