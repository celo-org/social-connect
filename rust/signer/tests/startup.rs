//! The signer binary refuses to start with the message written for the operator.

use std::process::{Command, Output};

/// Runs the signer binary with only `variables` set, from the system's temporary directory, so
/// neither the caller's environment nor a `.env` file of the repository reaches it.
fn run_signer(variables: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_odis-signer"))
        .env_clear()
        .envs(variables.iter().copied())
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap()
}

#[test]
fn config_error_is_printed_with_its_message() {
    let output = run_signer(&[]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "failed to load config: missing required env var: KEYSTORE_TYPE\n"
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "");
}

#[test]
fn startup_check_error_is_logged_with_its_message() {
    let output = run_signer(&[
        ("KEYSTORE_TYPE", "GoogleSecretManager"),
        ("LOG_FORMAT", "json"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let errors: Vec<String> = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["level"] == "ERROR")
        .map(|line| line["message"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        errors,
        [
            "a persistent database is required when keystore_type is not Mock; set DATABASE_URL. \
             An in-memory database forgets all used quota on restart.",
            "failed to build router: CELO_ODIS_ERR_03 DB_ERR Failed to get database entry",
        ],
        "{stdout}"
    );
}
