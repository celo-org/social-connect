//! Calls to the signers: one request to every signer at once under one deadline, each answer
//! classified as the TS combiner does (`apps/combiner/src/common/combine.ts`), and an early stop
//! once the threshold is out of reach or the caller has what it needs.

use std::collections::BTreeSet;
use std::error::Error;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use futures::stream::{FuturesUnordered, StreamExt};
use metrics::{counter, histogram};
use odis_signer::types::KEY_VERSION_HEADER;
use serde::de::IgnoredAny;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::config::SignerEntry;
use crate::errors::{
    CANCELLED_REQUEST_TO_SIGNER, INCONSISTENT_SIGNER_RESPONSES, INVALID_KEY_VERSION_RESPONSE,
    INVALID_SIGNER_RESPONSE, SIGNER_REQUEST_ERROR, SIGNER_RESPONSE_FAILED_WITH_OK_STATUS,
    TIMEOUT_FROM_SIGNER,
};
use crate::metrics::{
    SIG_INCONSISTENCY_ERRORS, SIG_REQUEST_ERRORS, SIG_RESPONSE_ERRORS, SIG_RESPONSES,
    SIGNER_LATENCY, WARNINGS,
};
use crate::quota::{SignerQuota, majority_status};
use crate::request::key_version_header;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignerEndpoint {
    Sign,
    QuotaStatus,
}

impl SignerEndpoint {
    /// The path on the signer. It is also the combiner's route and the `endpoint` label.
    pub fn path(self) -> &'static str {
        match self {
            Self::Sign => "/sign",
            Self::QuotaStatus => "/quotaStatus",
        }
    }
}

/// The longest signer answer the combiner reads. A longer one is refused as invalid, so that a
/// faulty signer cannot make the combiner buffer without limit.
pub const MAX_SIGNER_RESPONSE_BYTES: usize = 64 * 1024;

/// The client for every signer call. It keeps connections alive and reuses them, as the TS
/// combiner's agents do. It speaks only HTTP/1.1, as the TS combiner does. It goes through the
/// proxy that the proxy environment variables name, which node-fetch ignores. It asks for no
/// compression even if another crate turns reqwest's decompression features on, so the size
/// limit counts the bytes that arrived.
pub fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .http1_only()
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .build()
}

/// A signer's 2xx answer that matches the TS response schema.
#[derive(Debug, Clone, PartialEq)]
pub struct SignerSuccess {
    pub quota: SignerQuota,
    /// The partial signature; `Some` for `/sign`.
    pub signature: Option<String>,
}

/// Whether the calls still running should go on after a success.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flow {
    Continue,
    Stop,
}

/// One request, sent to every signer.
pub struct ThresholdCall<'a> {
    pub client: &'a reqwest::Client,
    pub signers: &'a [SignerEntry],
    pub endpoint: SignerEndpoint,
    /// Sent in `odis-key-version`; a `/sign` answer must carry it back.
    pub key_version: u32,
    pub threshold: usize,
    /// The client's `Authorization` header, forwarded as it is.
    pub authorization: &'a str,
    /// The canonical body.
    pub body: Bytes,
    /// One limit for all calls, body reads included.
    pub deadline: Duration,
}

#[derive(Debug)]
pub struct ThresholdOutcome {
    /// Signer index and answer, in arrival order.
    pub successes: Vec<(usize, SignerSuccess)>,
    /// The most frequent status among the non-2xx JSON answers, the lower one on a tie.
    pub majority_status: Option<u16>,
}

/// Why a call to one signer brought no success.
enum Failure {
    /// A non-2xx answer whose body is JSON. Its status enters the majority status.
    Status(u16),
    /// Any other failure. Counted as `ERR_09`, as in TS. `reason` is the TS error that names
    /// it in the log.
    NoStatus {
        reason: &'static str,
        detail: String,
    },
}

