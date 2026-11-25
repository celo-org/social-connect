//! The Rust combiner against the TS combiner of the release `odis-combiner-4.0.5`, run from its
//! image. Both get the same raw requests, each in front of Rust signers of its own, and their
//! answers are compared request by request (`comparisons.rs`).
//!
//! Needs Docker. Without it the comparison is skipped, unless `ODIS_REQUIRE_DIFFERENTIAL_TESTS`
//! is set, as CI sets it: then a missing Docker fails it.

#[path = "../common/mod.rs"]
mod common;
mod comparisons;
mod harness;

#[test]
fn every_comparison_has_a_name_of_its_own() {
    let names: Vec<String> = comparisons::all()
        .into_iter()
        .map(|comparison| comparison.name)
        .collect();
    let duplicated: Vec<&String> = names
        .iter()
        .enumerate()
        .filter(|&(index, name)| names.iter().position(|other| other == name) != Some(index))
        .map(|(_, name)| name)
        .collect();
    assert_eq!(duplicated, Vec::<&String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn rust_and_ts_combiners_answer_alike() {
    let metrics = odis_signer::metrics::install_recorder();
    let full_node = harness::stub_full_node().await;
    let (rust_url, _rust_signers) = harness::rust_fleet(&full_node, metrics).await;
    let ts_signers = harness::differential_signers().await;
    let ts_signer_urls: Vec<String> = ts_signers.iter().map(|signer| signer.url.clone()).collect();
    let Some(container) = harness::ts_combiner(&ts_signer_urls, &full_node).await else {
        return;
    };
    let ts_url = harness::ts_combiner_url(&container).await;

    let mut failures = Vec::new();
    for comparison in comparisons::all() {
        let request = (comparison.request)(&harness::test_user(&comparison.name));
        let ts = harness::send(&ts_url, &request).await;
        let rust = harness::send(&rust_url, &request).await;
        let failure = match (ts, rust) {
            (Ok(ts), Ok(rust)) => harness::difference(&comparison.expected, &ts, &rust),
            (ts, rust) => {
                let errors: Vec<String> = [("TS", ts.err()), ("Rust", rust.err())]
                    .into_iter()
                    .filter_map(|(side, error)| error.map(|error| format!("{side}: {error}")))
                    .collect();
                Some(errors.join("; "))
            }
        };
        if let Some(failure) = failure {
            failures.push(format!("{}: {failure}", comparison.name));
        }
    }
    if !failures.is_empty() {
        eprintln!(
            "The TS combiner's log:\n{}",
            harness::container_logs(&container).await
        );
    }
    assert!(
        failures.is_empty(),
        "{} comparisons failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
