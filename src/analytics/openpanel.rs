//! The OpenPanel transport, and the one function that chooses a tracker.
//!
//! [`build`] is compiled into **every** build; `HttpOpenPanelTracker` is
//! compiled only under `--features analytics`. That split is the acceptance
//! criterion of issue #1739 expressed as a type rather than as a rule: in a
//! default build there is no type here that owns an HTTP client, so "a build
//! with no opt-in emits zero outbound analytics requests" is not a behaviour
//! that could regress — the code that would make the request is not in the
//! binary.
//!
//! Under the feature, [`build`] still returns [`NullTracker`] for every
//! [`Decision::Silent`], which is what a desktop or self-hosted install resolves
//! to. `a_self_hosted_build_makes_no_request` proves that against a real local
//! collector, and `a_hosted_tenant_reports` is its positive control — without
//! the second, a zero request count would be indistinguishable from a test that
//! never sends anything at all.
//!
//! # What is here, and what is deliberately not
//!
//! Everything about *what* an event says lives in [`crate::analytics::payload`],
//! [`Envelope`] and [`Event`], which are un-gated and tested in every lane. This
//! file owns only the HTTP call: how the body is delivered, how the credential
//! is presented, and what happens when the collector does not answer. Keeping
//! that line where it is is why a content leak would be caught by the default
//! `cargo test` rather than only by the one lane that compiles `reqwest`.
//!
//! # One request per event
//!
//! **OpenPanel has no batch endpoint.** `POST /track` takes a single
//! discriminated-union object; there is no array body and no `/batch` route, so
//! the batching this module used to do has nowhere to go. See [`Inner::drain`]
//! for what replaced it and why the queue survived the batch.

use std::sync::Arc;

use crate::analytics::config::Decision;
use crate::analytics::{Envelope, NullTracker, Tracker};

/// Chooses the tracker this process will use.
///
/// The whole of the "hosted tenants only, by default" decision lands here: a
/// [`Decision::Silent`] gets a [`NullTracker`], and in a build without the
/// `analytics` feature *every* decision does, because there is nothing else to
/// return.
///
/// # A transport that cannot be built is a [`NullTracker`], never a degraded one
///
/// [`HttpOpenPanelTracker::new`] is fallible because the HTTP client it wraps is
/// where the credential headers and the send timeout are configured, and both
/// are load-bearing. The obvious fallback — `reqwest::Client::default()` — is
/// the wrong answer twice: that client carries **no default headers**, so every
/// request goes out unauthenticated and is refused, and it carries **no
/// timeout**, so a slow collector parks a drain forever and `Tracker::flush`
/// waits behind it, which is exactly the shutdown block the five-second bound
/// exists to prevent. It is also not even a safe fallback in the case that
/// produces it: `Client::default()` is `Client::new()`, which is
/// `ClientBuilder::new().build().expect(…)` — the same `build` that just
/// failed, now panicking at boot instead of returning an error.
///
/// So a client that will not build disables reporting and says so once, loudly.
/// Sending nothing is a documented outcome of this module with a whole
/// vocabulary of reasons behind it; sending unauthenticated requests with no
/// timeout is not.
pub fn build(decision: &Decision, envelope: Envelope) -> Arc<dyn Tracker> {
    match decision {
        Decision::Silent(_) => Arc::new(NullTracker),
        #[cfg(feature = "analytics")]
        Decision::Report {
            endpoint,
            credentials,
        } => match http::HttpOpenPanelTracker::new(endpoint, credentials, envelope) {
            Ok(tracker) => Arc::new(tracker),
            // Routed through the same redaction the send path uses. A builder
            // error carries no request URL today, but "the dependency does not
            // print one here" is not a property this crate owns, and the
            // endpoint on the same line comes from the one helper that redacts.
            Err(error) => {
                tracing::warn!(
                    endpoint = %crate::analytics::boot::loggable_endpoint(endpoint),
                    error = %http::loggable_send_error(error),
                    "[analytics] the HTTP client for the collector could not be built, so \
                     reporting is off for this process. Nothing will be sent."
                );
                Arc::new(NullTracker)
            }
        },
        // Without the feature there is no transport to hand back. Reporting was
        // configured and the build cannot honour it, which is worth one line at
        // boot: silently ignoring an explicit `OPENCOMPANY_ANALYTICS=on` is the
        // kind of quiet no-op an operator debugs for an hour.
        #[cfg(not(feature = "analytics"))]
        Decision::Report { .. } => {
            let _ = envelope;
            tracing::info!(
                "[analytics] reporting is configured but this build was compiled without \
                 the `analytics` feature, so nothing is sent"
            );
            Arc::new(NullTracker)
        }
    }
}

#[cfg(feature = "analytics")]
pub use http::HttpOpenPanelTracker;

/// The header OpenPanel takes the client id in.
///
/// Named here rather than inline because the gated tests assert the exact
/// spelling: a header the collector does not recognise is a 401 behind a
/// `debug!`, which is the silent failure this whole module is built around.
#[cfg(feature = "analytics")]
pub const CLIENT_ID_HEADER: &str = "openpanel-client-id";

/// The header OpenPanel takes the client secret in.
#[cfg(feature = "analytics")]
pub const CLIENT_SECRET_HEADER: &str = "openpanel-client-secret";

/// Names this client on the operator's own collector.
///
/// OpenPanel stores `openpanel-sdk-name` / `openpanel-sdk-version` on the event,
/// which is how an operator running one collector for several things tells this
/// traffic apart from a browser SDK's. It costs one header and answers "what is
/// writing to my project?" without anyone having to ask us.
#[cfg(feature = "analytics")]
pub const SDK_NAME_HEADER: &str = "openpanel-sdk-name";

/// The version half of the pair above.
#[cfg(feature = "analytics")]
pub const SDK_VERSION_HEADER: &str = "openpanel-sdk-version";

/// The value sent as [`SDK_NAME_HEADER`].
#[cfg(feature = "analytics")]
pub const SDK_NAME: &str = "opencompany";

/// **Event names OpenPanel refuses outright.**
///
/// `packages/constants/index.ts`, read at commit
/// `3060ca10213693cf0385be2713c8743d16733a2b`. A `track` whose `payload.name` is
/// one of these fails the collector's own zod refinement and comes back 400.
///
/// Copied here rather than merely known, because the failure it guards is the
/// one this module is least able to notice: a rejected event is a `debug!` line
/// and nothing else, so a name collision introduced years from now would look
/// exactly like a healthy instance that happens to report one fewer event. The
/// test below is a compile-time-vocabulary check against a runtime constant, and
/// it costs nothing to keep.
///
/// It is not the whole of OpenPanel's name validation — `event-blocklist.ts`
/// also rejects names over 80 characters, names containing a newline, names
/// beginning `/`, and a long anti-abuse substring list (`${`, `%{`, `../`,
/// `union select`, …). Those are asserted alongside it rather than transcribed:
/// transcribing a fifty-entry blocklist is how a copy goes stale.
pub const OPENPANEL_RESERVED_EVENT_NAMES: [&str; 2] = ["session_start", "session_end"];

#[cfg(feature = "analytics")]
mod http {
    use std::sync::{Arc, Mutex, Weak};
    use std::time::Duration;

    use async_trait::async_trait;

    use crate::analytics::config::ClientCredentials;
    use crate::analytics::{Envelope, Event, Tracker, payload};

    /// How often the background task drains the queue.
    ///
    /// A threshold alone is not enough: a quiet instance would hold its events
    /// until the next one arrived, which on a company that ran two turns and
    /// stopped is forever.
    const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

    /// The most events held before the oldest are dropped.
    ///
    /// Analytics must never be able to grow without bound inside a tenant
    /// container. If the collector is unreachable for long enough to fill this,
    /// the right outcome is losing telemetry, not the process.
    const MAX_QUEUED: usize = 500;

    /// How long a send may take before it is abandoned. Short on purpose:
    /// nothing waits on this, but a request that never completes is a task that
    /// never ends.
    const SEND_TIMEOUT: Duration = Duration::from_secs(5);

    /// Queues events and POSTs them to OpenPanel, one request each.
    pub struct HttpOpenPanelTracker {
        inner: Arc<Inner>,
    }

