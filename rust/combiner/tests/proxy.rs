//! Signer calls and full node lookups go through the proxy that the proxy variables name. The
//! variables belong to the whole process, so this binary holds a single test: it sets them before
//! it starts its runtime, while no other thread of its own exists, under `cargo test` as under
//! nextest.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::Uri;
use odis_combiner::config::{Config, SignerEntry};
use odis_combiner::dek::FullNodeDekLookup;
use odis_combiner::quota::SignerQuota;
use odis_combiner::signers::{
    Flow, SignerEndpoint, SignerSuccess, ThresholdCall, build_client, call_signers,
};
use odis_signer::config::LogFormat;
use tokio::net::TcpListener;

const QUOTA_ANSWER: &str =
    r#"{"success":true,"version":"1.0.0","performedQueryCount":2,"totalQuota":10}"#;
/// Celo Sepolia's chain ID, 11142220.
const CHAIN_ID_ANSWER: &str = r#"{"jsonrpc":"2.0","id":0,"result":"0xaa044c"}"#;

/// Points the HTTP proxy variables at `proxy` and clears the others, so the caller's
/// environment cannot change where the calls go.
///
/// # Safety
///
/// No other thread may run while it changes the environment.
unsafe fn set_proxy_variables(proxy: &str) {
    unsafe {
        for name in ["HTTP_PROXY", "http_proxy"] {
            std::env::set_var(name, proxy);
        }
        // A set `REQUEST_METHOD` makes the client ignore `HTTP_PROXY`, as in CGI.
        for name in [
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
            "REQUEST_METHOD",
        ] {
            std::env::remove_var(name);
        }
    }
}

#[test]
fn signer_calls_and_full_node_lookups_go_through_the_proxy() {
    // Bound before the runtime starts, so that the variables can name it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    // SAFETY: this is the binary's only test, and it has started no thread yet.
    unsafe { set_proxy_variables(&proxy) };

    let seen = Arc::new(Mutex::new(Vec::new()));
    let proxied = seen.clone();
    let (outcome, chain_id) = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let listener = TcpListener::from_std(listener).unwrap();
            // A proxy that answers every request itself, as the signer or the full node would.
            let proxy = Router::new().fallback(move |uri: Uri| async move {
                let answer = if uri.path() == "/quotaStatus" {
                    QUOTA_ANSWER
                } else {
                    CHAIN_ID_ANSWER
                };
                proxied.lock().unwrap().push(uri.to_string());
                answer
            });
            tokio::spawn(async move { axum::serve(listener, proxy).await.unwrap() });

            // `.invalid` names never resolve, so only a call through the proxy gets an answer.
            let client = build_client().unwrap();
            let signers = [SignerEntry {
                url: "http://signer.invalid".to_string(),
                share_index: None,
            }];
            let call = ThresholdCall {
                client: &client,
                signers: &signers,
                endpoint: SignerEndpoint::QuotaStatus,
                key_version: 1,
                threshold: 1,
                authorization: "0xc0ffee",
                body: Bytes::from_static(b"{}"),
                deadline: Duration::from_secs(10),
            };
            let outcome = call_signers(call, |_, _| Flow::Continue).await;

            let config = Config {
                server_port: 0,
                service_name: "combiner-test".to_string(),
                signers: vec![],
                signer_timeout: Duration::from_secs(1),
                key_versions: vec![],
                current_key_version: 1,
                blockchain_provider: "http://full-node.invalid".to_string(),
                blockchain_api_key: None,
                chain_id: 11142220,
                full_node_timeout: Duration::from_secs(5),
                full_node_retry_count: 0,
                full_node_retry_delay: Duration::from_millis(1),
                log_format: LogFormat::Text,
            };
            let chain_id = FullNodeDekLookup::new(&config)
                .unwrap()
                .verify_chain_id()
                .await;
            (outcome, chain_id)
        });

    let expected = SignerSuccess {
        quota: SignerQuota {
            version: "1.0.0".to_string(),
            performed_query_count: 2,
            total_quota: 10,
            warnings: None,
        },
        signature: None,
    };
    assert_eq!(outcome.successes, vec![(0, expected)]);
    assert!(chain_id.is_ok(), "{chain_id:?}");
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "http://signer.invalid/quotaStatus",
            "http://full-node.invalid/"
        ]
    );
}
