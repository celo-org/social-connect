use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ::metrics::{counter, histogram};
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

// -- Counters --

pub const REQUESTS: &str = "requests";
pub const RESPONSES: &str = "responses";
pub const DATABASE_ERRORS: &str = "database_errors";
pub const BLOCKCHAIN_ERRORS: &str = "blockchain_errors";
pub const SIGNATURE_COMPUTATION_ERRORS: &str = "signature_computation_errors";
pub const DUPLICATE_REQUESTS: &str = "duplicate_requests";
pub const REQUESTS_WITH_WALLET_ADDRESS: &str = "requests_with_wallet_address";
pub const TIMEOUTS: &str = "timeouts";
pub const ERRORS_CAUGHT_IN_ENDPOINT_HANDLER: &str = "errors_caught_in_endpoint_handler";
pub const CONNECTION_CLOSED: &str = "connection_closed";

/// Counters without labels. They are exported at 0 from startup, as in the TS
/// signer. A counter that first appears at 1 has no earlier sample, so
/// `rate()` and `increase()` would miss the first event after every restart.
const UNLABELLED_COUNTERS: &[&str] = &[
    BLOCKCHAIN_ERRORS,
    SIGNATURE_COMPUTATION_ERRORS,
    DUPLICATE_REQUESTS,
    REQUESTS_WITH_WALLET_ADDRESS,
    TIMEOUTS,
    ERRORS_CAUGHT_IN_ENDPOINT_HANDLER,
    CONNECTION_CLOSED,
];

// -- Histograms --

pub const RESPONSE_LATENCY: &str = "signature_endpoint_latency";
pub const FULL_NODE_LATENCY: &str = "full_node_latency";
pub const DB_OPS_LATENCY: &str = "db_ops_instrumentation";
pub const USER_REMAINING_QUOTA: &str = "user_remaining_quota_at_request";

/// Bucket bounds of every histogram, the same list as in the TS signer
/// (`buckets` in `odis-signer-4.0.6:apps/signer/src/common/metrics.ts`).
const HISTOGRAM_BUCKETS: &[f64] = &[
    0.001, 0.01, 0.1, 0.2, 0.3, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.2, 1.4, 1.6, 1.8, 2.0, 2.3, 2.6,
    2.9, 3.5, 4.0, 5.0, 10.0,
];

/// How often histogram samples are folded into buckets, the exporter's default.
const UPKEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Routes that serve operators, not clients, and are left out of the request
/// metrics.
const UNCOUNTED_ROUTES: [&str; 2] = ["/status", "/metrics"];

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Runs the recorder's upkeep on an interval.
///
/// The recorder keeps every histogram sample in memory until it is folded into
/// buckets, which happens when `/metrics` is rendered or upkeep runs. Without
/// this task, memory would grow with every request while nothing scrapes the
/// signer. The exporter leaves this to callers of `install_recorder`.
pub fn spawn_upkeep_task(handle: PrometheusHandle) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(UPKEEP_INTERVAL).await;
            handle.run_upkeep();
        }
    });
}

/// Install the Prometheus metrics recorder. Setting the buckets makes every
/// histogram a Prometheus histogram; without them the exporter writes summaries.
/// Returns a handle for rendering the `/metrics` endpoint.
///
/// Safe to call multiple times — the recorder is installed only once.
pub fn install_recorder() -> PrometheusHandle {
    PROMETHEUS_HANDLE
        .get_or_init(|| {
            let handle = PrometheusBuilder::new()
                .set_buckets(HISTOGRAM_BUCKETS)
                .unwrap()
                .install_recorder()
                .expect("failed to install Prometheus recorder");
            for name in UNLABELLED_COUNTERS {
                counter!(*name).increment(0);
            }
            handle
        })
        .clone()
}

/// Counts a request that ends without a response. The server drops a request's
/// future when its caller closes the connection, so a guard that is dropped
/// before the response is ready means the caller has gone.
struct ConnectionClosedGuard {
    answered: bool,
}

impl Drop for ConnectionClosedGuard {
    fn drop(&mut self) {
        if !self.answered {
            counter!(CONNECTION_CLOSED).increment(1);
        }
    }
}

/// Axum middleware that records per-request metrics:
/// - `requests` counter (label: endpoint)
/// - `responses` counter (labels: endpoint, status_code)
/// - `signature_endpoint_latency` histogram (label: endpoint)
/// - `connection_closed` counter
pub async fn http_metrics_layer(request: Request, next: Next) -> Response {
    // Like the TS signer, only the API endpoints are counted. Scrapes, probes
    // and requests for paths that do not exist would otherwise fill the request
    // and error panels.
    let Some(path) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .filter(|path| !UNCOUNTED_ROUTES.contains(&path.as_str()))
    else {
        return next.run(request).await;
    };

    counter!(REQUESTS, "endpoint" => path.clone()).increment(1);

    let mut guard = ConnectionClosedGuard { answered: false };
    let start = Instant::now();
    let response = next.run(request).await;
    let elapsed = start.elapsed().as_secs_f64();
    guard.answered = true;

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
            TIMEOUTS,
            ERRORS_CAUGHT_IN_ENDPOINT_HANDLER,
            CONNECTION_CLOSED,
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