    struct Inner {
        /// Carries the two credential headers as **default headers**, set once
        /// at construction and marked sensitive.
        ///
        /// This is the whole of the credential handling, and it is the part of
        /// the change worth reading twice. Mixpanel wanted its token stamped
        /// into every event's property bag, so the transport had to reach into
        /// a rendered payload and mutate it — which meant a captured body, a
        /// recorded event or a test fixture could carry the credential, and the
        /// only thing stopping it was that nothing did. OpenPanel authenticates
        /// with request headers, so the credential is set once, here, and never
        /// touches the body builder at all. There is no longer a code path that
        /// could put it in a payload.
        ///
        /// `HeaderValue::set_sensitive` on both, which keeps them out of
        /// `HeaderValue`'s own `Debug` and out of HPACK's shared table on
        /// HTTP/2.
        client: reqwest::Client,
        endpoint: String,
        /// Behind a lock because its cognition labels are re-read after boot
        /// — see [`Envelope::set_cognition`]. Only ever held to render one
        /// payload or to relabel, never across an await.
        envelope: std::sync::RwLock<Envelope>,
        queue: Mutex<Vec<serde_json::Value>>,
        /// Held for the whole of one `drain`, take **and** requests.
        ///
        /// Without it, the shutdown flush and the 30-second drain could
        /// overlap: the drain takes the entire queue and awaits its POSTs, the
        /// flush finds an empty queue, returns at once, and process exit
        /// cancels the requests still in flight. That loses events exactly when
        /// the collector is slow — the one case the graceful flush exists for.
        /// An **async** mutex because it is held across an await; the `queue`
        /// lock below stays a `std::sync` one and is never held across one.
        sending: tokio::sync::Mutex<()>,
        stop: tokio::sync::Notify,
        /// Whether the collector has already told us the credential is no good.
        ///
        /// A `401` is not a failure like the others. Every other thing that can
        /// go wrong here is transient — a collector restarting, a network
        /// blip — and deserves the `debug!` that #1739 settled on, because it
        /// resolves itself and a `warn!` per event would be a log flood for a
        /// problem nobody needs to act on. A refused credential resolves itself
        /// never: every event for the rest of the process's life is dropped, the
        /// boot line said "reporting to …", and the only trace is a `debug!` no
        /// operator has enabled. That is the exact failure this module exists to
        /// make impossible, arriving one layer below where the boot line can see
        /// it.
        ///
        /// So it is a `warn!`, and it is said **once**: the condition is
        /// permanent, so repeating it adds nothing and would drown the log of a
        /// busy tenant.
        credential_refused: std::sync::atomic::AtomicBool,
        /// Whether the collector has already answered with a redirect.
        ///
        /// The client follows none of them — see
        /// [`HttpOpenPanelTracker::new`] — which closes the credential leak and
        /// opens a diagnostic hole in its place: a `3xx` arrives here as an
        /// ordinary non-success response, so a misconfigured endpoint would
        /// look exactly like a collector rejecting every event, behind a
        /// `debug!` nobody has enabled, forever. That is the failure shape this
        /// module exists to refuse.
        ///
        /// So a redirect gets the [`Self::credential_refused`] treatment: it is
        /// a verdict on the *endpoint* rather than on one event, every event
        /// behind it gets the same one, and it is a `warn!` said exactly once.
        endpoint_redirects: std::sync::atomic::AtomicBool,
        /// How many events have been lost to a **cancelled** drain.
        ///
        /// Every other way a drain ends states its own count in its own log
        /// line. Cancellation cannot: the future is dropped, so there is no
        /// branch to log from. [`CancelledDrain`] reports it on `Drop` and
        /// records the total here, which makes the loss assertable — a log line
        /// alone is not something a test can hold to account.
        lost_to_cancellation: std::sync::atomic::AtomicUsize,
    }

    impl HttpOpenPanelTracker {
        /// Builds a tracker and starts its drain loop.
        ///
        /// The credential is validated for header-safety in
        /// [`crate::analytics::config::resolve`], which is why the two
        /// `from_str` calls here can fall back rather than fail: by the time a
        /// [`Decision::Report`](crate::analytics::config::Decision::Report)
        /// exists, both halves are printable ASCII with no space, which is a
        /// strict subset of what `HeaderValue` takes. The fallback is an empty
        /// header value, which the collector refuses with a 401 — a loud,
        /// bounded outcome rather than a panic at boot, for a branch that is
        /// unreachable given the check upstream.
        ///
        /// # Fallible, because there is no acceptable degraded client
        ///
        /// The client built here is the only place the credential headers and
        /// [`SEND_TIMEOUT`] are set, so a client built without them is not a
        /// weaker version of this one — it is one that authenticates against
        /// nothing and can hang a shutdown. [`super::build`] turns the error
        /// into a `NullTracker` and one `warn!`; see the note there for why
        /// `reqwest::Client::default()` is not the fallback it looks like.
        ///
        /// # Redirects are never followed
        ///
        /// `reqwest`'s default policy follows up to ten hops, and its
        /// cross-origin sanitization removes only `Authorization`, `Cookie`,
        /// `cookie2`, `Proxy-Authorization` and `WWW-Authenticate`
        /// (`redirect.rs::remove_sensitive_headers`, reqwest 0.12.28, read
        /// rather than assumed). The two `openpanel-client-*` headers are none
        /// of those, so a `302` from the configured endpoint to any other
        /// authority — a reverse proxy sending unauthenticated callers to an
        /// SSO host is the ordinary way one arrives — would have handed this
        /// instance's write secret to a host the operator never named.
        /// `HeaderValue::set_sensitive` does not help: it governs `Debug` and
        /// HPACK indexing, not redirect handling.
        ///
        /// That sanitization also compares only **host and port**, never the
        /// scheme, so an `https` endpoint that redirected to `http://` on the
        /// same host would have carried the secret across in cleartext — the
        /// `Silence::InsecureEndpoint` rule in
        /// [`crate::analytics::config`] bypassed by a response the operator
        /// does not control.
        ///
        /// So: [`reqwest::redirect::Policy::none`], with no same-origin
        /// exception. A same-origin policy would also be safe, but it is a
        /// predicate to keep correct rather than an invariant to state, and all
        /// it buys is a collector that 301s `/track` to `/api/track` — an
        /// endpoint the operator can type correctly once, after reading the
        /// warning [`Inner::report_redirected_endpoint`] emits. Following none
        /// of them makes "the credential only ever goes to the configured
        /// endpoint" a property of this client rather than a claim about a
        /// comparison.
        ///
        /// # A cleartext endpoint never goes through a proxy
        ///
        /// The same hole as the redirect one, by a different route, and it
        /// invalidates the loopback exception rather than merely widening it.
        /// `resolve` permits plain `http` only for a loopback host, and the
        /// entire justification is that such a request **does not leave the
        /// host** — so there is no wire between machines for the credential to
        /// be read off. A proxy makes that false. `reqwest`'s builder defaults
        /// to `auto_sys_proxy: true` (`async_impl/client.rs:309`), which pushes
        /// `ProxyMatcher::system()`, and that reads `HTTP_PROXY`/`ALL_PROXY`
        /// with exclusions taken **only** from `NO_PROXY` — hyper-util 0.1.20's
        /// matcher has no implicit carve-out for `localhost` or `127.0.0.0/8`,
        /// checked rather than assumed. So on a host with `HTTP_PROXY` set and
        /// no matching `NO_PROXY`, `http://localhost:3000/track` was sent to the
        /// proxy instead, in cleartext, with both credential headers on it.
        ///
        /// So the cleartext case builds with
        /// [`reqwest::ClientBuilder::no_proxy`], which makes "it does not leave
        /// the host" true by construction instead of by assumption about the
        /// operator's environment. That is the same move as
        /// `redirect::Policy::none()`: a security property should be a fact
        /// about this client, not a prediction about its surroundings.
        ///
        /// **`https` keeps its proxy support, deliberately.** A proxied `https`
        /// request is a `CONNECT` tunnel: the proxy learns the host and port and
        /// never sees a header, so the credential is not exposed to it, and
        /// egress-restricted networks genuinely need it to reach a collector at
        /// all. Disabling proxies outright would break those deployments to fix
        /// a leak they do not have.
        ///
        /// The scheme is the whole test, because by the time a
        /// [`Decision::Report`](crate::analytics::config::Decision::Report)
        /// exists, `http` **implies** loopback — `config::is_secure_endpoint`
        /// has already refused every other `http` endpoint.
        ///
        /// # Crate-private, because that implication is the invariant
        ///
        /// The sentence above is only true of endpoints that came through
        /// [`resolve`](crate::analytics::config::resolve). While this
        /// constructor was `pub` it was also a way around it: the type is
        /// re-exported from a `pub mod`, so an `analytics`-enabled caller could
        /// hand it `http://collector.internal/track` directly and get a tracker
        /// that posts the client secret across a network in cleartext, with
        /// [`is_cleartext`] dutifully turning off the proxy on the way. A
        /// safety property enforced only by the route callers happen to take is
        /// the thing this module keeps arguing against, so the route is now the
        /// only one there is: [`super::build`] takes a `&Decision`, and a
        /// `Decision::Report` is what `resolve` produces.
        ///
        /// The `debug_assert!` is defence in depth against the same mistake
        /// arriving from *inside* the crate later. It calls
        /// `config::is_secure_endpoint` rather than restating the rule, because
        /// a second reader of a security predicate is a bypass waiting to be
        /// found — the same reason `is_usable_endpoint` refuses to hand-roll the
        /// URL grammar `reqwest` already parses.
        pub(crate) fn new(
            endpoint: &str,
            credentials: &ClientCredentials,
            envelope: Envelope,
        ) -> Result<Self, reqwest::Error> {
            debug_assert!(
                crate::analytics::config::is_secure_endpoint(endpoint),
                "a tracker was built for an endpoint the credential cannot safely cross; \
                 every endpoint must come through config::resolve"
            );
            let mut builder = reqwest::Client::builder()
                .timeout(SEND_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .default_headers(request_headers(credentials));
            if is_cleartext(endpoint) {
                builder = builder.no_proxy();
            }
            let inner = Arc::new(Inner {
                client: builder.build()?,
                endpoint: endpoint.to_string(),
                envelope: std::sync::RwLock::new(envelope),
                queue: Mutex::new(Vec::new()),
                sending: tokio::sync::Mutex::new(()),
                stop: tokio::sync::Notify::new(),
                credential_refused: std::sync::atomic::AtomicBool::new(false),
                endpoint_redirects: std::sync::atomic::AtomicBool::new(false),
                lost_to_cancellation: std::sync::atomic::AtomicUsize::new(0),
            });

            // A `Weak` so the loop cannot keep the tracker alive, and
            // `try_current` so constructing one outside a runtime is a
            // flush-only tracker rather than a panic. Neither is theoretical:
            // the drop path is how a rebuilt runtime retires its tracker, and a
            // synchronous test constructs one with no reactor.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let weak = Arc::downgrade(&inner);
                handle.spawn(async move { drain_loop(weak).await });
            }

            Ok(Self { inner })
        }

