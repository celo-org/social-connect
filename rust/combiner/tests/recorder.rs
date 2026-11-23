//! The recorder is global to the process, and the unit tests install one. This binary runs in a
//! process of its own, under `cargo test` as under nextest, so its first install is the first.

use metrics_exporter_prometheus::BuildError;
use odis_combiner::metrics::{FULL_NODE_LATENCY, install_recorder};

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

    assert!(
        matches!(
            install_recorder("combiner-test"),
            Err(BuildError::FailedToSetGlobalRecorder(_))
        ),
        "a second install must fail because a recorder is installed"
    );
}
