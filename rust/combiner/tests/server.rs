//! The combiner's routes, CORS, transport answers and startup check, through the real router.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use alloy::primitives::Address;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use odis_combiner::config::Config;
use odis_combiner::dek::{DekLookup, DekLookupError, FullNodeError};
use odis_combiner::metrics::install_recorder;
use odis_combiner::server::{StartupError, build_router, build_router_with_services, panic_layer};
use odis_signer::config::LogFormat;
use tower::ServiceExt;

const SEPOLIA: u64 = 11142220;
const MAINNET: u64 = 42220;
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct FixedDek;

#[async_trait]
impl DekLookup for FixedDek {
    async fn dek(&self, _account: Address) -> Result<String, DekLookupError> {
        Ok(String::new())
    }
}

/// The recorder is global to the process, so every test shares one.
fn recorder() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| install_recorder("combiner-test").expect("install recorder"))
        .clone()
}

fn config_for(provider: &str) -> Config {
    Config {
        server_port: 0,
        service_name: "combiner-test".to_string(),
        signers: vec![],
        signer_timeout: Duration::from_secs(1),
        key_versions: vec![],
        current_key_version: 1,
        blockchain_provider: provider.to_string(),
        blockchain_api_key: None,
        chain_id: SEPOLIA,
        full_node_timeout: Duration::from_secs(5),
        full_node_retry_count: 0,
        full_node_retry_delay: Duration::from_millis(1),
        log_format: LogFormat::Text,
    }
}

fn router() -> axum::Router {
    build_router_with_services(
        config_for("http://127.0.0.1:1"),
        Arc::new(FixedDek),
        recorder(),
    )
    .expect("router")
}

async fn send(method: Method, path: &str, extra: &[(&str, &str)], body: Vec<u8>) -> Response<Body> {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    router()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn get(path: &str) -> Response<Body> {
    send(Method::GET, path, &[], vec![]).await
}

async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

fn assert_cors(headers: &HeaderMap, what: &str) {
    assert_eq!(headers["access-control-allow-origin"], "*", "{what}");
    assert_eq!(
        headers["access-control-allow-methods"], "GET, POST, OPTIONS",
        "{what}"
    );
    assert_eq!(
        headers["access-control-allow-headers"],
        "Origin, X-Requested-With, Content-Type, Accept, Authorization, odis-key-version",
        "{what}"
    );
}

#[tokio::test]
async fn every_answer_carries_the_cors_headers() {
    let oversized = send(
        Method::POST,
        "/sign",
        &[("content-length", "300000")],
        vec![b'a'; 300_000],
    )
    .await;
    let answers = [
        ("200 /status", StatusCode::OK, get("/status").await),
        ("404 /nope", StatusCode::NOT_FOUND, get("/nope").await),
        (
            "405 GET /sign",
            StatusCode::METHOD_NOT_ALLOWED,
            get("/sign").await,
        ),
        ("413 POST /sign", StatusCode::PAYLOAD_TOO_LARGE, oversized),
        (
            "503 /domain/sign",
            StatusCode::SERVICE_UNAVAILABLE,
            send(Method::POST, "/domain/sign", &[], vec![]).await,
        ),
    ];
    for (what, status, response) in answers {
        assert_eq!(response.status(), status, "{what}");
        assert_cors(response.headers(), what);
    }
}

#[tokio::test]
async fn preflight_is_answered_with_200_and_no_body() {
    for path in ["/sign", "/nope"] {
        let response = send(
            Method::OPTIONS,
            path,
            &[
                ("origin", "https://example.org"),
                ("access-control-request-method", "POST"),
            ],
            vec![],
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_cors(response.headers(), path);
        assert_eq!(body_bytes(response).await, b"", "{path}");
    }
}

#[tokio::test]
async fn routing_answers_follow_axum() {
    for path in ["/SIGN", "/sign/", "/nope"] {
        let response = get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_bytes(response).await, b"", "{path}");
    }

    let response = get("/sign").await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "POST");
    assert_eq!(body_bytes(response).await, b"");
}