        /// How many events a **cancelled** drain has lost so far.
        ///
        /// Exists so the shutdown-budget loss is assertable rather than merely
        /// logged: a `warn!` is what an operator sees, and a counter is what a
        /// test can hold to account. Every other way a drain ends already names
        /// its own count in its own line.
        #[cfg(test)]
        pub(super) fn lost_to_cancellation(&self) -> usize {
            self.inner
                .lost_to_cancellation
                .load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    /// Every header this client sends on every request: the two credential
    /// halves, marked sensitive, and the two that name the client.
    pub(super) fn request_headers(credentials: &ClientCredentials) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let sensitive = |raw: &str| {
            let mut value =
                HeaderValue::from_str(raw).unwrap_or_else(|_| HeaderValue::from_static(""));
            value.set_sensitive(true);
            value
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(super::CLIENT_ID_HEADER),
            sensitive(credentials.expose_id()),
        );
        headers.insert(
            HeaderName::from_static(super::CLIENT_SECRET_HEADER),
            sensitive(credentials.expose_secret()),
        );
        // Compile-time constants, both. They name this client on a collector the
        // operator may be pointing several things at.
        headers.insert(
            HeaderName::from_static(super::SDK_NAME_HEADER),
            HeaderValue::from_static(super::SDK_NAME),
        );
        if let Ok(version) = HeaderValue::from_str(env!("CARGO_PKG_VERSION")) {
            headers.insert(HeaderName::from_static(super::SDK_VERSION_HEADER), version);
        }
        headers
    }

    impl Drop for HttpOpenPanelTracker {
        fn drop(&mut self) {
            self.inner.stop.notify_waiters();
        }
    }

    impl std::fmt::Debug for HttpOpenPanelTracker {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // No endpoint and certainly no credential: this type holds one, and
            // a `{:?}` in a log line is exactly how one escapes.
            f.write_str("HttpOpenPanelTracker")
        }
    }

    async fn drain_loop(weak: Weak<Inner>) {
        loop {
            let Some(inner) = weak.upgrade() else { return };
            let stopped = {
                let stop = &inner.stop;
                tokio::select! {
                    _ = stop.notified() => true,
                    _ = tokio::time::sleep(FLUSH_INTERVAL) => false,
                }
            };
            inner.drain().await;
            if stopped {
                return;
            }
        }
    }