/// Calls every signer at once and hands each success to `on_success` as it arrives.
///
/// The calls end when all have ended, when `on_success` says `Stop`, when so many have failed
/// that `threshold` successes are out of reach, or at the deadline. Calls still running are
/// dropped, which closes their connections. A call cut off at the deadline is counted as
/// `ERR_10`, one cut off by a stop as `WARN_08`, and either gets a latency sample if its
/// answer's headers had not arrived (see `LatencyTimer`).
pub async fn call_signers(
    call: ThresholdCall<'_>,
    mut on_success: impl FnMut(usize, &SignerSuccess) -> Flow,
) -> ThresholdOutcome {
    let endpoint = call.endpoint.path();
    let call = &call;
    let mut pending: FuturesUnordered<_> = call
        .signers
        .iter()
        .enumerate()
        .map(|(index, signer)| async move { (index, call_signer(call, signer).await) })
        .collect();
    let mut ended = vec![false; call.signers.len()];
    let deadline = tokio::time::sleep(call.deadline);
    tokio::pin!(deadline);

    let mut successes = Vec::new();
    let mut statuses = Vec::new();
    let mut errors = 0;
    loop {
        tokio::select! {
            biased;
            next = pending.next() => {
                let Some((index, result)) = next else { break };
                ended[index] = true;
                let url = &call.signers[index].url;
                match result {
                    Ok(success) => {
                        let flow = on_success(index, &success);
                        successes.push((index, success));
                        if flow == Flow::Stop {
                            cancel_running(call, &ended);
                            break;
                        }
                    }
                    Err(failure) => {
                        match failure {
                            Failure::Status(status) => statuses.push(status),
                            Failure::NoStatus { reason, detail } => {
                                counter!(
                                    SIG_REQUEST_ERRORS,
                                    "signer" => url.clone(),
                                    "endpoint" => endpoint,
                                    "error_type" => SIGNER_REQUEST_ERROR,
                                )
                                .increment(1);
                                error!(signer = %url, %detail, "{reason}");
                            }
                        }
                        errors += 1;
                        if call.signers.len() - errors < call.threshold {
                            warn!(
                                "Not possible to reach a threshold of signer responses. \
                                 Failing fast"
                            );
                            cancel_running(call, &ended);
                            break;
                        }
                    }
                }
            }
            () = &mut deadline => {
                for signer in running(call.signers, &ended) {
                    counter!(
                        SIG_REQUEST_ERRORS,
                        "signer" => signer.url.clone(),
                        "endpoint" => endpoint,
                        "error_type" => TIMEOUT_FROM_SIGNER,
                    )
                    .increment(1);
                    error!(signer = %signer.url, "{TIMEOUT_FROM_SIGNER}");
                }
                break;
            }
        }
    }
    drop(pending);

    if statuses.iter().collect::<BTreeSet<_>>().len() > 1 {
        counter!(SIG_INCONSISTENCY_ERRORS, "endpoint" => endpoint).increment(1);
        error!(?statuses, "{INCONSISTENT_SIGNER_RESPONSES}");
    }
    ThresholdOutcome {
        successes,
        majority_status: majority_status(&statuses),
    }
}

/// The signers whose calls have not ended.
fn running<'a>(
    signers: &'a [SignerEntry],
    ended: &'a [bool],
) -> impl Iterator<Item = &'a SignerEntry> {
    signers
        .iter()
        .zip(ended)
        .filter(|(_, ended)| !**ended)
        .map(|(signer, _)| signer)
}

fn cancel_running(call: &ThresholdCall<'_>, ended: &[bool]) {
    for signer in running(call.signers, ended) {
        counter!(
            WARNINGS,
            "endpoint" => call.endpoint.path(),
            "warning_type" => CANCELLED_REQUEST_TO_SIGNER,
        )
        .increment(1);
        info!(signer = %signer.url, "{CANCELLED_REQUEST_TO_SIGNER}");
    }
}

/// Times one call for `SIGNER_LATENCY` as TS's `newMeter` times `fetch`, whose timer ends in a
/// `finally`: up to the answer's headers or the transport failure, or, for a call cut off before
/// either, up to the moment the call is dropped at the deadline or a cancellation. It records
/// one sample per call.
struct LatencyTimer<'a> {
    endpoint: &'static str,
    signer: &'a str,
    started: Instant,
    recorded: bool,
}

impl<'a> LatencyTimer<'a> {
    fn start(endpoint: &'static str, signer: &'a str) -> Self {
        Self {
            endpoint,
            signer,
            started: Instant::now(),
            recorded: false,
        }
    }

    fn record(&mut self) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        histogram!(SIGNER_LATENCY, "endpoint" => self.endpoint, "signer" => self.signer.to_owned())
            .record(self.started.elapsed().as_secs_f64());
    }
}

impl Drop for LatencyTimer<'_> {
    fn drop(&mut self) {
        self.record();
    }
}

