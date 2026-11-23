//! The combiner binary refuses to start with the message written for the operator.

use std::process::{Command, Output};

/// Runs the combiner binary with only `variables` set, from the system's temporary directory,
/// so neither the caller's environment nor a `.env` file of the repository reaches it.
fn run_combiner(variables: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_odis-combiner"))
        .env_clear()
        .envs(variables.iter().copied())
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap()
}

/// `PNP_KEYS_VERSIONS` with the first key version of the combine vectors.
fn key_versions() -> String {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("data/combine_vectors.json")).unwrap();
    let key = &vectors["keys"][0];
    serde_json::json!([{
        "keyVersion": key["keyVersion"],
        "threshold": key["threshold"],
        "polynomial": key["polynomial"],
        "pubKey": key["pubKey"],
    }])
    .to_string()
}

#[test]
fn config_error_is_printed_with_its_message() {
    let output = run_combiner(&[]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "failed to load config: missing required env var: PNP_ODIS_SERVICES_SIGNERS\n"
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
}

#[test]
fn startup_check_error_is_logged_with_its_message() {
    let key_versions = key_versions();
    let output = run_combiner(&[
        (
            "PNP_ODIS_SERVICES_SIGNERS",
            r#"[{"url":"http://s0"},{"url":"http://s1"},{"url":"http://s2"}]"#,
        ),
        ("PNP_KEYS_VERSIONS", &key_versions),
        ("BLOCKCHAIN_PROVIDER", "http://127.0.0.1:9"),
        ("CHAIN_ID", "1"),
        ("LOG_FORMAT", "json"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let error_lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|line: &serde_json::Value| line["level"] == "ERROR")
        .collect();
    assert_eq!(error_lines.len(), 1, "{stdout}");
    assert_eq!(
        error_lines[0]["message"],
        "failed to build router: CHAIN_ID 1 is not supported: the Accounts contract address is \
         known only for 42220 (Celo mainnet) and 11142220 (Celo Sepolia)"
    );
}