    /// Reports the tail of a drain that was **cancelled** rather than finished.
    ///
    /// Cancellation is the one way [`Inner::drain`] can end without saying
    /// anything, because it is not a branch the drain takes — the future is
    /// dropped out from under it. In practice that means the shutdown flush
    /// running out of its budget (`server::shutdown::flush_budget`, at most 2s),
    /// which with one request per event is a routine occurrence for a busy
    /// tenant rather than an exotic one. Before this, those events vanished and
    /// the only trace was a `debug!` at the call site that names no count.
    ///
    /// `Drop` **is** the cancellation path, so the report lives there. Every
    /// deliberate exit disarms the guard first, because each of those logs its
    /// own count and a second line would double-count the same events.
    ///
    /// It never prints the raw endpoint — `loggable_endpoint` is applied when
    /// the guard is built, for the reason [`loggable_send_error`] exists.
    struct CancelledDrain<'a> {
        /// Events taken off the queue that have not been sent yet.
        remaining: usize,
        /// Already redacted at construction; a `Drop` impl is the last place to
        /// remember to redact something.
        endpoint: String,
        /// Bumped by the total lost, so the loss is observable and not merely
        /// logged — a test can assert it without standing up a subscriber.
        lost: &'a std::sync::atomic::AtomicUsize,
    }

    impl CancelledDrain<'_> {
        /// The drain ended on a path that reports for itself.
        fn disarm(&mut self) {
            self.remaining = 0;
        }
    }

    impl Drop for CancelledDrain<'_> {
        fn drop(&mut self) {
            if self.remaining == 0 {
                return;
            }
            self.lost
                .fetch_add(self.remaining, std::sync::atomic::Ordering::Relaxed);
            // `warn!` rather than `debug!`, and this is the one place in the
            // module where that is not the transient/permanent rule at work.
            // It is bounded — a drain is cancelled at most once per shutdown —
            // and it is the only notice an operator gets that their restarts
            // are costing them the end of every session's telemetry. A `debug!`
            // here would be the same silence the count was added to break.
            tracing::warn!(
                endpoint = %self.endpoint,
                dropped = self.remaining,
                "[analytics] the drain was cancelled before it finished — almost always \
                 the shutdown flush running out of its budget. These events are lost. \
                 OpenPanel has no batch endpoint, so a queue costs one request per \
                 event; a collector that answers slowly, or a busy queue, will not fit \
                 the budget."
            );
        }
    }

    /// Whether `endpoint` is a plain `http` URL, and so one whose safety rests
    /// on the request never leaving the host.
    ///
    /// Parsed with `url` rather than matched on a `http://` prefix, for the
    /// reason `config::is_usable_endpoint` gives at length: the transport's own
    /// parser is the only one whose answer is the operative one, and `HTTP://`
    /// is a legal spelling that a prefix match reads as safe.
    ///
    /// A value that does not parse answers `false`, which is the harmless
    /// direction *here* — it can only leave the system proxy enabled for an
    /// endpoint that `resolve` has already refused to report to, so no request
    /// is ever built from it.
    pub(super) fn is_cleartext(endpoint: &str) -> bool {
        url::Url::parse(endpoint).is_ok_and(|parsed| parsed.scheme() == "http")
    }

    /// Whether `status` is the collector's answer about **itself** rather than
    /// about the event that happened to be in flight.
    ///
    /// Three statuses reach the drain that are not per-event verdicts, and they
    /// split by whether they resolve on their own. A `401` and a `3xx` are
    /// permanent misconfigurations, so each gets its own said-once `warn!`.
    /// These are the transient half: `429` is the collector or its proxy asking
    /// for less traffic, and a `5xx` is it failing to serve at all. Neither says
    /// anything about the body that was posted, so every event behind it in the
    /// queue would get the same answer.
    ///
    /// Without this the drain treated them as a rejected *event* and carried on,
    /// which is the worst available response to `503`: up to [`MAX_QUEUED`]
    /// requests aimed at a service that has just said it is overloaded, and
    /// again at the next [`FLUSH_INTERVAL`], for as long as the collector stays
    /// down. That is the same runaway [`Inner::report_refused_credential`] was
    /// added to stop, arriving from the transient direction — an analytics
    /// client should not be the thing that keeps an operator's collector down.
    ///
    /// **`408` and `425` are deliberately not here.** Both are arguably
    /// retryable, but neither is evidence the collector is unwell, and widening
    /// this predicate costs a whole drain each time it is wrong. `4xx` other
    /// than `401` and `429` stays per-event, which is the reading that loses the
    /// least when it is mistaken: one dropped event rather than a whole drain.
    pub(super) fn is_collector_wide(status: reqwest::StatusCode) -> bool {
        status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }

    /// The one rendering of a transport failure this module is allowed to log.
    ///
    /// `reqwest::Error` keeps the request URL and prints it — `… for url (…)` —
    /// and that URL is `OPENCOMPANY_ANALYTICS_ENDPOINT`. A self-hosted collector
    /// is routinely reached through an authenticated proxy, and that is
    /// precisely where the proxy's key lives: in userinfo
    /// (`https://user:key@host/track`) or in the query string (`?key=…`). So a
    /// collector that merely goes unreachable wrote the operator's credential
    /// into container logs, on a path the boot line's redaction never touched
    /// and the `ClientCredentials` redaction guards different strings from
    /// entirely.
    ///
    /// `without_url` **removes** the URL rather than rewriting it, which is why
    /// this is not a second redaction surface to keep in step with
    /// `boot::loggable_endpoint`. There is nothing here to diverge: the error
    /// carries no URL at all, and the destination on the same log line comes
    /// from that one helper, so the transport learns about a new place a URL can
    /// hold a secret at the same moment the boot line does.
    pub(super) fn loggable_send_error(error: reqwest::Error) -> String {
        error.without_url().to_string()
    }

    impl Inner {
        /// Takes everything queued and posts it, **one request per event**.
        /// Every failure is swallowed after one debug line: a dead collector is
        /// a no-op, per #1739's constraints.
        ///
        /// Serialized: a caller entering while another drain is in flight waits
        /// for it and then takes whatever has arrived since. That is what makes
        /// [`Tracker::flush`] a real guarantee rather than a queue inspection —
        /// see [`Inner::sending`].
        ///
        /// # Why the queue survived the loss of batching
        ///
        /// The obvious reading of "OpenPanel has no batch endpoint" is to drop
        /// the queue and fire a request from `track` itself. That is the wrong
        /// trade twice over. `track` is called from the cycle bracket and the
        /// usage meter, both on a turn's hot path, and it is **synchronous and
        /// infallible** by contract — it cannot await, so firing from it means
        /// spawning a task per event, which is unbounded concurrency against a
        /// collector the process does not control, with no back-pressure and no
        /// ceiling on memory. The queue is what bounds both: at most
        /// [`MAX_QUEUED`] events exist at once, and at most one drain runs at a
        /// time.
        ///
        /// # The unreachable collector, and why the drain gives up early
        ///
        /// Each request has its own [`SEND_TIMEOUT`]. Sequentially, a full
        /// queue against a black-holing collector would be
        /// `500 × 5s` — over forty minutes of a task doing nothing but proving
        /// the same thing five hundred times, and forty minutes in which the
        /// shutdown flush would block behind [`Inner::sending`] until its own
        /// budget cut it off. So a **transport** error abandons the rest of the
        /// drain: the collector is down, the remaining events are going
        /// nowhere, and the next interval will try again with whatever has
        /// accumulated since. They are dropped rather than requeued, because
        /// requeuing an unbounded backlog is how a bounded queue stops being
        /// bounded.
        ///
        /// An **HTTP status** failure does not abandon it. That is a per-event
        /// answer — a rejected event name, a payload the collector will not
        /// accept — and the events behind it may well be fine. Treating the two
        /// alike would let one malformed event silence a whole drain.
        ///
        /// **A `401` is the exception, because it is not a per-event answer at
        /// all.** It is the collector's verdict on this process's credential,
        /// so every event behind it in the queue will get the same one. Carrying
        /// on would fire up to [`MAX_QUEUED`] requests, every
        /// [`FLUSH_INTERVAL`], for the life of a misconfigured tenant — a
        /// thousand pointless requests a minute at the operator's own
        /// collector, to learn something already known. So it abandons the drain
        /// like a transport failure, and says so once.
        ///
        /// **A `3xx` is the same shape of exception, for the same reason.**
        /// This client follows no redirect at all — see
        /// [`HttpOpenPanelTracker::new`] for why the alternative hands the
        /// write secret to a host nobody configured — so a redirecting endpoint
        /// arrives here as a plain non-success response that will never
        /// resolve. It is a verdict on the endpoint, not on the event, so it
        /// abandons the drain and warns once rather than logging a `debug!` per
        /// event for the life of the process.
        ///
        /// # The tail a cancelled drain loses, and why it is said out loud
        ///
        /// Every path above ends the drain *deliberately* and says how many
        /// events it dropped. There is one that does not: the shutdown flush is
        /// wrapped in a [`tokio::time::timeout`] at its call site
        /// (`src/bin/opencompany.rs`, bounded by
        /// `server::shutdown::flush_budget`, at most **2s**), so when the budget
        /// runs out this future is simply **dropped mid-drain**. The events it
        /// had already taken out of the queue are gone, and nothing in this
        /// module ever said so — the only trace was a `debug!` at the call site
        /// that names no count.
        ///
        /// That gap is new with OpenPanel, and it is a direct consequence of
        /// there being no batch endpoint. Mixpanel's whole queue left in **one**
        /// request, so 2s was never the binding constraint; one request per
        /// event means a queue of `n` costs `n` round trips, and at a very
        /// ordinary 25 ms each the budget is spent after about eighty. A busy
        /// tenant restarting therefore loses the tail of its telemetry, quietly,
        /// on every rollout.
        ///
        /// [`CancelledDrain`] makes that loud instead. It is armed with the
        /// number of events still unsent, disarmed by every deliberate exit
        /// above (each of which logs its own line), and on `Drop` — which is
        /// what cancellation *is* — reports the count that never left.
        ///
        /// **The loss itself is not fixed here, on purpose.** The obvious
        /// remedy is to send with bounded concurrency, which would fit roughly
        /// `concurrency ×` more events into the same budget. It is declined
        /// because it is paid for out of the guarantee directly above: a drain
        /// that issues eight requests at once against a black-holing collector
        /// opens eight connections rather than one, and
        /// `an_unreachable_collector_costs_one_timeout_for_the_whole_drain`
        /// asserts exactly one. Trading a bounded shutdown for a multiplied
        /// hammering of a collector that is already unreachable is the wrong
        /// direction, and #1739 is explicit that telemetry loss beats a
        /// shutdown overrun — the budget exists because an overrun buys a
        /// `SIGKILL` mid-turn. The real fix is a batch endpoint on the
        /// collector, which OpenPanel does not have.
        async fn drain(&self) {
            let _sending = self.sending.lock().await;
            let events = {
                let mut queue = self.queue.lock().expect("analytics queue");
                if queue.is_empty() {
                    return;
                }
                std::mem::take(&mut *queue)
            };

            let total = events.len();
            // Armed for the whole loop. Every `return` below disarms it first,
            // because those paths log their own count; what is left for the
            // guard is the one exit that cannot log for itself — being dropped.
            let mut cancelled = CancelledDrain {
                remaining: total,
                endpoint: crate::analytics::boot::loggable_endpoint(&self.endpoint),
                lost: &self.lost_to_cancellation,
            };
            for (sent, event) in events.into_iter().enumerate() {
                cancelled.remaining = total - sent;
                match self.client.post(&self.endpoint).json(&event).send().await {
                    Ok(response) if response.status().is_success() => {}
                    // Not a per-event answer: the credential is wrong for every
                    // event behind this one too.
                    Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
                        cancelled.disarm();
                        self.report_refused_credential(total - sent);
                        return;
                    }
                    // Also not a per-event answer, and — because this client
                    // follows no redirects — not one that resolves itself.
                    Ok(response) if response.status().is_redirection() => {
                        cancelled.disarm();
                        self.report_redirected_endpoint(response.status(), total - sent);
                        return;
                    }
                    // Not a per-event answer either — but unlike the two above,
                    // this one resolves itself, so it gets the transient
                    // treatment rather than a `warn!`.
                    Ok(response) if is_collector_wide(response.status()) => {
                        cancelled.disarm();
                        tracing::debug!(
                            endpoint = %crate::analytics::boot::loggable_endpoint(&self.endpoint),
                            status = %response.status(),
                            dropped = total - sent,
                            "[analytics] the collector cannot take traffic right now; \
                             dropping the rest of this drain"
                        );
                        return;
                    }
                    Ok(response) => tracing::debug!(
                        status = %response.status(),
                        "[analytics] the collector refused an event; dropping it"
                    ),
                    Err(error) => {
                        cancelled.disarm();
                        tracing::debug!(
                            endpoint = %crate::analytics::boot::loggable_endpoint(&self.endpoint),
                            error = %loggable_send_error(error),
                            dropped = total - sent,
                            "[analytics] could not reach the collector; dropping the rest \
                             of this drain"
                        );
                        return;
                    }
                }
            }
            cancelled.disarm();
        }

        /// Says once, out loud, that the configured endpoint redirects and that
        /// nothing is being sent as a result.
        ///
        /// **Never prints the `Location` header.** It is a URL the collector
        /// chose, and a URL is the one place this module already knows a
        /// credential hides — an authenticated proxy's key lives in the
        /// userinfo or the query string, which is the whole reason
        /// [`loggable_send_error`] exists. A redirect target is *less* trusted
        /// than the configured endpoint, not more: the operator did not write
        /// it, and printing it verbatim would hand a hostile or merely careless
        /// collector a way to write arbitrary text into a tenant's logs. The
        /// status code alone is enough to act on, and the fix is in the
        /// operator's own environment file either way.
        fn report_redirected_endpoint(&self, status: reqwest::StatusCode, dropped: usize) {
            use std::sync::atomic::Ordering;
            if self.endpoint_redirects.swap(true, Ordering::Relaxed) {
                return;
            }
            tracing::warn!(
                endpoint = %crate::analytics::boot::loggable_endpoint(&self.endpoint),
                status = %status,
                dropped,
                "[analytics] the collector answered with a redirect, which this client \
                 never follows: the credential headers would otherwise travel to a host \
                 OPENCOMPANY_ANALYTICS_ENDPOINT does not name. Every event will be \
                 dropped until that variable points at the collector directly. For a \
                 self-hosted OpenPanel behind its bundled Caddy that is \
                 https://<your-domain>/api/track."
            );
        }

        /// Says once, out loud, that the collector will not accept this
        /// process's credential. Never quotes it.
        fn report_refused_credential(&self, dropped: usize) {
            use std::sync::atomic::Ordering;
            if self.credential_refused.swap(true, Ordering::Relaxed) {
                return;
            }
            tracing::warn!(
                endpoint = %crate::analytics::boot::loggable_endpoint(&self.endpoint),
                dropped,
                "[analytics] the collector refused this instance's credential (401). \
                 Every event will be dropped until OPENCOMPANY_ANALYTICS_CLIENT_ID and \
                 OPENCOMPANY_ANALYTICS_CLIENT_SECRET name a write client on that \
                 collector. Note that OpenPanel requires the client id to be a UUIDv4."
            );
        }
    }

    #[async_trait]
    impl Tracker for HttpOpenPanelTracker {
        fn track(&self, event: Event) {
            let body = {
                let envelope = self.inner.envelope.read().expect("analytics envelope");
                payload(&envelope, &event)
            };
            let mut queue = self.inner.queue.lock().expect("analytics queue");
            if queue.len() >= MAX_QUEUED {
                queue.remove(0);
            }
            queue.push(body);
        }

        async fn flush(&self) {
            // Waits on any in-flight periodic drain before taking what is left,
            // so a shutdown overlapping the 30-second loop does not return while
            // the previous drain is still on the wire.
            self.inner.drain().await;
        }

        fn observe_cognition(&self, cognition: crate::ports::brain::Cognition) {
            self.inner
                .envelope
                .write()
                .expect("analytics envelope")
                .set_cognition(cognition);
        }
    }
}