/// One call, from the request to the classified answer.
async fn call_signer(
    call: &ThresholdCall<'_>,
    signer: &SignerEntry,
) -> Result<SignerSuccess, Failure> {
    let endpoint = call.endpoint.path();
    let mut latency = LatencyTimer::start(endpoint, &signer.url);
    // The URL is concatenated as in TS, so a configured trailing slash gives `//sign`.
    let sent = call
        .client
        .post(format!("{}{endpoint}", signer.url))
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, call.authorization)
        .header(KEY_VERSION_HEADER, call.key_version.to_string())
        .body(call.body.clone())
        .send()
        .await;
    latency.record();
    let response = sent.map_err(|error| Failure::NoStatus {
        reason: SIGNER_REQUEST_ERROR,
        detail: describe(&error),
    })?;

    let status = response.status();
    counter!(
        SIG_RESPONSES,
        "status" => status.as_u16().to_string(),
        "signer" => signer.url.clone(),
        "endpoint" => endpoint,
    )
    .increment(1);
    info!(signer = %signer.url, status = status.as_u16(), "Received signer answer");

    if !status.is_success() {
        let body = read_body(response).await?;
        // Only whether the body is JSON matters. `IgnoredAny` checks the syntax alone, so like
        // `JSON.parse` it takes numbers beyond f64 and lone surrogate escapes, which a
        // `serde_json::Value` refuses.
        if serde_json::from_slice::<IgnoredAny>(&body).is_err() {
            return Err(Failure::NoStatus {
                reason: SIGNER_REQUEST_ERROR,
                detail: format!("status {status} with a body that is not JSON"),
            });
        }
        counter!(
            SIG_RESPONSE_ERRORS,
            "status" => status.as_u16().to_string(),
            "signer" => signer.url.clone(),
            "endpoint" => endpoint,
        )
        .increment(1);
        info!(
            signer = %signer.url,
            status = status.as_u16(),
            body = %String::from_utf8_lossy(&body),
            "Signer answered with an error"
        );
        return Err(Failure::Status(status.as_u16()));
    }

    if call.endpoint == SignerEndpoint::Sign
        && key_version_header(response.headers()) != Some(Ok(call.key_version))
    {
        let received: Vec<_> = response
            .headers()
            .get_all(KEY_VERSION_HEADER)
            .iter()
            .collect();
        return Err(Failure::NoStatus {
            reason: INVALID_KEY_VERSION_RESPONSE,
            detail: format!("expected {}, received {received:?}", call.key_version),
        });
    }

    let body = read_body(response).await?;
    let answer: Value = serde_json::from_slice(&body).map_err(|error| Failure::NoStatus {
        reason: INVALID_SIGNER_RESPONSE,
        detail: error.to_string(),
    })?;
    success_fields(call.endpoint, &answer).ok_or_else(|| Failure::NoStatus {
        reason: if answer.get("success") == Some(&Value::Bool(false)) {
            SIGNER_RESPONSE_FAILED_WITH_OK_STATUS
        } else {
            INVALID_SIGNER_RESPONSE
        },
        detail: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Reads the body chunk by chunk and refuses it beyond `MAX_SIGNER_RESPONSE_BYTES`, whether or
/// not it declares a length.
async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, Failure> {
    let mut body = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|error| Failure::NoStatus {
            reason: SIGNER_REQUEST_ERROR,
            detail: describe(&error),
        })?;
        let Some(chunk) = chunk else {
            return Ok(body);
        };
        if body.len() + chunk.len() > MAX_SIGNER_RESPONSE_BYTES {
            return Err(Failure::NoStatus {
                reason: INVALID_SIGNER_RESPONSE,
                detail: format!("answer longer than {MAX_SIGNER_RESPONSE_BYTES} bytes"),
            });
        }
        body.extend_from_slice(&chunk);
    }
}

/// The answer if it matches the success side of the TS schema (`SignMessageResponseSchema`,
/// `PnpQuotaResponseSchema`): `success` is `true`, `version` a string, `warnings` absent or a
/// list of strings (io-ts refuses `null`), and for `/sign` a string `signature`. Other fields
/// are ignored. The quota numbers must be integers from 0 to `u64::MAX`; io-ts takes any number,
/// and signers write only such integers.
fn success_fields(endpoint: SignerEndpoint, answer: &Value) -> Option<SignerSuccess> {
    if answer.get("success")? != &Value::Bool(true) {
        return None;
    }
    let signature = match endpoint {
        SignerEndpoint::Sign => Some(answer.get("signature")?.as_str()?.to_owned()),
        SignerEndpoint::QuotaStatus => None,
    };
    let warnings = match answer.get("warnings") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        ),
        Some(_) => return None,
    };
    Some(SignerSuccess {
        quota: SignerQuota {
            version: answer.get("version")?.as_str()?.to_owned(),
            performed_query_count: answer.get("performedQueryCount")?.as_u64()?,
            total_quota: answer.get("totalQuota")?.as_u64()?,
            warnings,
        },
        signature,
    })
}

