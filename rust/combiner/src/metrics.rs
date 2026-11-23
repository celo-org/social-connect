//! Metric names and the process-global Prometheus recorder. Names are the TS
//! combiner's (`apps/combiner/src/common/metrics.ts`); labels are snake_case.

use std::time::Duration;

use ::metrics::counter;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, because the recorder is global to the process.
    #[test]
    fn recorder_exports_ts_buckets_and_app_label_and_refuses_a_second_install() {
        let handle = install_recorder("combiner-test").expect("first install");
        ::metrics::histogram!(FULL_NODE_LATENCY, "code_segment" => "x").record(0.05);
        let text = handle.render();

        assert!(
            text.contains(r#"combiner_connection_closed_total{app="combiner-test"} 0"#),
            "{text}"
        );
        for bound in ["0.001", "0.01", "0.1", "0.5", "1", "2", "5", "10", "+Inf"] {
            let line = format!(
                r#"combiner_full_node_latency_bucket{{app="combiner-test",code_segment="x",le="{bound}"}}"#
            );
            assert!(text.contains(&line), "missing {line} in {text}");
        }
        assert!(!text.contains(r#"le="0.2""#), "{text}");

        assert!(install_recorder("combiner-test").is_err());
    }
}