#[cfg(all(test, feature = "analytics"))]
mod test {
    use super::*;
    use crate::analytics::config::{
        CLIENT_ID_ENV, CLIENT_SECRET_ENV, ENABLE_ENV, ENDPOINT_ENV, resolve,
    };
    use crate::analytics::types::OpaqueId;
    use crate::analytics::{Event, Outcome, Trigger};
    use crate::app::config::MapEnv;
    use crate::app::deployment::{DEPLOYMENT_ENV, Deployment};
    use crate::ports::brain::Cognition;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// Obviously-fake credentials. Never a real one, in a file or anywhere else.
    const TEST_CLIENT_ID: &str = "not-a-real-client-id";
    const TEST_CLIENT_SECRET: &str = "not-a-real-client-secret";

    /// The headers each request arrived with, in order, name and value.
    type SeenHeaders = Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>>;

    /// A local collector that counts what it is sent and keeps the bodies and
    /// the headers.
    struct Collector {
        hits: Arc<AtomicUsize>,
        bodies: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        headers: SeenHeaders,
        url: String,
        shutdown: tokio::sync::oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    }

    async fn spawn_collector() -> Collector {
        spawn_collector_with(Duration::ZERO, 0, axum::http::StatusCode::BAD_REQUEST).await
    }

    /// A collector that takes `delay` to answer and refuses the first
    /// `refuse_first` requests with `refusal`, so a test can observe what
    /// happens while a request is in flight, what happens after one event is
    /// rejected, and what happens when the refusal is about the credential
    /// rather than about the event.
    async fn spawn_collector_with(
        delay: Duration,
        refuse_first: usize,
        refusal: axum::http::StatusCode,
    ) -> Collector {
        let hits = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let headers = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_hits = hits.clone();
        let seen_bodies = bodies.clone();
        let seen_headers = headers.clone();

        let app = axum::Router::new().route(
            "/track",
            axum::routing::post(
                move |received: axum::http::HeaderMap,
                      axum::Json(body): axum::Json<serde_json::Value>| {
                    let hits = seen_hits.clone();
                    let bodies = seen_bodies.clone();
                    let headers = seen_headers.clone();
                    async move {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        let seen = hits.fetch_add(1, Ordering::SeqCst);
                        bodies.lock().unwrap().push(body);
                        headers.lock().unwrap().push(
                            received
                                .iter()
                                .map(|(name, value)| {
                                    (
                                        name.as_str().to_string(),
                                        value.to_str().unwrap_or_default().to_string(),
                                    )
                                })
                                .collect(),
                        );
                        if seen < refuse_first {
                            refusal
                        } else {
                            axum::http::StatusCode::OK
                        }
                    }
                },
            ),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (shutdown, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });

        Collector {
            hits,
            bodies,
            headers,
            url,
            shutdown,
            handle,
        }
    }

    impl Collector {
        async fn stop(self) {
            let _ = self.shutdown.send(());
            let _ = self.handle.await;
        }

        fn header(&self, request: usize, name: &str) -> Option<String> {
            self.headers.lock().unwrap()[request]
                .iter()
                .find(|(seen, _)| seen == name)
                .map(|(_, value)| value.clone())
        }
    }

    fn envelope() -> Envelope {
        Envelope::new(
            OpaqueId::instance("0123456789abcdef0123456789abcdef"),
            Deployment::HostedTenant,
            Cognition::default(),
        )
    }

    /// A reporting environment pointed at `endpoint`, which `pairs` overrides.
    fn env(endpoint: &str, pairs: &[(&str, &str)]) -> MapEnv {
        let mut all = vec![
            (CLIENT_ID_ENV, TEST_CLIENT_ID),
            (CLIENT_SECRET_ENV, TEST_CLIENT_SECRET),
            (ENDPOINT_ENV, endpoint),
        ];
        all.extend_from_slice(pairs);
        MapEnv::new(all)
    }