#[tokio::test]
async fn status_reports_the_workspace_version() {
    let response = get("/status").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_bytes(response).await,
        format!(r#"{{"version":"{VERSION}"}}"#).into_bytes()
    );
}

#[tokio::test]
async fn domain_routes_answer_warn_13() {
    let expected = format!(
        r#"{{"success":false,"version":"{VERSION}","error":"CELO_ODIS_WARN_13 BAD_INPUT API is unavailable"}}"#
    );
    for path in ["/domain/sign", "/domain/quotaStatus", "/domain/disable"] {
        let response = send(Method::POST, path, &[], b"{}".to_vec()).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert!(response.headers().get("x-powered-by").is_none(), "{path}");
        assert!(response.headers().get(header::ETAG).is_none(), "{path}");
        assert_eq!(body_bytes(response).await, expected.as_bytes(), "{path}");
    }
}

/// A full node that answers `eth_chainId` and nothing else.
async fn spawn_full_node(chain_id: u64) -> String {
    let app = axum::Router::new().fallback(
        move |axum::Json(request): axum::Json<serde_json::Value>| async move {
            axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": format!("0x{chain_id:x}"),
            }))
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn build_router_refuses_a_full_node_on_another_chain() {
    let url = spawn_full_node(MAINNET).await;

    let result = build_router(config_for(&url), recorder()).await;

    assert!(
        matches!(
            result,
            Err(StartupError::FullNode(FullNodeError::ChainIdMismatch {
                configured: SEPOLIA,
                full_node: MAINNET
            }))
        ),
        "{:?}",
        result.err()
    );
}

/// The value of the exported series that starts with `series`, or 0 if it is not exported yet.
fn counter_value(series: &str) -> u64 {
    recorder()
        .render()
        .lines()
        .find_map(|line| line.strip_prefix(series)?.trim().parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn only_the_api_routes_are_counted() {
    let requests = r#"combiner_requests_total{app="combiner-test",endpoint="/sign"} "#;
    let not_allowed =
        r#"combiner_responses_total{app="combiner-test",endpoint="/sign",status_code="405"} "#;
    let (requests_before, not_allowed_before) =
        (counter_value(requests), counter_value(not_allowed));

    get("/sign").await;
    get("/nope").await;
    get("/status").await;

    assert_eq!(counter_value(requests), requests_before + 1);
    assert_eq!(counter_value(not_allowed), not_allowed_before + 1);
    let text = recorder().render();
    for route in ["/nope", "/status"] {
        assert!(!text.contains(&format!(r#"endpoint="{route}""#)), "{text}");
    }
}

#[tokio::test]
async fn domain_routes_count_the_unavailable_warning() {
    let warning = r#"combiner_warnings_total{app="combiner-test",endpoint="/domain/sign",warning_type="CELO_ODIS_WARN_13 BAD_INPUT API is unavailable"} "#;
    let before = counter_value(warning);

    send(Method::POST, "/domain/sign", &[], vec![]).await;

    assert_eq!(counter_value(warning), before + 1);
}

#[tokio::test]
async fn a_panicking_handler_is_answered_and_counted() {
    let series = [
        r#"combiner_endpoint_handler_errors_total{app="combiner-test",endpoint="/boom"} "#,
        r#"combiner_unknown_errors_total{app="combiner-test",endpoint="/boom"} "#,
        r#"combiner_errors_total{app="combiner-test",endpoint="/boom",error_type="CELO_ODIS_ERR_00 Something went wrong"} "#,
    ];
    let before = series.map(counter_value);
    recorder();
    let app = axum::Router::new()
        .route("/boom", axum::routing::post(|| async { panic!("boom") }))
        .layer(axum::middleware::from_fn(panic_layer));

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/boom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_bytes(response).await,
        format!(
            r#"{{"success":false,"version":"{VERSION}","error":"CELO_ODIS_ERR_00 Something went wrong"}}"#
        )
        .into_bytes()
    );
    assert_eq!(series.map(counter_value), before.map(|n| n + 1));
}
