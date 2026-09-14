//! Outbound platform webhooks: an at-least-once delivery seam behind a
//! mockable [`WebhookSink`] trait.
//!
//! The default build ships an in-memory [`RecordingWebhookSink`] and a
//! deterministic, non-cryptographic signer ([`DefaultHashSigner`]) so the whole
//! surface is exercised offline by `cargo test`. Real HTTP POST with an
//! HMAC-SHA256 signature is added under the `webhooks` feature (via
//! `HttpWebhookSink` and `HmacSha256Signer`); nothing here links a network or
//! crypto crate in the default build.
//!
//! Every delivery carries an `X-OpenCompany-Signature`-equivalent header value
//! computed by the configured [`WebhookSigner`] over the JSON body: `kh1=<hex>`
//! by default, `sha256=<hex>` under `webhooks`.

use std::sync::{Arc, Mutex};
#[cfg(feature = "webhooks")]
use std::time::Duration;

use serde::Serialize;

use crate::Result;
use crate::ports::types::CompanyId;

/// The category of a platform webhook (api.md §Platform webhooks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookKind {
    /// An effect was parked and awaits operator approval.
    ApprovalRequested,
    /// A company completed a unit of work (a cycle produced output).
    WorkCompleted,
    /// A feedback item was captured.
    FeedbackCreated,
    /// A company's budget was exhausted.
    BudgetExhausted,
}

impl WebhookKind {
    /// The stable wire string for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalRequested => "approval_requested",
            Self::WorkCompleted => "work_completed",
            Self::FeedbackCreated => "feedback_created",
            Self::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// A platform webhook event delivered to the tenant's configured sink.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebhookEvent {
    /// The event category.
    #[serde(rename = "type")]
    pub kind: WebhookKind,
    /// The company the event is about.
    pub company_id: CompanyId,
    /// Epoch-millis timestamp the event was produced.
    pub at_millis: u64,
    /// Event-specific payload.
    pub data: serde_json::Value,
}

impl WebhookEvent {
    /// Builds an event stamped with the current time.
    pub fn now(kind: WebhookKind, company_id: CompanyId, data: serde_json::Value) -> Self {
        Self {
            kind,
            company_id,
            at_millis: crate::ports::now_millis(),
            data,
        }
    }
}

/// The at-least-once delivery seam. The default build uses the in-memory
/// [`RecordingWebhookSink`]; real HTTP POST is added under `webhooks`.
#[async_trait::async_trait]
pub trait WebhookSink: Send + Sync {
    /// Delivers `event` with the precomputed `signature` header value. An
    /// implementation may retry internally; a returned error means every attempt
    /// failed (and the caller logs, never blocking the cycle).
    async fn deliver(&self, event: &WebhookEvent, signature: &str) -> Result<()>;
}

/// An offline mock sink that records every delivery for assertions and never
/// fails. Used by the default build and tests.
#[derive(Clone, Default)]
pub struct RecordingWebhookSink {
    sent: Arc<Mutex<Vec<(WebhookEvent, String)>>>,
}

impl RecordingWebhookSink {
    /// Creates an empty recording sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of every `(event, signature)` delivered so far.
    pub fn delivered(&self) -> Vec<(WebhookEvent, String)> {
        self.sent.lock().expect("webhook sink poisoned").clone()
    }

    /// The number of deliveries recorded.
    pub fn count(&self) -> usize {
        self.sent.lock().expect("webhook sink poisoned").len()
    }
}

#[async_trait::async_trait]
impl WebhookSink for RecordingWebhookSink {
    async fn deliver(&self, event: &WebhookEvent, signature: &str) -> Result<()> {
        self.sent
            .lock()
            .expect("webhook sink poisoned")
            .push((event.clone(), signature.to_string()));
        Ok(())
    }
}

/// The signing seam so the delivery header is deterministic and offline-testable.
pub trait WebhookSigner: Send + Sync {
    /// Signs `body` with the tenant `secret`, returning the header value.
    fn sign(&self, secret: &str, body: &[u8]) -> String;
}

/// The default-build signer: a deterministic, non-cryptographic keyed hash over
/// `secret ‖ body`, rendered `kh1=<hex>`. Clearly labelled insecure; real
/// HMAC-SHA256 lives behind the `webhooks` feature.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultHashSigner;

impl WebhookSigner for DefaultHashSigner {
    fn sign(&self, secret: &str, body: &[u8]) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        secret.as_bytes().hash(&mut hasher);
        body.hash(&mut hasher);
        format!("kh1={:016x}", hasher.finish())
    }
}

