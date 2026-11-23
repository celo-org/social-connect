//! Metric names and the process-global Prometheus recorder. Names are the TS
//! combiner's (`apps/combiner/src/common/metrics.ts`); labels are snake_case.

use std::time::{Duration, Instant};

use ::metrics::{counter, histogram};
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder, PrometheusHandle};

// -- Counters --

pub const REQUESTS: &str = "combiner_requests_total";
pub const RESPONSES: &str = "combiner_responses_total";
pub const ERRORS: &str = "combiner_errors_total";
pub const BLOCKCHAIN_ERRORS: &str = "combiner_blockchain_errors_total";
pub const BLS_COMPUTE_ERRORS: &str = "combiner_bls_compute_errors_total";
pub const ENDPOINT_HANDLER_ERRORS: &str = "combiner_endpoint_handler_errors_total";
pub const NOT_ENOUGH_SIG_ERRORS: &str = "combiner_not_enough_sig_errors_total";
pub const SIG_REQUEST_ERRORS: &str = "combiner_sig_request_errors_total";
pub const SIG_RESPONSE_ERRORS: &str = "combiner_sig_response_errors_total";
pub const SIG_INCONSISTENCY_ERRORS: &str = "combiner_sig_inconsistency_errors_total";
pub const SIG_RESPONSES: &str = "combiner_sig_response_total";
pub const UNKNOWN_ERRORS: &str = "combiner_unknown_errors_total";
pub const WARNINGS: &str = "combiner_warnings_total";
pub const CONNECTION_CLOSED: &str = "combiner_connection_closed_total";

// -- Histograms --

pub const ENDPOINT_LATENCY: &str = "combiner_endpoint_latency";
pub const FULL_NODE_LATENCY: &str = "combiner_full_node_latency";
pub const SIGNER_LATENCY: &str = "combiner_signer_latency";
pub const SIGNATURE_AGGREGATION_LATENCY: &str = "combiner_signature_aggregation_latency";
pub const SIGNER_TAIL_LATENCY: &str = "signer_tail_latency";

/// Bucket bounds of every histogram, the TS combiner's `buckets`.
pub const HISTOGRAM_BUCKETS: &[f64] = &[0.001, 0.01, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0];

/// How often histogram samples are folded into buckets, the exporter's default.
const UPKEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Installs the process-global recorder with the TS buckets and the global `app`
/// label, and exports `CONNECTION_CLOSED` at 0, so that `rate()` and
/// `increase()` see the first event after a restart. Setting the buckets makes
/// every histogram a Prometheus histogram; without them the exporter writes
/// summaries. Fails if a recorder is already installed.
pub fn install_recorder(service_name: &str) -> Result<PrometheusHandle, BuildError> {
    let handle = PrometheusBuilder::new()
        .set_buckets(HISTOGRAM_BUCKETS)?
        .add_global_label("app", service_name)
        .install_recorder()?;
    counter!(CONNECTION_CLOSED).increment(0);
    Ok(handle)
}

/// Runs the recorder's upkeep on an interval.
///
/// The recorder keeps every histogram sample in memory until it is folded into
/// buckets, which happens when `/metrics` is rendered or upkeep runs. Without
/// this task, memory grows with every request while nothing scrapes the
/// combiner. The exporter leaves this to the caller of `install_recorder`.
pub fn spawn_upkeep_task(handle: PrometheusHandle) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(UPKEEP_INTERVAL).await;
            handle.run_upkeep();
        }
    });
}

/// The routes the request metrics count, as the TS combiner counts only its API
/// endpoints. Scrapes, probes and unknown paths would otherwise fill the request
/// and error panels.
const COUNTED_ROUTES: &[&str] = &[
    "/sign",
    "/quotaStatus",
    "/domain/sign",
    "/domain/quotaStatus",
    "/domain/disable",
];

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

/// Axum middleware that records per-request metrics: `REQUESTS`, `RESPONSES`
/// (with the status code), `ENDPOINT_LATENCY` and `CONNECTION_CLOSED`, all
/// labelled with the route.
pub async fn http_metrics_layer(request: Request, next: Next) -> Response {
    let Some(path) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .filter(|path| COUNTED_ROUTES.contains(&path.as_str()))
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
    histogram!(ENDPOINT_LATENCY, "endpoint" => path).record(elapsed);

    response
}