    fn events() -> Vec<Event> {
        vec![
            Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            },
            Event::TurnFinished {
                trigger: Trigger::OperatorMessage,
                outcome: Outcome::Ok,
                failure: None,
                duration_ms: 12,
                effects_executed: 0,
                approvals_parked: 0,
            },
        ]
    }

    /// **Issue #1739's first acceptance criterion.** A build that *has* the
    /// transport compiled in, pointed at a live collector, with a credential in
    /// the environment, and not declared hosted: it must send nothing.
    ///
    /// Note what is deliberately stacked against the assertion — the feature is
    /// on, the client exists, the endpoint resolves, both halves of the
    /// credential are present. The only thing that is not is consent. That is
    /// the configuration a self-hoster who copied a hosted deployment's env file
    /// would have.
    #[tokio::test]
    async fn a_self_hosted_build_makes_no_request() {
        let collector = spawn_collector().await;
        let env = env(&collector.url, &[]);

        let decision = resolve(Deployment::from_env(&env), &env);
        let tracker = build(&decision, envelope());
        for event in events() {
            tracker.track(event);
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            0,
            "a self-hosted build must not dial out"
        );
        collector.stop().await;
    }

    /// The positive control that makes the test above non-vacuous: the same
    /// collector, the same events, the same code path, one variable changed.
    ///
    /// It also pins the whole wire contract — **one request per event**, the two
    /// auth headers by their exact spelling, and OpenPanel's discriminated-union
    /// body with the identity as `profileId` rather than as a property.
    #[tokio::test]
    async fn a_hosted_tenant_reports_with_the_full_envelope() {
        let collector = spawn_collector().await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);

        let decision = resolve(Deployment::from_env(&env), &env);
        assert!(decision.reports(), "{decision:?}");
        let tracker = build(&decision, envelope());
        for event in events() {
            tracker.track(event);
        }
        tracker.flush().await;

        // Two events, two requests. OpenPanel has no batch endpoint, so this is
        // the one number that changed shape rather than value.
        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            2,
            "one request per event"
        );

        let bodies = collector.bodies.lock().unwrap().clone();
        let first = &bodies[0];
        assert_eq!(first["type"], "track");
        assert_eq!(first["payload"]["name"], "instance_started");
        assert_eq!(
            first["payload"]["profileId"],
            "i_0123456789abcdef0123456789abcdef"
        );

        let properties = &first["payload"]["properties"];
        assert_eq!(properties["deployment"], "hosted-tenant");
        assert!(properties["app_version"].is_string());
        assert!(properties["harness_in_build"].is_boolean());
        assert_eq!(bodies[1]["payload"]["name"], "turn_finished");

        for request in 0..2 {
            assert_eq!(
                collector.header(request, CLIENT_ID_HEADER).as_deref(),
                Some(TEST_CLIENT_ID),
                "request {request} carried no client id header"
            );
            assert_eq!(
                collector.header(request, CLIENT_SECRET_HEADER).as_deref(),
                Some(TEST_CLIENT_SECRET),
                "request {request} carried no client secret header"
            );
        }

        collector.stop().await;
    }

    /// **The credential travels in headers and nowhere else.**
    ///
    /// The transport this replaced stamped Mixpanel's token into every event's
    /// property bag, which put a credential one `dbg!` away from a test fixture
    /// or a captured body. Nothing does that now, and this is the assertion that
    /// keeps it true: not one byte of either half appears in any body on the
    /// wire.
    #[tokio::test]
    async fn no_credential_reaches_the_request_body() {
        let collector = spawn_collector().await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for event in events() {
            tracker.track(event);
        }
        tracker.flush().await;

        for body in collector.bodies.lock().unwrap().iter() {
            let rendered = body.to_string().to_ascii_lowercase();
            for half in [TEST_CLIENT_ID, TEST_CLIENT_SECRET] {
                assert!(
                    !rendered.contains(&half.to_ascii_lowercase()),
                    "the body carried {half}: {rendered}"
                );
            }
        }
        // The self-check: the needle really is findable where it *is* supposed
        // to be, or the guard above would pass on a transport that sent no
        // credential at all.
        assert_eq!(
            collector.header(0, CLIENT_SECRET_HEADER).as_deref(),
            Some(TEST_CLIENT_SECRET)
        );

        collector.stop().await;
    }

    /// An operator who switched it off stays off, even on a hosted tenant.
    #[tokio::test]
    async fn an_opted_out_tenant_makes_no_request() {
        let collector = spawn_collector().await;
        let env = env(
            &collector.url,
            &[(DEPLOYMENT_ENV, "hosted-tenant"), (ENABLE_ENV, "off")],
        );

        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for event in events() {
            tracker.track(event);
        }
        tracker.flush().await;

        assert_eq!(collector.hits.load(Ordering::SeqCst), 0);
        collector.stop().await;
    }

    /// **A refused event does not stop the drain.**
    ///
    /// A per-event HTTP status is a per-event answer — a name the collector
    /// rejects, a body it will not take — and the events behind it may be
    /// perfectly good. Without this, one malformed event would silence an entire
    /// drain, which is the failure mode that matters most in a module where
    /// every other failure is already silent.
    #[tokio::test]
    async fn a_refused_event_does_not_stop_the_drain() {
        let collector =
            spawn_collector_with(Duration::ZERO, 1, axum::http::StatusCode::BAD_REQUEST).await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            3,
            "the two events behind a refused one must still be attempted"
        );
        collector.stop().await;
    }

    /// **A refused *credential* does stop the drain, unlike a refused event.**
    ///
    /// A `401` is not the collector's verdict on one event; it is its verdict on
    /// this process, so every event behind it in the queue gets the same answer.
    /// Carrying on would fire up to 500 requests every thirty seconds for the
    /// life of a misconfigured tenant — a thousand a minute at the operator's
    /// own collector — to learn something already known.
    ///
    /// The contrast with `a_refused_event_does_not_stop_the_drain` is the point:
    /// same collector, same three events, one status code changed, opposite
    /// behaviour. Neither test means much without the other.
    #[tokio::test]
    async fn a_refused_credential_stops_the_drain() {
        let collector = spawn_collector_with(
            Duration::ZERO,
            usize::MAX,
            axum::http::StatusCode::UNAUTHORIZED,
        )
        .await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            1,
            "a 401 is about the credential, not about the event, so the two behind it \
             must not be attempted"
        );
        collector.stop().await;
    }

    /// **The write secret never follows a redirect to another host.**
    ///
    /// The leak this closes is not exotic. `reqwest`'s default policy follows
    /// ten hops, and its cross-origin sanitization
    /// (`redirect.rs::remove_sensitive_headers`, 0.12.28) removes exactly
    /// `Authorization`, `Cookie`, `cookie2`, `Proxy-Authorization` and
    /// `WWW-Authenticate` — and nothing else. `openpanel-client-secret` is none
    /// of them, so before [`reqwest::redirect::Policy::none`] a single `307`
    /// from the configured collector handed this instance's long-lived write
    /// credential to whatever host the `Location` named.
    ///
    /// `set_sensitive` is not a defence and is worth naming, because it looks
    /// like one in the source: it governs `Debug` output and HPACK indexing,
    /// and has no bearing on which headers survive a hop.
    ///
    /// Two collectors on two ports, so `next.port_or_known_default() !=
    /// previous.port_or_known_default()` — reqwest's own cross-host test — is
    /// unambiguously true and the sanitization it does perform is in play. The
    /// assertion is on the **destination**: it must be untouched. Asserting
    /// only "the redirect was not followed" would pass against a client that
    /// followed it and merely dropped the header, which is a different and
    /// weaker property than the one being claimed.
    #[tokio::test]
    async fn a_redirect_never_carries_the_credential_to_another_host() {
        // Where a followed redirect would land: a real collector that records
        // every header of everything it is sent.
        let elsewhere = spawn_collector().await;
        let target = elsewhere.url.clone();

        // The configured endpoint: answers every POST with a 307 to the other
        // collector, on a different port and so a different origin.
        let redirected = Arc::new(AtomicUsize::new(0));
        let counted = redirected.clone();
        let app = axum::Router::new().route(
            "/track",
            axum::routing::post(move || {
                let hits = counted.clone();
                let target = target.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [(axum::http::header::LOCATION, target)],
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (shutdown, rx) = tokio::sync::oneshot::channel();
        let redirector = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });

        let env = env(&url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            elsewhere.hits.load(Ordering::SeqCst),
            0,
            "a redirect must not carry the client credential to a host \
             OPENCOMPANY_ANALYTICS_ENDPOINT never named"
        );
        assert_eq!(
            redirected.load(Ordering::SeqCst),
            1,
            "a redirecting endpoint is a verdict on the endpoint, not on one event, so \
             the two behind it must not be attempted"
        );

        let _ = shutdown.send(());
        let _ = redirector.await;
        elsewhere.stop().await;
    }

    /// The control that makes the test above non-vacuous.
    ///
    /// `elsewhere.hits == 0` would also hold if the destination collector were
    /// simply broken, or if `spawn_collector` did not record what it received.
    /// Same collector, same events, pointed at directly rather than through a
    /// redirect: it must see all three requests, carrying the secret, so the
    /// zero above is about the redirect and nothing else.
    #[tokio::test]
    async fn the_redirect_destination_would_have_recorded_the_credential() {
        let elsewhere = spawn_collector().await;
        let env = env(&elsewhere.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            elsewhere.hits.load(Ordering::SeqCst),
            3,
            "the destination records what it is sent, so the zero above is the redirect \
             policy rather than a collector that counts nothing"
        );
        assert_eq!(
            elsewhere.header(0, CLIENT_SECRET_HEADER).as_deref(),
            Some(TEST_CLIENT_SECRET),
            "and it records the credential header, which is the thing that must not \
             have arrived across a redirect"
        );
        elsewhere.stop().await;
    }

    /// The queue is bounded. An unreachable collector must cost telemetry, not
    /// a tenant container's memory — and `track` must not block whatever the
    /// collector does.
    #[tokio::test]
    async fn the_queue_is_bounded() {
        // Nothing listens here; the point is that `track` never blocks and
        // never grows without bound whatever the collector does.
        let env = env(
            "http://127.0.0.1:1/track",
            &[(DEPLOYMENT_ENV, "hosted-tenant")],
        );
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for _ in 0..2_000 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        // No assertion on an internal count — the observable property is that
        // this returns at all, promptly, with no reachable collector.
    }

    /// **An unreachable collector costs one timeout, not one per queued event.**
    ///
    /// This is the case losing the batch endpoint created. With a single POST
    /// carrying everything, an unreachable collector cost exactly one
    /// `SEND_TIMEOUT`. One request per event, drained sequentially, would cost
    /// `queued × SEND_TIMEOUT` — up to forty minutes at a full queue — during
    /// which the shutdown flush is blocked behind the same lock and a
    /// container's `SIGTERM` budget is long gone.
    ///
    /// Asserted on **connections the collector actually accepted**, not on
    /// elapsed time, because a timing threshold on a black-holing socket is a
    /// flaky test. The listener accepts and never answers, so each attempt is a
    /// real connection that pays the full timeout: three queued events must
    /// produce **one** connection, not three.
    #[tokio::test]
    async fn an_unreachable_collector_costs_one_timeout_for_the_whole_drain() {
        let accepted = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let counted = accepted.clone();
        // Accepts and holds. Never reads, never answers — the shape a collector
        // behind a wedged proxy has, and the one a refused port does not
        // exercise because it fails instantly.
        let black_hole = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });

        let env = env(&url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());
        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }

        let started = Instant::now();
        tracker.flush().await;
        let waited = started.elapsed();

        assert_eq!(
            accepted.load(Ordering::SeqCst),
            1,
            "the drain must give up after the first transport failure, not pay a \
             timeout for every queued event"
        );
        assert!(
            waited < Duration::from_secs(12),
            "a drain against a black hole took {waited:?}, which is more than one \
             send timeout and would outlive a container's shutdown budget"
        );
        // And the flush still returned, which is the property `Tracker::flush`
        // promises: analytics never prevents a shutdown.
        black_hole.abort();
    }

    /// **A transport failure must not carry the collector credential.**
    ///
    /// `OPENCOMPANY_ANALYTICS_ENDPOINT` names a collector the operator runs, and
    /// such a collector is routinely fronted by an authenticated proxy, which
    /// carries its key in one of the two places a URL can hold one.
    /// `reqwest::Error` retains the request URL and prints it, so an unreachable
    /// collector — a routine event, not an exotic one — wrote that key into the
    /// debug log.
    ///
    /// Measured against reqwest 0.12.28 rather than assumed, and the two places
    /// do **not** behave alike:
    ///
    /// | in the endpoint | what `reqwest::Error`'s `Display` printed |
    /// |---|---|
    /// | `http://someone:KEY@127.0.0.1:1/track` | `… for url (http://127.0.0.1:1/track)` — userinfo already stripped |
    /// | `http://127.0.0.1:1/track?key=KEY` | `… for url (http://127.0.0.1:1/track?key=KEY)` — **leaked verbatim** |
    ///
    /// So the query string is the live leak; userinfo is not, today. Both are
    /// covered here anyway, because "the dependency strips it" is not a
    /// property this crate owns — it is one `cargo update` from being false,
    /// and nothing here would fail when it changed. `without_url` removes the
    /// URL outright, so neither shape can reach the line whatever reqwest
    /// decides to print.
    ///
    /// Asserted **case-insensitively**, with the self-check below: this guard
    /// once shipped in a form that passed a deliberate leak, because the value
    /// came back lowercased.
    #[tokio::test]
    async fn a_transport_failure_never_carries_the_endpoint_credential() {
        const SECRET: &str = "NotARealCollectorKey";
        let needle = SECRET.to_ascii_lowercase();

        // The self-check, on the shape that is measurably still leaking. A
        // guard that cannot find the needle in the **unstripped** error proves
        // nothing about the stripped one — the needle may never have been
        // there at all. If reqwest ever starts redacting query strings too,
        // this fails loudly and says so, rather than leaving a guard behind
        // that asserts nothing.
        let leaky = format!("http://127.0.0.1:1/track?key={SECRET}");
        let unstripped = send_failing(&leaky).await.to_string();
        assert!(
            unstripped.to_ascii_lowercase().contains(&needle),
            "the needle must be findable before stripping, or this guard is \
             vacuous: {unstripped}"
        );

        for endpoint in [
            // Port 1 refuses, so each of these is a real transport error rather
            // than a fabricated one.
            format!("http://someone:{SECRET}@127.0.0.1:1/track"),
            leaky.clone(),
            format!("http://someone:{SECRET}@127.0.0.1:1/track?key={SECRET}"),
        ] {
            let logged = super::http::loggable_send_error(send_failing(&endpoint).await);
            assert!(
                !logged.to_ascii_lowercase().contains(&needle),
                "the transport error leaked the collector credential from \
                 {endpoint:?}: {logged}"
            );
            assert!(
                !logged.is_empty(),
                "stripping the URL must still leave the operator a reason: {logged}"
            );

            // And the destination is still named on the same line — through the
            // one redaction helper the boot line uses, not a second one.
            let named = crate::analytics::boot::loggable_endpoint(&endpoint);
            assert!(
                !named.to_ascii_lowercase().contains(&needle),
                "the endpoint field leaked it instead: {named}"
            );
            assert!(
                named.contains("127.0.0.1"),
                "the operator still has to be able to tell where it was going: {named}"
            );
        }
    }

    /// One real, refused request. Nothing listens on port 1.
    async fn send_failing(endpoint: &str) -> reqwest::Error {
        reqwest::Client::new()
            .post(endpoint)
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect_err("nothing listens on port 1")
    }

    /// **A shutdown flush waits for a send already in flight.**
    ///
    /// The periodic drain takes the whole queue before it awaits its POSTs, so
    /// a flush that only inspected the queue would find it empty, return at
    /// once, and let process exit cancel the request carrying the event —
    /// losing telemetry precisely when the collector is slow, which is the one
    /// case the graceful flush exists for.
    ///
    /// Asserted by timing, against a collector that takes 600ms: the second
    /// flush must not return before the first request completes. The threshold
    /// is 300ms against a 600ms delay, so it neither trips on scheduling jitter
    /// nor passes without the wait (the unserialized version returns in
    /// microseconds).
    #[tokio::test]
    async fn a_flush_waits_for_a_send_already_in_flight() {
        let collector =
            spawn_collector_with(Duration::from_millis(600), 0, axum::http::StatusCode::OK).await;
        let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
        let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

        tracker.track(Event::InstanceStarted {
            companies: 1,
            storage: "fs",
            setup_complete: true,
        });

        // Stands in for the 30-second drain loop: it takes the queue and is
        // then parked on the POST.
        let first = {
            let tracker = tracker.clone();
            tokio::spawn(async move { tracker.flush().await })
        };
        // Long enough for the spawned task to take the queue and start its
        // request, short enough to be well inside the 600ms the collector takes.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let started = Instant::now();
        tracker.flush().await;
        let waited = started.elapsed();

        assert!(
            waited >= Duration::from_millis(300),
            "the flush returned in {waited:?} while a send was still in flight; \
             on a real shutdown that event would be cancelled with the process"
        );

        first.await.expect("the in-flight send finished");
        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            1,
            "the event really was in flight and really did land"
        );
        collector.stop().await;
    }

    /// **A collector-wide status stops the drain, like a refused credential.**
    ///
    /// `429`, `502`, `503` are the collector saying it cannot take traffic —
    /// not a verdict on the body that happened to be in flight. Treating one as
    /// a rejected *event* and carrying on is the worst available response: up
    /// to `MAX_QUEUED` requests aimed at a service that has just said it is
    /// overloaded, and again at the next `FLUSH_INTERVAL` for as long as it
    /// stays down. An analytics client must not be the thing that keeps an
    /// operator's own collector down.
    ///
    /// The contrast with `a_refused_event_does_not_stop_the_drain` is the whole
    /// point, and it is the same contrast a `401` draws: same collector, same
    /// three events, one status code apart, opposite behaviour.
    #[tokio::test]
    async fn a_collector_that_cannot_take_traffic_stops_the_drain() {
        for refusal in [
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            axum::http::StatusCode::BAD_GATEWAY,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            let collector = spawn_collector_with(Duration::ZERO, usize::MAX, refusal).await;
            let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
            let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

            for _ in 0..3 {
                tracker.track(Event::InstanceStarted {
                    companies: 1,
                    storage: "fs",
                    setup_complete: true,
                });
            }
            tracker.flush().await;

            assert_eq!(
                collector.hits.load(Ordering::SeqCst),
                1,
                "{refusal} is the collector's answer about itself, so the two events \
                 behind it must not be attempted"
            );
            collector.stop().await;
        }
    }

    /// The control for the test above: a status that really *is* about one
    /// event must still not stop the drain.
    ///
    /// Without it, "stops the drain" would be satisfied by a client that gave
    /// up on any refusal at all, which is the behaviour
    /// `a_refused_event_does_not_stop_the_drain` exists to forbid. `400` and
    /// `404` are the two an operator actually meets — a body OpenPanel will not
    /// take, and a `/track` path typed wrong — and neither is a reason to
    /// abandon the events queued behind it.
    #[tokio::test]
    async fn a_per_event_refusal_still_does_not_stop_the_drain() {
        for refusal in [
            axum::http::StatusCode::BAD_REQUEST,
            axum::http::StatusCode::NOT_FOUND,
        ] {
            let collector = spawn_collector_with(Duration::ZERO, usize::MAX, refusal).await;
            let env = env(&collector.url, &[(DEPLOYMENT_ENV, "hosted-tenant")]);
            let tracker = build(&resolve(Deployment::from_env(&env), &env), envelope());

            for _ in 0..3 {
                tracker.track(Event::InstanceStarted {
                    companies: 1,
                    storage: "fs",
                    setup_complete: true,
                });
            }
            tracker.flush().await;

            assert_eq!(
                collector.hits.load(Ordering::SeqCst),
                3,
                "{refusal} is about one event, so the two behind it must still be tried"
            );
            collector.stop().await;
        }
    }

    /// **A drain cancelled mid-flight says how many events it lost.**
    ///
    /// This is the shutdown budget, reproduced. `src/bin/opencompany.rs` wraps
    /// the final flush in a `tokio::time::timeout` of at most two seconds
    /// (`server::shutdown::flush_budget`), and OpenPanel has no batch endpoint,
    /// so a queue of `n` costs `n` sequential round trips. When the budget runs
    /// out the future is **dropped mid-drain**: the events already taken off the
    /// queue are gone, and before `CancelledDrain` nothing in this module said
    /// so — the only trace was a `debug!` at the call site naming no count.
    ///
    /// The loss is not fixed, deliberately (see `Inner::drain` for why bounded
    /// concurrency is the wrong trade against the black-hole guarantee). What
    /// is fixed is the silence, so this asserts the **count**, which is the part
    /// an operator can act on. Asserted on the counter rather than on a log
    /// line, because a test that needs a subscriber to see a regression is a
    /// test that stops seeing it the day the subscriber changes.
    ///
    /// A 300 ms collector and a 120 ms budget: the first event is still in
    /// flight when the timeout fires, so four of five are certain to be lost —
    /// no timing race, because the assertion is a lower bound rather than an
    /// exact count.
    #[tokio::test]
    async fn a_cancelled_drain_reports_the_tail_it_lost() {
        let collector =
            spawn_collector_with(Duration::from_millis(300), 0, axum::http::StatusCode::OK).await;
        // Built directly rather than through `build`, because the counter is on
        // the concrete type and `build` hands back an `Arc<dyn Tracker>`.
        let tracker = HttpOpenPanelTracker::new(
            &collector.url,
            &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
            envelope(),
        )
        .expect("the client builds");

        for _ in 0..5 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }

        assert_eq!(
            tracker.lost_to_cancellation(),
            0,
            "nothing is lost before a drain is cancelled, or the assertion below is \
             measuring the wrong thing"
        );

        // Stands in for the shutdown budget, an order of magnitude smaller so
        // the test does not take two seconds to prove a two-second bound.
        let outcome = tokio::time::timeout(Duration::from_millis(120), tracker.flush()).await;
        assert!(
            outcome.is_err(),
            "the flush finished inside the budget, so nothing was cancelled and this \
             test proves nothing"
        );

        assert!(
            tracker.lost_to_cancellation() >= 4,
            "a cancelled drain lost {} events and reported {} — the tail of a shutdown \
             flush must be counted, not dropped in silence",
            5 - collector.hits.load(Ordering::SeqCst),
            tracker.lost_to_cancellation()
        );
        collector.stop().await;
    }

    /// **A loopback endpoint does not go through the system proxy.**
    ///
    /// The loopback exception in `config::is_secure_endpoint` rests entirely on
    /// the claim that such a request does not leave the host. A system proxy
    /// makes that false: `reqwest`'s builder defaults to `auto_sys_proxy: true`,
    /// which reads `HTTP_PROXY`/`ALL_PROXY` and takes exclusions **only** from
    /// `NO_PROXY` — hyper-util 0.1.20's matcher has no implicit carve-out for
    /// `localhost` or `127.0.0.0/8` (read, not assumed). So on a host with a
    /// proxy configured, `http://127.0.0.1:…/track` went to the proxy in
    /// cleartext with both credential headers on it, and the endpoint check
    /// prevented nothing.
    ///
    /// Two servers and one variable: a stand-in "proxy" that records anything
    /// it is handed, and the real collector. With `HTTP_PROXY` pointing at the
    /// first, the request must still arrive at the second. Asserting the proxy
    /// saw **zero** is the security property; asserting the collector saw the
    /// events is what stops that zero from being vacuous.
    ///
    /// Mutates the process environment, so it holds the crate-wide
    /// [`crate::test_support::EnvVarGuard`] — `reqwest` reads these variables
    /// from the real environment at client-build time, which is the one thing
    /// this crate's `MapEnv` seam cannot intercept.
    #[tokio::test]
    async fn a_loopback_endpoint_never_goes_through_a_system_proxy() {
        // Stands in for a corporate proxy: records every request and would be
        // the thing receiving the credential if the client honoured it.
        let proxy = spawn_collector().await;
        let collector = spawn_collector().await;

        let tracker = {
            let env = crate::test_support::EnvVarGuard::capture(&[
                "HTTP_PROXY",
                "http_proxy",
                "ALL_PROXY",
                "all_proxy",
                "NO_PROXY",
                "no_proxy",
            ]);
            // A proxy for everything, and no exclusions at all — the shape that
            // used to divert this traffic.
            env.remove("NO_PROXY");
            env.remove("no_proxy");
            env.remove("http_proxy");
            env.remove("all_proxy");
            env.set("ALL_PROXY", proxy.url.trim_end_matches("/track"));
            env.set("HTTP_PROXY", proxy.url.trim_end_matches("/track"));
            // Built inside the guard: `reqwest` samples the environment here,
            // not at send time.
            HttpOpenPanelTracker::new(
                &collector.url,
                &crate::analytics::config::ClientCredentials::new(
                    TEST_CLIENT_ID,
                    TEST_CLIENT_SECRET,
                ),
                envelope(),
            )
            .expect("the client builds")
        };

        for _ in 0..2 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            proxy.hits.load(Ordering::SeqCst),
            0,
            "a loopback endpoint went through the system proxy, so the client secret \
             left the host in cleartext and the loopback exception protects nothing"
        );
        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            2,
            "and the events must still reach the collector directly, or the zero above \
             is a client that simply sent nothing"
        );
        proxy.stop().await;
        collector.stop().await;
    }

    /// **The transport refuses to be built for an endpoint the credential
    /// cannot safely cross**, even from inside the crate.
    ///
    /// `HttpOpenPanelTracker::new` is `pub(crate)` so that
    /// [`build`] — which takes a `&Decision`, and a `Decision::Report` is what
    /// `resolve` produces — is the only way to obtain a tracker. That closes the
    /// route from outside. This closes the route from *inside*: a future caller
    /// in this crate that reaches past `resolve` with
    /// `http://collector.internal/track` would otherwise get a tracker that
    /// posts the client secret across a network in cleartext, with
    /// `is_cleartext` politely turning off the proxy on the way.
    ///
    /// The assertion calls `config::is_secure_endpoint` rather than restating
    /// the rule, so there is one implementation of it and no second reader to
    /// drift.
    #[tokio::test]
    #[should_panic(expected = "the credential cannot safely cross")]
    async fn the_transport_refuses_an_endpoint_that_never_passed_resolve() {
        let _ = HttpOpenPanelTracker::new(
            "http://collector.internal/track",
            &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
            envelope(),
        );
    }

    /// The control: the same construction with a loopback endpoint must be
    /// accepted, or the test above would pass for a constructor that refused
    /// everything.
    #[tokio::test]
    async fn the_transport_accepts_an_endpoint_resolve_would_have_allowed() {
        for allowed in [
            "http://127.0.0.1:9/track",
            "https://collector.invalid/track",
        ] {
            assert!(
                HttpOpenPanelTracker::new(
                    allowed,
                    &crate::analytics::config::ClientCredentials::new(
                        TEST_CLIENT_ID,
                        TEST_CLIENT_SECRET
                    ),
                    envelope(),
                )
                .is_ok(),
                "{allowed} is one resolve would allow and must still build"
            );
        }
    }

    /// The control: a drain that **finishes** must report nothing lost.
    ///
    /// Without it, `lost_to_cancellation() >= 4` above would also pass for a
    /// guard that fired on every drain, which would turn a real signal into a
    /// line an operator learns to ignore — and this module's whole problem is
    /// notices nobody reads.
    #[tokio::test]
    async fn a_drain_that_finishes_reports_nothing_lost() {
        let collector = spawn_collector().await;
        let tracker = HttpOpenPanelTracker::new(
            &collector.url,
            &crate::analytics::config::ClientCredentials::new(TEST_CLIENT_ID, TEST_CLIENT_SECRET),
            envelope(),
        )
        .expect("the client builds");

        for _ in 0..3 {
            tracker.track(Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            });
        }
        tracker.flush().await;

        assert_eq!(
            collector.hits.load(Ordering::SeqCst),
            3,
            "the positive control: all three really did land"
        );
        assert_eq!(
            tracker.lost_to_cancellation(),
            0,
            "a drain that ran to completion must not report a lost tail"
        );
        collector.stop().await;
    }

    /// **The end-to-end check against a real OpenPanel instance.**
    ///
    /// `#[ignore]` because it needs a collector, a credential and a network,
    /// none of which CI has. Everything above proves this transport does what
    /// this repository believes OpenPanel wants; only this proves OpenPanel
    /// agrees. Every failure mode in this module is silent, so "the unit suite
    /// is green" and "events are landing" are genuinely different claims.
    ///
    /// ```text
    /// OPENCOMPANY_ANALYTICS_ENDPOINT=https://<host>/api/track \
    /// OPENCOMPANY_ANALYTICS_CLIENT_ID=<uuid> \
    /// OPENCOMPANY_ANALYTICS_CLIENT_SECRET=<secret> \
    ///   cargo test --features analytics -- --ignored --nocapture \
    ///   analytics::openpanel::test::a_real_collector_accepts_an_event
    /// ```
    ///
    /// Credentials come from the environment and are never written anywhere:
    /// not to a fixture, not to a log line, and not to this test's output,
    /// which prints only the `profileId` it sent and the ids the collector
    /// returned — enough to find the event in the dashboard and nothing more.
    ///
    /// It asserts a `2xx` **and** that the body names a `deviceId`, because a
    /// collector fronted by a proxy that swallows the request can answer `200`
    /// with something else entirely, and a status-only assertion would call
    /// that a pass.
    #[tokio::test]
    #[ignore = "needs a real OpenPanel instance and a credential from the environment"]
    async fn a_real_collector_accepts_an_event() {
        use crate::app::config::EnvSource;

        let os_env = crate::app::config::ProcessEnv;
        let endpoint = os_env
            .get(ENDPOINT_ENV)
            .expect("set OPENCOMPANY_ANALYTICS_ENDPOINT");
        let credentials = match resolve(
            Deployment::HostedTenant,
            &MapEnv::new([
                (
                    CLIENT_ID_ENV,
                    os_env.get(CLIENT_ID_ENV).expect("set the client id"),
                ),
                (
                    CLIENT_SECRET_ENV,
                    os_env
                        .get(CLIENT_SECRET_ENV)
                        .expect("set the client secret"),
                ),
                (ENDPOINT_ENV, endpoint.clone()),
            ]),
        ) {
            crate::analytics::config::Decision::Report { credentials, .. } => credentials,
            other => panic!("the environment does not resolve to reporting: {other:?}"),
        };

        // A run-specific id, so the event is findable and no real instance's
        // numbers are disturbed.
        let id = OpaqueId::instance(&format!("{:032x}", crate::ports::now_millis()));
        println!("posting as profileId {}", id.as_str());

        let body = crate::analytics::payload(
            &Envelope::new(id, Deployment::HostedTenant, Cognition::default()),
            &Event::InstanceStarted {
                companies: 1,
                storage: "fs",
                setup_complete: true,
            },
        );

        let response = reqwest::Client::builder()
            .default_headers(super::http::request_headers(&credentials))
            .build()
            .expect("a client")
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .expect("the collector is reachable");

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "the collector refused the event with {status}: {text}"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        assert!(
            parsed.get("deviceId").is_some(),
            "a 2xx with no deviceId is a proxy answering, not OpenPanel accepting: \
             {status} {text}"
        );
        println!("collector accepted it: {status} {text}");
    }

    /// **The header-safety check in `config` really is a subset of what a
    /// header value accepts.**
    ///
    /// `config::resolve` refuses a credential it judges unfit for a header, and
    /// it makes that judgement in the un-gated build, where `reqwest` may not
    /// even be in the dependency graph — so the rule is written by hand there
    /// and is deliberately *stricter* than `HeaderValue`. That is only safe
    /// while the subset claim holds: anything `resolve` accepts, the transport
    /// must be able to put on the wire. Nothing else in the tree would notice
    /// the day it stopped holding, so it is asserted here, in the one lane that
    /// has a `HeaderValue` to compare against.
    ///
    /// The reverse containment is deliberately **not** asserted: `HeaderValue`
    /// takes space, tab and the whole `0xA0..=0xFF` range, and refusing those is
    /// the point.
    #[test]
    fn the_header_safety_check_is_a_subset_of_what_a_header_accepts() {
        use crate::analytics::config::Decision;
        use reqwest::header::HeaderValue;

        // Every single byte, plus the multi-byte shapes a mangled secret
        // actually arrives in.
        let mut candidates: Vec<String> = (1u8..=255)
            .map(|byte| format!("ok{}", byte as char))
            .collect();
        candidates.extend(
            [
                "not-a-real-client-secret",
                "op_sk_9zQx-4Kd_7Yb2Lp0",
                "550e8400-e29b-41d4-a716-446655440000",
                "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo=",
                "wrapped\nsecret",
                "tab\tseparated",
                "spaced out",
                "caf\u{e9}-latte",
                "\u{4f8b}\u{3048}",
            ]
            .map(str::to_string),
        );

        let mut accepted = 0usize;
        for candidate in &candidates {
            let decision = resolve(
                Deployment::HostedTenant,
                &env(
                    "https://collector.invalid/track",
                    &[
                        (CLIENT_ID_ENV, candidate.as_str()),
                        (CLIENT_SECRET_ENV, candidate.as_str()),
                    ],
                ),
            );
            if matches!(decision, Decision::Report { .. }) {
                accepted += 1;
                assert!(
                    HeaderValue::from_str(candidate.trim()).is_ok(),
                    "`resolve` accepted {candidate:?}, which cannot go in a header — the \
                     subset claim in `config::is_header_safe` no longer holds"
                );
            }
        }

        // The control: the loop above would pass trivially if `resolve` had
        // started refusing everything.
        assert!(
            accepted > 50,
            "only {accepted} of {} candidates were accepted; the check has become so \
             strict that the subset assertion means nothing",
            candidates.len()
        );
    }
}