/// A tenant's webhook delivery configuration: the sink, the signer, and the
/// per-tenant signing secret.
#[derive(Clone)]
pub struct WebhookConfig {
    /// The delivery sink (recording mock by default; HTTP under `webhooks`).
    pub sink: Arc<dyn WebhookSink>,
    /// The signer that computes the delivery header value.
    pub signer: Arc<dyn WebhookSigner>,
    /// The shared secret mixed into the signature.
    pub secret: String,
}

impl WebhookConfig {
    /// Builds a config over the in-memory recording sink and the deterministic
    /// signer — the offline default used by the CLI when no real transport is
    /// linked, and by tests.
    pub fn recording(secret: impl Into<String>) -> (Self, RecordingWebhookSink) {
        let sink = RecordingWebhookSink::new();
        (
            Self {
                sink: Arc::new(sink.clone()),
                signer: Arc::new(DefaultHashSigner),
                secret: secret.into(),
            },
            sink,
        )
    }

    /// Signs and delivers `event` with bounded best-effort retry. A delivery
    /// failure is logged and swallowed so a webhook never blocks a cycle.
    pub async fn emit(&self, event: &WebhookEvent) {
        let body = match serde_json::to_vec(event) {
            Ok(body) => body,
            Err(err) => {
                tracing::warn!(company = %event.company_id, "webhook serialize failed: {err}");
                return;
            }
        };
        let signature = self.signer.sign(&self.secret, &body);
        // At-least-once: a few bounded attempts, then give up with a warning.
        for attempt in 1..=3u32 {
            match self.sink.deliver(event, &signature).await {
                Ok(()) => return,
                Err(err) if attempt == 3 => {
                    tracing::warn!(
                        company = %event.company_id,
                        kind = event.kind.as_str(),
                        "webhook delivery failed after {attempt} attempts: {err}"
                    );
                }
                Err(_) => continue,
            }
        }
    }
}

impl std::fmt::Debug for WebhookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookConfig")
            .field("secret", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Real HMAC-SHA256 signer, emitting `sha256=<hex>`. Gated so the default build
/// links no crypto.
#[cfg(feature = "webhooks")]
#[derive(Clone, Copy, Debug, Default)]
pub struct HmacSha256Signer;

#[cfg(feature = "webhooks")]
impl WebhookSigner for HmacSha256Signer {
    fn sign(&self, secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
        mac.update(body);
        let bytes = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2 + 7);
        hex.push_str("sha256=");
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }
}

/// Real HTTP POST sink. Delivers the event as JSON with the signature header,
/// retrying a bounded number of times. Gated behind `webhooks`.
#[cfg(feature = "webhooks")]
pub struct HttpWebhookSink {
    url: String,
    client: reqwest::Client,
    /// Carried on the sink rather than baked into the client so the wiring is
    /// something a test can read: a timeout that lives only inside the client
    /// builder is asserted by exercising it, and a case that builds its own
    /// client to do so passes whether or not `new` sets one.
    timeout: Duration,
}

#[cfg(feature = "webhooks")]
impl HttpWebhookSink {
    /// The header carrying the delivery signature.
    pub const SIGNATURE_HEADER: &'static str = "X-OpenCompany-Signature";

    /// How long a single delivery attempt may take before it is treated as a
    /// failure.
    ///
    /// `reqwest::Client::new()` carries no timeout of its own — a receiving
    /// endpoint that accepts the connection and never responds would hang
    /// `deliver` (and, through it, every one of `emit`'s three bounded
    /// attempts) forever, defeating this module's own "never blocks a cycle"
    /// promise for the one failure mode a bounded retry loop cannot help
    /// with: an attempt that never returns at all.
    const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

    /// Builds a sink posting to `url`.
    pub fn new(url: impl Into<String>) -> Self {
        Self::with_timeout(url, Self::DELIVERY_TIMEOUT)
    }

    /// [`new`](Self::new), against a deadline the caller chooses.
    pub fn with_timeout(url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            url: url.into(),
            client: reqwest::Client::new(),
            timeout,
        }
    }
}