/// The error and its sources, for the log. reqwest's own text names only the kind and the URL.
fn describe(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use axum::Router;
    use axum::body::{Body, Bytes};
    use axum::extract::ConnectInfo;
    use axum::http::{HeaderMap, Uri};
    use axum::response::Response;
    use futures::{StreamExt, stream};
    use metrics_exporter_prometheus::PrometheusHandle;
    use odis_signer::types::KEY_VERSION_HEADER;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use crate::config::SignerEntry;
    use crate::errors::{CANCELLED_REQUEST_TO_SIGNER, SIGNER_REQUEST_ERROR, TIMEOUT_FROM_SIGNER};
    use crate::metrics::{
        SIG_INCONSISTENCY_ERRORS, SIG_REQUEST_ERRORS, SIG_RESPONSE_ERRORS, SIG_RESPONSES,
        SIGNER_LATENCY, WARNINGS, install_recorder,
    };
    use crate::quota::SignerQuota;
    use SignerEndpoint::{QuotaStatus, Sign};

    const AUTHORIZATION_VALUE: &str = "0xc0ffee";
    const KEY_VERSION: u32 = 1;
    /// Non-ASCII and two spaces, to show the bytes go out as given.
    const REQUEST_BODY: &str = r#"{"account":"0xabc","note":"é  x"}"#;
    /// Long enough that no test ends at the deadline unless it means to.
    const NO_DEADLINE: Duration = Duration::from_secs(10);

    const SIGNATURE: &str = "c2lnbmF0dXJl";
    const SIGN_ANSWER: &str = r#"{"success":true,"version":"1.0.0","signature":"c2lnbmF0dXJl","performedQueryCount":2,"totalQuota":10}"#;
    const QUOTA_ANSWER: &str =
        r#"{"success":true,"version":"1.0.0","performedQueryCount":2,"totalQuota":10}"#;
    const ERROR_ANSWER: &str = r#"{"success":false,"version":"1.0.0","error":"CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota"}"#;

    fn success(signature: Option<&str>, warnings: Option<Vec<String>>) -> SignerSuccess {
        SignerSuccess {
            quota: SignerQuota {
                version: "1.0.0".to_string(),
                performed_query_count: 2,
                total_quota: 10,
                warnings,
            },
            signature: signature.map(str::to_owned),
        }
    }

    /// A `/quotaStatus` success with the two numbers written as given.
    fn quota_answer(performed_query_count: &str, total_quota: &str) -> String {
        format!(
            r#"{{"success":true,"version":"1.0.0","performedQueryCount":{performed_query_count},"totalQuota":{total_quota}}}"#
        )
    }

    /// `answer` with a `warnings` field holding `json`.
    fn with_warnings(answer: &str, json: &str) -> String {
        format!(r#"{},"warnings":{json}}}"#, &answer[..answer.len() - 1])
    }

    /// `answer` padded with an unknown field to exactly `len` bytes, still valid.
    fn padded(answer: &str, len: usize) -> String {
        let open = &answer[..answer.len() - 1];
        let filler = len - open.len() - r#","padding":""}"#.len();
        let padded = format!(r#"{open},"padding":"{}"}}"#, "x".repeat(filler));
        assert_eq!(padded.len(), len);
        padded
    }

    // -- Metrics --

    /// The combiner's recorder, installed once per process. Under nextest every test runs in its
    /// own process, so the differences the tests compare are exact.
    fn recorder() -> &'static PrometheusHandle {
        static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
        HANDLE.get_or_init(|| install_recorder("combiner-test").expect("no other recorder"))
    }

    /// The sum of the samples of `name` whose labels include all of `labels`.
    fn sample(name: &str, labels: &[(&str, &str)]) -> u64 {
        recorder()
            .render()
            .lines()
            .filter_map(|line| {
                let (line_labels, value) = line
                    .strip_prefix(name)?
                    .strip_prefix('{')?
                    .rsplit_once("} ")?;
                let matches = labels.iter().all(|(key, value)| {
                    let pair = format!(r#"{key}="{value}""#);
                    line_labels.starts_with(&pair) || line_labels.contains(&format!(",{pair}"))
                });
                matches.then(|| value.parse::<f64>().unwrap() as u64)
            })
            .sum()
    }

    /// What the calls to one signer added to its metrics.
    #[derive(Debug, Default, PartialEq)]
    struct Counts {
        /// `SIG_RESPONSES`, at the given status if there is one.
        answers: u64,
        /// `SIG_RESPONSE_ERRORS`, at the given status if there is one.
        error_answers: u64,
        /// `SIG_REQUEST_ERRORS` with `ERR_09`.
        request_errors: u64,
        /// `SIG_REQUEST_ERRORS` with `ERR_10`.
        timeouts: u64,
        /// Samples in `SIGNER_LATENCY`.
        latencies: u64,
    }

    impl Counts {
        fn of(url: &str, endpoint: SignerEndpoint, status: Option<u16>) -> Counts {
            let status = status.map(|status| status.to_string());
            let mut labels = vec![("signer", url), ("endpoint", endpoint.path())];
            let request_error = |error_type| {
                let mut labels = labels.clone();
                labels.push(("error_type", error_type));
                sample(SIG_REQUEST_ERRORS, &labels)
            };
            let (request_errors, timeouts) = (
                request_error(SIGNER_REQUEST_ERROR),
                request_error(TIMEOUT_FROM_SIGNER),
            );
            let latencies = sample(&format!("{SIGNER_LATENCY}_count"), &labels);
            if let Some(status) = &status {
                labels.push(("status", status));
            }
            Counts {
                answers: sample(SIG_RESPONSES, &labels),
                error_answers: sample(SIG_RESPONSE_ERRORS, &labels),
                request_errors,
                timeouts,
                latencies,
            }
        }

        fn since(&self, before: &Counts) -> Counts {
            Counts {
                answers: self.answers - before.answers,
                error_answers: self.error_answers - before.error_answers,
                request_errors: self.request_errors - before.request_errors,
                timeouts: self.timeouts - before.timeouts,
                latencies: self.latencies - before.latencies,
            }
        }
    }

    /// `WARNINGS` with `WARN_08` for `endpoint`.
    fn cancelled(endpoint: SignerEndpoint) -> u64 {
        sample(
            WARNINGS,
            &[
                ("endpoint", endpoint.path()),
                ("warning_type", CANCELLED_REQUEST_TO_SIGNER),
            ],
        )
    }

    fn inconsistencies(endpoint: SignerEndpoint) -> u64 {
        sample(SIG_INCONSISTENCY_ERRORS, &[("endpoint", endpoint.path())])
    }

    // -- Signers --

    /// Serves `router` on a free local port and returns its URL.
    async fn serve(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
        url
    }

    /// A signer that answers every request with what `reply` builds.
    async fn answering(reply: impl Fn() -> Response + Clone + Send + Sync + 'static) -> String {
        serve(Router::new().fallback(move || std::future::ready(reply()))).await
    }

    /// The URL of a local port nothing listens on.
    async fn closed_port() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    fn reply(status: u16, key_version: Option<&str>, body: Body) -> Response {
        let mut builder = Response::builder().status(status);
        if let Some(key_version) = key_version {
            builder = builder.header(KEY_VERSION_HEADER, key_version);
        }
        builder.body(body).unwrap()
    }

    /// A body without a declared length, sent in 1 KiB chunks.
    fn chunked(text: String) -> Body {
        let chunks: Vec<Result<Bytes, Infallible>> = text
            .into_bytes()
            .chunks(1024)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        Body::from_stream(stream::iter(chunks))
    }

    /// Sends on its channel when dropped.
    struct DropGuard(Option<oneshot::Sender<()>>);

    impl Drop for DropGuard {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    /// A signer that never answers. The receiver resolves when the server drops the request,
    /// which it does once the combiner closes the connection.
    async fn silent() -> (String, oneshot::Receiver<()>) {
        let (sender, dropped) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        let url = serve(Router::new().fallback(move || {
            let guard = DropGuard(sender.lock().unwrap().take());
            async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            }
        }))
        .await;
        (url, dropped)
    }

    // -- Calls --

    fn entries(urls: &[&str]) -> Vec<SignerEntry> {
        urls.iter()
            .map(|url| SignerEntry {
                url: url.to_string(),
                share_index: None,
            })
            .collect()
    }

    fn threshold_call<'a>(
        client: &'a reqwest::Client,
        signers: &'a [SignerEntry],
        endpoint: SignerEndpoint,
        threshold: usize,
        deadline: Duration,
    ) -> ThresholdCall<'a> {
        ThresholdCall {
            client,
            signers,
            endpoint,
            key_version: KEY_VERSION,
            threshold,
            authorization: AUTHORIZATION_VALUE,
            body: Bytes::from_static(REQUEST_BODY.as_bytes()),
            deadline,
        }
    }

    async fn call_all(
        client: &reqwest::Client,
        signers: &[SignerEntry],
        endpoint: SignerEndpoint,
        threshold: usize,
        deadline: Duration,
    ) -> ThresholdOutcome {
        let call = threshold_call(client, signers, endpoint, threshold, deadline);
        call_signers(call, |_, _| Flow::Continue).await
    }

    #[tokio::test]
    async fn requests_carry_the_ts_headers_and_the_given_body() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let url = serve(
            Router::new().fallback(move |headers: HeaderMap, body: Bytes| {
                record.lock().unwrap().push((headers, body));
                std::future::ready(reply(200, Some("3"), SIGN_ANSWER.into()))
            }),
        )
        .await;
        let client = build_client().unwrap();
        let signers = entries(&[&url]);
        let call = ThresholdCall {
            key_version: 3,
            ..threshold_call(&client, &signers, Sign, 1, NO_DEADLINE)
        };

        let outcome = call_signers(call, |_, _| Flow::Continue).await;

        assert_eq!(outcome.successes, vec![(0, success(Some(SIGNATURE), None))]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (headers, body) = &seen[0];
        let mut headers: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_string()))
            .collect();
        headers.sort();
        let expected: Vec<(String, String)> = [
            ("accept", "application/json"),
            ("authorization", AUTHORIZATION_VALUE),
            ("content-length", &REQUEST_BODY.len().to_string()),
            ("content-type", "application/json"),
            ("host", url.strip_prefix("http://").unwrap()),
            ("odis-key-version", "3"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
        assert_eq!(headers, expected);
        assert_eq!(body, REQUEST_BODY.as_bytes());
    }

    #[tokio::test]
    async fn signer_url_is_concatenated_as_in_ts() {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let record = paths.clone();
        let url = serve(Router::new().fallback(move |uri: Uri| {
            record.lock().unwrap().push(uri.path().to_string());
            std::future::ready(reply(200, Some("1"), SIGN_ANSWER.into()))
        }))
        .await;
        let client = build_client().unwrap();

        for (entry_url, endpoint) in [(format!("{url}/"), Sign), (url.clone(), QuotaStatus)] {
            let signers = entries(&[&entry_url]);
            let outcome = call_all(&client, &signers, endpoint, 1, NO_DEADLINE).await;
            assert_eq!(outcome.successes.len(), 1, "{entry_url}");
        }

        assert_eq!(*paths.lock().unwrap(), ["//sign", "/quotaStatus"]);
    }

    /// The client is built with rustls: an `https://` signer gets a TLS handshake, where a
    /// client without TLS would refuse the URL before connecting.
    #[tokio::test]
    async fn https_signers_are_called_over_tls() {
        use tokio::io::AsyncReadExt;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let first_bytes = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 2];
            socket.read_exact(&mut header).await.unwrap();
            header
        });
        let client = build_client().unwrap();
        let signers = entries(&[&url]);

        let outcome = call_all(&client, &signers, Sign, 1, Duration::from_secs(2)).await;

        assert!(outcome.successes.is_empty());
        // A TLS record of type handshake (22), protocol major version 3.
        let header = timeout(Duration::from_secs(2), first_bytes)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(header, [0x16, 0x03]);
    }

    #[tokio::test]
    async fn answers_are_classified_as_in_the_ts_combiner() {
        /// What the combiner makes of one answer.
        enum Expect {
            /// A success, kept.
            Kept(SignerSuccess),
            /// An error with the answer's status.
            Status,
            /// An error without a status, `ERR_09`.
            NoStatus,
        }
        use Expect::{Kept, NoStatus, Status};
        const OVER_LIMIT: usize = 70 * 1024;

        let cases: Vec<(&str, SignerEndpoint, String, Option<u16>, Expect)> = vec![
            (
                "403 JSON",
                Sign,
                answering(|| reply(403, None, ERROR_ANSWER.into())).await,
                Some(403),
                Status,
            ),
            (
                "500 JSON that JavaScript reads and serde_json's Value refuses",
                Sign,
                answering(|| reply(500, None, r#"{"error":"\ud83d","n":1e400}"#.into())).await,
                Some(500),
                Status,
            ),
            (
                "500 HTML",
                Sign,
                answering(|| reply(500, None, "<html>Internal Server Error</html>".into())).await,
                Some(500),
                NoStatus,
            ),
            (
                "502 empty",
                Sign,
                answering(|| reply(502, None, Body::empty())).await,
                Some(502),
                NoStatus,
            ),
            (
                "200 /sign without the key version header",
                Sign,
                answering(|| reply(200, None, SIGN_ANSWER.into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 /sign with key version 2 when 1 was asked for",
                Sign,
                answering(|| reply(200, Some("2"), SIGN_ANSWER.into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 /sign with the key version in spaces, which TS trims",
                Sign,
                answering(|| reply(200, Some(" 1 "), SIGN_ANSWER.into())).await,
                Some(200),
                Kept(success(Some(SIGNATURE), None)),
            ),
            (
                "200 /sign without signature",
                Sign,
                answering(|| reply(200, Some("1"), QUOTA_ANSWER.into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 warnings null",
                Sign,
                answering(|| reply(200, Some("1"), with_warnings(SIGN_ANSWER, "null").into()))
                    .await,
                Some(200),
                NoStatus,
            ),
            (
                "200 success false",
                Sign,
                answering(|| reply(200, Some("1"), ERROR_ANSWER.into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 of 70 KiB with Content-Length",
                Sign,
                answering(|| reply(200, Some("1"), padded(SIGN_ANSWER, OVER_LIMIT).into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 of 70 KiB chunked",
                Sign,
                answering(|| reply(200, Some("1"), chunked(padded(SIGN_ANSWER, OVER_LIMIT)))).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 of exactly the limit, chunked",
                Sign,
                answering(|| {
                    let body = chunked(padded(SIGN_ANSWER, MAX_SIGNER_RESPONSE_BYTES));
                    reply(200, Some("1"), body)
                })
                .await,
                Some(200),
                Kept(success(Some(SIGNATURE), None)),
            ),
            (
                "200 one byte above the limit, chunked",
                Sign,
                answering(|| {
                    let body = chunked(padded(SIGN_ANSWER, MAX_SIGNER_RESPONSE_BYTES + 1));
                    reply(200, Some("1"), body)
                })
                .await,
                Some(200),
                NoStatus,
            ),
            (
                "200 /sign valid, warnings absent",
                Sign,
                answering(|| reply(200, Some("1"), SIGN_ANSWER.into())).await,
                Some(200),
                Kept(success(Some(SIGNATURE), None)),
            ),
            (
                "200 /sign valid, warnings []",
                Sign,
                answering(|| reply(200, Some("1"), with_warnings(SIGN_ANSWER, "[]").into())).await,
                Some(200),
                Kept(success(Some(SIGNATURE), Some(vec![]))),
            ),
            (
                "200 /quotaStatus valid, without the key version header",
                QuotaStatus,
                answering(|| reply(200, None, QUOTA_ANSWER.into())).await,
                Some(200),
                Kept(success(None, None)),
            ),
            (
                "200 /quotaStatus with a query count above u32::MAX",
                QuotaStatus,
                answering(|| reply(200, None, quota_answer("4294967296", "10").into())).await,
                Some(200),
                Kept(SignerSuccess {
                    quota: SignerQuota {
                        performed_query_count: 1 << 32,
                        ..success(None, None).quota
                    },
                    signature: None,
                }),
            ),
            (
                "200 /quotaStatus with totalQuota 2.0",
                QuotaStatus,
                answering(|| reply(200, None, quota_answer("2", "2.0").into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 /quotaStatus with totalQuota 1e2",
                QuotaStatus,
                answering(|| reply(200, None, quota_answer("2", "1e2").into())).await,
                Some(200),
                NoStatus,
            ),
            (
                "200 /quotaStatus with performedQueryCount -0",
                QuotaStatus,
                answering(|| reply(200, None, quota_answer("-0", "10").into())).await,
                Some(200),
                NoStatus,
            ),
            ("closed port", Sign, closed_port().await, None, NoStatus),
        ];

        let client = build_client().unwrap();
        for (name, endpoint, url, status, expect) in cases {
            let signers = entries(&[&url]);
            let before = Counts::of(&url, endpoint, status);
            let cancelled_before = cancelled(endpoint);

            let outcome = call_all(&client, &signers, endpoint, 1, NO_DEADLINE).await;

            let (successes, majority_status, error_answers, request_errors) = match expect {
                Kept(success) => (vec![(0, success)], None, 0, 0),
                Status => (vec![], status, 1, 0),
                NoStatus => (vec![], None, 0, 1),
            };
            let counts = Counts {
                answers: u64::from(status.is_some()),
                error_answers,
                request_errors,
                timeouts: 0,
                latencies: 1,
            };
            assert_eq!(
                (
                    outcome.successes,
                    outcome.majority_status,
                    Counts::of(&url, endpoint, status).since(&before),
                    cancelled(endpoint) - cancelled_before,
                ),
                (successes, majority_status, counts, 0),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn deadline_counts_err_10_and_not_warn_08() {
        let (url, dropped) = silent().await;
        let client = build_client().unwrap();
        let signers = entries(&[&url]);
        let before = Counts::of(&url, QuotaStatus, None);
        let cancelled_before = cancelled(QuotaStatus);

        let started = Instant::now();
        let outcome = call_all(
            &client,
            &signers,
            QuotaStatus,
            1,
            Duration::from_millis(300),
        )
        .await;
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(1),
            "{elapsed:?}"
        );
        assert_eq!((outcome.successes, outcome.majority_status), (vec![], None));
        // One latency sample for the call cut off before its headers, as TS's meter records.
        assert_eq!(
            Counts::of(&url, QuotaStatus, None).since(&before),
            Counts {
                timeouts: 1,
                latencies: 1,
                ..Counts::default()
            }
        );
        assert_eq!(cancelled(QuotaStatus), cancelled_before);
        assert_eq!(timeout(Duration::from_secs(1), dropped).await, Ok(Ok(())));
    }

    #[tokio::test]
    async fn stalled_body_after_headers_ends_at_the_deadline() {
        let url = answering(|| {
            let start = stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                br#"{"success":true"#,
            ))]);
            reply(200, None, Body::from_stream(start.chain(stream::pending())))
        })
        .await;
        let client = build_client().unwrap();
        let signers = entries(&[&url]);
        let before = Counts::of(&url, QuotaStatus, Some(200));
        let cancelled_before = cancelled(QuotaStatus);

        let started = Instant::now();
        let outcome = call_all(
            &client,
            &signers,
            QuotaStatus,
            1,
            Duration::from_millis(300),
        )
        .await;
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(1),
            "{elapsed:?}"
        );
        assert_eq!((outcome.successes, outcome.majority_status), (vec![], None));
        // The headers arrived, so the deadline adds no second latency sample.
        assert_eq!(
            Counts::of(&url, QuotaStatus, Some(200)).since(&before),
            Counts {
                answers: 1,
                timeouts: 1,
                latencies: 1,
                ..Counts::default()
            }
        );
        assert_eq!(cancelled(QuotaStatus), cancelled_before);
    }

    #[tokio::test]
    async fn early_stop_when_the_threshold_is_out_of_reach() {
        let first = answering(|| reply(500, None, ERROR_ANSWER.into())).await;
        let second = answering(|| reply(500, None, ERROR_ANSWER.into())).await;
        let (silent_url, dropped) = silent().await;
        let client = build_client().unwrap();
        let signers = entries(&[&first, &second, &silent_url]);
        let silent_before = Counts::of(&silent_url, Sign, None);
        let cancelled_before = cancelled(Sign);

        let started = Instant::now();
        let outcome = call_all(&client, &signers, Sign, 2, NO_DEADLINE).await;
        let elapsed = started.elapsed();

        assert!(elapsed < Duration::from_millis(200), "{elapsed:?}");
        assert_eq!(
            (outcome.successes, outcome.majority_status),
            (vec![], Some(500))
        );
        assert_eq!(cancelled(Sign) - cancelled_before, 1);
        assert_eq!(
            Counts::of(&silent_url, Sign, None).since(&silent_before),
            Counts {
                latencies: 1,
                ..Counts::default()
            }
        );
        assert_eq!(timeout(Duration::from_secs(1), dropped).await, Ok(Ok(())));
    }

    #[tokio::test]
    async fn stop_cancels_the_remaining_calls() {
        let (first_silent, first_dropped) = silent().await;
        let answered = answering(|| reply(200, None, QUOTA_ANSWER.into())).await;
        let (second_silent, second_dropped) = silent().await;
        let client = build_client().unwrap();
        let signers = entries(&[&first_silent, &answered, &second_silent]);
        let silent_before =
            [&first_silent, &second_silent].map(|url| Counts::of(url, QuotaStatus, None));
        let cancelled_before = cancelled(QuotaStatus);
        let mut seen = Vec::new();

        let started = Instant::now();
        let call = threshold_call(&client, &signers, QuotaStatus, 1, NO_DEADLINE);
        let outcome = call_signers(call, |index, success| {
            seen.push((index, success.clone()));
            Flow::Stop
        })
        .await;
        let elapsed = started.elapsed();

        let expected = vec![(1, success(None, None))];
        assert_eq!(seen, expected);
        assert_eq!(
            (outcome.successes, outcome.majority_status),
            (expected, None)
        );
        assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
        assert_eq!(cancelled(QuotaStatus) - cancelled_before, 2);
        for (url, before) in [&first_silent, &second_silent].iter().zip(&silent_before) {
            assert_eq!(
                Counts::of(url, QuotaStatus, None).since(before),
                Counts {
                    latencies: 1,
                    ..Counts::default()
                },
                "{url}"
            );
        }
        assert_eq!(
            timeout(Duration::from_secs(1), first_dropped).await,
            Ok(Ok(()))
        );
        assert_eq!(
            timeout(Duration::from_secs(1), second_dropped).await,
            Ok(Ok(()))
        );
    }

    #[tokio::test]
    async fn differing_error_statuses_are_counted_once() {
        let forbidden = answering(|| reply(403, None, ERROR_ANSWER.into())).await;
        let failing = answering(|| reply(500, None, ERROR_ANSWER.into())).await;
        let also_failing = answering(|| reply(500, None, ERROR_ANSWER.into())).await;
        let client = build_client().unwrap();

        for (urls, majority_status, counted) in [
            (vec![&forbidden, &failing], Some(403), 1),
            (vec![&forbidden, &failing, &also_failing], Some(500), 1),
            (vec![&failing, &also_failing], Some(500), 0),
        ] {
            let urls: Vec<&str> = urls.into_iter().map(String::as_str).collect();
            let signers = entries(&urls);
            let before = inconsistencies(Sign);

            let outcome = call_all(&client, &signers, Sign, 1, NO_DEADLINE).await;

            assert_eq!(
                (outcome.majority_status, inconsistencies(Sign) - before),
                (majority_status, counted),
                "{urls:?}"
            );
        }
    }

    #[tokio::test]
    async fn connections_to_a_signer_are_reused() {
        let peers = Arc::new(Mutex::new(Vec::new()));
        let record = peers.clone();
        let url = serve(Router::new().fallback(
            move |ConnectInfo(peer): ConnectInfo<SocketAddr>| {
                record.lock().unwrap().push(peer);
                std::future::ready(reply(200, None, QUOTA_ANSWER.into()))
            },
        ))
        .await;
        let client = build_client().unwrap();
        let signers = entries(&[&url]);

        for _ in 0..2 {
            let outcome = call_all(&client, &signers, QuotaStatus, 1, NO_DEADLINE).await;
            assert_eq!(outcome.successes, vec![(0, success(None, None))]);
        }

        let peers = peers.lock().unwrap();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0], peers[1]);
    }
}
