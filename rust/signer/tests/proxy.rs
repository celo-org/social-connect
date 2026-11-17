//! Full node lookups go through the proxy that the proxy variables name. The environment belongs
//! to the whole process, so this binary holds a single test: it sets the environment before it
//! starts its runtime, while no other thread of its own exists, under `cargo test` as under
//! nextest.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::Uri;
use odis_signer::account_service::ClientAccountService;
use odis_signer::config::Config;
use tokio::net::TcpListener;

/// Celo Sepolia's chain ID, 11142220, the default `CHAIN_ID`.
const CHAIN_ID_ANSWER: &str = r#"{"jsonrpc":"2.0","id":0,"result":"0xaa044c"}"#;

#[test]
fn full_node_lookups_go_through_the_proxy() {
    // Bound before the runtime starts, so that the variables can name it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    // SAFETY: this is the binary's only test, and it has started no thread yet.
    unsafe {
        // Only these variables, so the caller's environment cannot change the config or where
        // the calls go. A set `REQUEST_METHOD` would make the client ignore `HTTP_PROXY`.
        for (name, _) in std::env::vars_os() {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("KEYSTORE_TYPE", "Mock");
        // `.invalid` names never resolve, so only a call through the proxy gets an answer.
        std::env::set_var("BLOCKCHAIN_PROVIDER", "http://full-node.invalid");
    }
    let config = Config::from_env().unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let proxied = seen.clone();
    let chain_id = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let listener = TcpListener::from_std(listener).unwrap();
            // A proxy that answers every request itself, as the full node would.
            let proxy = Router::new().fallback(move |uri: Uri| async move {
                proxied.lock().unwrap().push(uri.to_string());
                CHAIN_ID_ANSWER
            });
            tokio::spawn(async move { axum::serve(listener, proxy).await.unwrap() });

            ClientAccountService::new(&config)
                .unwrap()
                .verify_chain_id()
                .await
        });

    assert_eq!(chain_id, Ok(()));
    assert_eq!(*seen.lock().unwrap(), ["http://full-node.invalid/"]);
}