#[cfg(feature = "webhooks")]
#[async_trait::async_trait]
impl WebhookSink for HttpWebhookSink {
    async fn deliver(&self, event: &WebhookEvent, signature: &str) -> Result<()> {
        let resp = self
            .client
            .post(&self.url)
            .timeout(self.timeout)
            .header(Self::SIGNATURE_HEADER, signature)
            .json(event)
            .send()
            .await
            .map_err(|e| {
                crate::error::OpenCompanyError::Store(format!("webhook POST failed: {e}"))
            })?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(crate::error::OpenCompanyError::Store(format!(
                "webhook endpoint returned {}",
                resp.status()
            )))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn default_signer_is_deterministic_and_prefixed() {
        let signer = DefaultHashSigner;
        let a = signer.sign("secret", b"body");
        let b = signer.sign("secret", b"body");
        assert_eq!(a, b);
        assert!(a.starts_with("kh1="));
        // A different secret or body changes the signature.
        assert_ne!(a, signer.sign("other", b"body"));
        assert_ne!(a, signer.sign("secret", b"other"));
    }

    #[tokio::test]
    async fn recording_sink_captures_deliveries() {
        let (config, sink) = WebhookConfig::recording("s3cret");
        let event = WebhookEvent::now(
            WebhookKind::ApprovalRequested,
            CompanyId::new("acme"),
            serde_json::json!({ "approval_id": "a1" }),
        );
        config.emit(&event).await;

        let delivered = sink.delivered();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].0.kind, WebhookKind::ApprovalRequested);
        assert!(delivered[0].1.starts_with("kh1="));
    }

    /// A sink whose first `fail_first` calls return an error, so `emit`'s
    /// bounded retry has something real to exercise. Every attempt — failing
    /// and succeeding alike — is recorded, so a test can tell "retried and
    /// then delivered" from "delivered on the first try".
    #[derive(Clone, Default)]
    struct FlakySink {
        attempts: Arc<Mutex<u32>>,
        fail_first: u32,
    }

    impl FlakySink {
        fn new(fail_first: u32) -> Self {
            Self {
                attempts: Arc::default(),
                fail_first,
            }
        }

        fn attempts(&self) -> u32 {
            *self.attempts.lock().expect("attempts poisoned")
        }
    }

    #[async_trait::async_trait]
    impl WebhookSink for FlakySink {
        async fn deliver(&self, _event: &WebhookEvent, _signature: &str) -> Result<()> {
            let mut attempts = self.attempts.lock().expect("attempts poisoned");
            *attempts += 1;
            if *attempts <= self.fail_first {
                Err(crate::error::OpenCompanyError::Store(
                    "flaky sink: simulated failure".to_string(),
                ))
            } else {
                Ok(())
            }
        }
    }

    fn event() -> WebhookEvent {
        WebhookEvent::now(
            WebhookKind::WorkCompleted,
            CompanyId::new("acme"),
            serde_json::Value::Null,
        )
    }

    /// A sink that fails on the first attempt but succeeds on the retry must
    /// end up delivered — `emit`'s "a few bounded attempts" is dead code
    /// without a sink that ever returns `Err` at all, and `RecordingWebhookSink`
    /// never does.
    #[tokio::test]
    async fn emit_retries_a_transient_failure_and_still_delivers() {
        let sink = FlakySink::new(2);
        let config = WebhookConfig {
            sink: Arc::new(sink.clone()),
            signer: Arc::new(DefaultHashSigner),
            secret: "s3cret".to_string(),
        };
        config.emit(&event()).await;
        assert_eq!(
            sink.attempts(),
            3,
            "two failures then a success is three attempts"
        );
    }

    /// A sink that fails every attempt must not panic or block the caller —
    /// `emit` logs and swallows after the bound, exactly as a delivery that
    /// eventually succeeds does. Bounded, not unbounded: exactly the three
    /// documented attempts, not one more.
    #[tokio::test]
    async fn emit_gives_up_after_the_bound_without_panicking() {
        let sink = FlakySink::new(u32::MAX);
        let config = WebhookConfig {
            sink: Arc::new(sink.clone()),
            signer: Arc::new(DefaultHashSigner),
            secret: "s3cret".to_string(),
        };
        config.emit(&event()).await;
        assert_eq!(sink.attempts(), 3, "must stop at the documented bound");
    }

    /// `RecordingWebhookSink` is `Mutex`-guarded so concurrent cycles across
    /// different companies can emit webhooks at the same time without losing
    /// or corrupting a delivery. Prove it under genuine concurrent access
    /// rather than only the single-threaded call the existing test makes.
    #[tokio::test]
    async fn recording_sink_loses_nothing_under_concurrent_emits() {
        const N: usize = 50;
        let (config, sink) = WebhookConfig::recording("s3cret");
        let config = Arc::new(config);

        let mut tasks = Vec::with_capacity(N);
        for i in 0..N {
            let config = config.clone();
            tasks.push(tokio::spawn(async move {
                let event = WebhookEvent::now(
                    WebhookKind::FeedbackCreated,
                    CompanyId::new(format!("company-{i}")),
                    serde_json::json!({ "i": i }),
                );
                config.emit(&event).await;
            }));
        }
        for task in tasks {
            task.await.expect("a concurrent emit must not panic");
        }

        assert_eq!(sink.count(), N);
        let delivered = sink.delivered();
        let mut seen: Vec<usize> = delivered
            .iter()
            .map(|(event, _)| event.company_id.as_ref().to_string())
            .map(|id| id.strip_prefix("company-").unwrap().parse().unwrap())
            .collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..N).collect::<Vec<_>>(),
            "every emit must land exactly once"
        );
    }

    #[test]
    fn webhook_event_serializes_type_and_snake_case_kind() {
        let event = WebhookEvent::now(
            WebhookKind::WorkCompleted,
            CompanyId::new("acme"),
            serde_json::Value::Null,
        );
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "work_completed");
        assert_eq!(json["company_id"], "acme");
    }

    // -----------------------------------------------------------------
    // PLAT-050: `HmacSha256Signer` and `HttpWebhookSink` — the only two
    // pieces a real deployment uses — behind `webhooks`. Every test above
    // this point exercises `DefaultHashSigner` / `RecordingWebhookSink`
    // only.
    // -----------------------------------------------------------------

    /// A local, in-process HTTP server the tests below post to. Not the
    /// network `HttpWebhookSink` is forbidden from reaching — a loopback
    /// listener this test process owns and tears down, the same pattern
    /// `chargebee::client::test` uses for its own outbound-HTTP-client
    /// coverage.
    #[cfg(feature = "webhooks")]
    struct CapturedRequest {
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    }

    #[cfg(feature = "webhooks")]
    async fn capturing_mock_server() -> (
        String,
        Arc<Mutex<Vec<CapturedRequest>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::default();
        let captured_for_handler = captured.clone();
        let app = axum::Router::new().fallback(axum::routing::any(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let captured = captured_for_handler.clone();
                async move {
                    captured
                        .lock()
                        .expect("captured poisoned")
                        .push(CapturedRequest { headers, body });
                    axum::http::StatusCode::OK
                }
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), captured, server)
    }

    /// PLAT-050 (AUTH / STATE): a real `HttpWebhookSink::deliver` actually
    /// reaches an HTTP receiver and carries a signature that receiver can
    /// independently authenticate — a keyed HMAC over the exact body it
    /// received, computed with the correct secret and rejected under any
    /// other. Nothing before this test ever drove `HttpWebhookSink` over a
    /// real connection at all.
    #[cfg(feature = "webhooks")]
    #[tokio::test]
    async fn http_webhook_sink_delivers_a_signature_a_receiver_can_authenticate() {
        let (url, captured, server) = capturing_mock_server().await;
        let config = WebhookConfig {
            sink: Arc::new(HttpWebhookSink::new(url)),
            signer: Arc::new(HmacSha256Signer),
            secret: "s3cret".to_string(),
        };
        let event = WebhookEvent::now(
            WebhookKind::ApprovalRequested,
            CompanyId::new("acme"),
            serde_json::json!({ "approval_id": "a1" }),
        );
        config.emit(&event).await;

        let requests = captured.lock().expect("captured poisoned");
        assert_eq!(
            requests.len(),
            1,
            "exactly one delivery must reach the receiver"
        );
        let request = &requests[0];
        let signature = request
            .headers
            .get(HttpWebhookSink::SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .expect("signature header present");

        // The receiver's own independent check: recompute the HMAC over the
        // exact bytes it received, with the correct secret.
        let expected = HmacSha256Signer.sign("s3cret", &request.body);
        assert_eq!(
            signature, expected,
            "the receiver must be able to authenticate the delivery itself"
        );
        // And under the wrong secret, authentication must fail — the
        // signature is not just present, it is actually keyed.
        let wrong = HmacSha256Signer.sign("not-the-secret", &request.body);
        assert_ne!(signature, wrong);
        server.abort();
    }

    /// PLAT-050 (FAIL): a receiver that answers non-2xx must be reported as
    /// a delivery failure, not swallowed as success — the distinction
    /// `emit`'s bounded retry depends on to know whether to try again.
    #[cfg(feature = "webhooks")]
    #[tokio::test]
    async fn http_webhook_sink_reports_a_non_success_status_as_an_error() {
        let app = axum::Router::new().fallback(axum::routing::any(|| async {
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let sink = HttpWebhookSink::new(format!("http://{addr}"));
        let event = WebhookEvent::now(
            WebhookKind::WorkCompleted,
            CompanyId::new("acme"),
            serde_json::Value::Null,
        );
        let result = sink.deliver(&event, "sha256=deadbeef").await;
        assert!(
            result.is_err(),
            "a 500 response must be reported as a delivery failure"
        );
        server.abort();
    }

    /// PLAT-050 (CONC): production concurrency is many companies' cycles
    /// calling `emit` on one tenant's `Arc<WebhookConfig>` — one shared
    /// `HttpWebhookSink`, one shared `reqwest::Client`, many callers. This
    /// drives that same single sink with real concurrent deliveries and
    /// proves none are lost or corrupted in transit, matching
    /// `recording_sink_loses_nothing_under_concurrent_emits` for the sink a
    /// real deployment actually uses.
    #[cfg(feature = "webhooks")]
    #[tokio::test]
    async fn http_webhook_sink_loses_nothing_under_concurrent_delivery() {
        const N: usize = 20;
        let (url, captured, server) = capturing_mock_server().await;
        let sink = Arc::new(HttpWebhookSink::new(url));

        let mut tasks = Vec::with_capacity(N);
        for i in 0..N {
            let sink = sink.clone();
            tasks.push(tokio::spawn(async move {
                let event = WebhookEvent::now(
                    WebhookKind::ApprovalRequested,
                    CompanyId::new(format!("company-{i}")),
                    serde_json::json!({ "i": i }),
                );
                sink.deliver(&event, &format!("sha256={i:064x}")).await
            }));
        }
        for task in tasks {
            assert!(
                task.await
                    .expect("a concurrent delivery must not panic")
                    .is_ok(),
                "every concurrent delivery to one shared sink must succeed"
            );
        }

        let requests = captured.lock().expect("captured poisoned");
        assert_eq!(
            requests.len(),
            N,
            "every delivery must reach the receiver exactly once"
        );
        let mut seen: Vec<usize> = requests
            .iter()
            .map(|r| {
                let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                body["company_id"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("company-")
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..N).collect::<Vec<_>>(),
            "none dropped, none duplicated"
        );
        server.abort();
    }

    /// PLAT-050 (LIMIT / BOUND): the sink the runtime builds carries the
    /// deadline, not just one a test can construct.
    ///
    /// Exercising a timeout through a sink the case builds itself proves the
    /// mechanism and nothing about the wiring: `new` could stop setting one
    /// and that case would still pass. This reads the deadline off the
    /// constructor the runtime actually calls.
    #[cfg(feature = "webhooks")]
    #[test]
    fn the_sink_the_runtime_builds_carries_a_delivery_deadline() {
        let sink = HttpWebhookSink::new("http://example.invalid/hook");
        assert_eq!(
            sink.timeout,
            HttpWebhookSink::DELIVERY_TIMEOUT,
            "a delivery with no deadline hangs every bounded retry behind it"
        );
        assert!(
            sink.timeout <= Duration::from_secs(30),
            "the deadline must be short enough to keep a cycle moving: {:?}",
            sink.timeout
        );
    }

    /// PLAT-050 (LIMIT / BOUND): a receiver that accepts the connection and
    /// never answers must not hang `deliver` forever — `reqwest::Client::new()`
    /// alone carries no timeout, so this is the one failure mode `emit`'s
    /// bounded-attempt retry cannot protect against on its own: an attempt that
    /// never *returns* at all rather than one that returns quickly with an
    /// error.
    #[cfg(feature = "webhooks")]
    #[tokio::test]
    async fn http_webhook_sink_times_out_rather_than_hanging_forever() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        // Accepts the TCP connection and then never reads or writes anything
        // — a receiver that hung mid-request, not one that refused the
        // connection outright (which `reqwest` would fail on immediately
        // regardless of any timeout).
        let server = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    // Hold the connection open and do nothing with it.
                    std::mem::forget(socket);
                } else {
                    break;
                }
            }
        });

        let sink = HttpWebhookSink::with_timeout(
            format!("http://{addr}"),
            std::time::Duration::from_millis(200),
        );
        let event = WebhookEvent::now(
            WebhookKind::WorkCompleted,
            CompanyId::new("acme"),
            serde_json::Value::Null,
        );
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            sink.deliver(&event, "sha256=deadbeef"),
        )
        .await;
        server.abort();

        let delivered = outcome.expect(
            "deliver must return on its own within the client timeout, not hang until this \
             test's outer 5s bound",
        );
        assert!(
            delivered.is_err(),
            "a receiver that never responds must be reported as a failed delivery"
        );
    }
}
