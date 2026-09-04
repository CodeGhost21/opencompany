//! Crash and error reporting to a Sentry project the **operator** owns.
//!
//! `docs/spec/runtime/crash-reporting.md` is the contract; this is the wiring.
//! Nothing here reports anything until an operator sets
//! [`config::DSN_ENV`], and a build compiled without the `crash-reporting`
//! feature contains no client that could.
//!
//! # The compile-time domain gate
//!
//! The shape is the vendored runtime's, deliberately — same feature name,
//! same carve-out (`vendor/openhuman/Cargo.toml`, "TYPE CARVE-OUT"). Every
//! function in this module is compiled in **both** builds with the **same
//! signature**; only the bodies that name a `sentry::` type are gated. So:
//!
//! * [`tracing_layer`] returns a no-op [`Identity`] layer rather than nothing,
//!   and the subscriber that adds it needs no `#[cfg]`;
//! * [`scope::identify`] keeps its `tracing::debug!` and drops the scope call;
//! * [`init`] still resolves the decision, and reports it as
//!   [`config::Silence::NotCompiled`] when a DSN was configured but this binary
//!   has nothing to install — which is the line an operator needs, because
//!   "reporting to …" would be the opposite of the truth;
//! * `opencompany sentry-test` still exists and says which build it is, rather
//!   than looking like a subcommand that was never added.
//!
//! There is therefore no stub file and no `#[cfg]` at a call site, which is the
//! whole point: a gate that spreads into its callers gets one of them wrong.
//!
//! [`Identity`]: tracing_subscriber::layer::Identity

pub mod config;
pub mod redaction;

pub use config::{Decision, Dsn, Silence, Traces, release_tag};

use std::time::Duration;

use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// How long [`Guard::flush`] waits for queued events on the way out.
///
/// Two seconds, matching `analytics::shutdown::FLUSH_BUDGET` and chosen the
/// same way: the collector is a third party this process does not control, and
/// a drain that overruns Kubernetes' default 30s `terminationGracePeriodSeconds`
/// buys a `SIGKILL` in the middle of the shutdown those seconds exist to
/// protect. A dropped crash report costs one row in a dashboard; an overrun
/// costs a half-finished turn.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// The live client, held for the lifetime of the process.
///
/// Bind it to a **named** local, never to `_`: binding to `_` drops it
/// immediately, which closes the client while the process carries on running
/// and reports nothing for the rest of its life. (The same trap
/// `store::lock::acquire` documents, with a quieter symptom.)
#[cfg(feature = "crash-reporting")]
pub struct Guard(Option<sentry::ClientInitGuard>);

/// The same handle in a build with no client in it. Zero-sized, and every
/// method on it answers honestly rather than pretending.
#[cfg(not(feature = "crash-reporting"))]
pub struct Guard;

impl Guard {
    /// Whether a client is installed and reporting.
    #[cfg(feature = "crash-reporting")]
    pub fn is_active(&self) -> bool {
        self.0.as_ref().is_some_and(|guard| guard.is_enabled())
    }

    /// Whether a client is installed and reporting. Never, in this build.
    #[cfg(not(feature = "crash-reporting"))]
    pub fn is_active(&self) -> bool {
        false
    }

    /// Drains queued events, returning whether the queue emptied in time.
    ///
    /// `true` when there is nothing to drain, so a caller can treat the result
    /// as "reporting is in a good state" without asking first whether reporting
    /// is on at all.
    #[cfg(feature = "crash-reporting")]
    pub fn flush(&self, timeout: Duration) -> bool {
        match &self.0 {
            Some(guard) => guard.flush(Some(timeout)),
            None => true,
        }
    }

    /// Drains queued events. Nothing is queued in this build.
    #[cfg(not(feature = "crash-reporting"))]
    pub fn flush(&self, timeout: Duration) -> bool {
        let _ = timeout;
        true
    }
}

/// Resolves the decision and, when it says to report, installs the client.
///
/// Call this **first** in `main`, before the subscriber and before any other
/// work: the panic hook is installed here, so anything that panics earlier
/// panics unobserved. Keep the returned [`Guard`] alive for the whole process.
///
/// The [`Decision`] comes back so the caller can print one line saying what
/// this process will do — see [`Decision::describe`].
#[cfg(feature = "crash-reporting")]
pub fn init(deployment: Deployment, env: &dyn EnvSource) -> (Decision, Guard) {
    let decision = config::resolve(deployment, env);
    let Decision::Report {
        dsn,
        environment,
        release,
        traces,
    } = &decision
    else {
        return (decision, Guard(None));
    };

    let options = sentry::ClientOptions {
        // `parse` cannot fail: `config::parse_dsn` already accepted this string
        // through the same grammar. `ok()` rather than `expect` because a
        // panic inside crash reporting is the one panic nothing will report.
        dsn: dsn.expose().parse().ok(),
        release: Some(release.clone().into()),
        environment: Some(environment.clone().into()),
        // No IP address, no cookies, no request body. This crate never binds a
        // user to the scope either (see `scope`), so there is nothing for the
        // SDK to enrich even if this flipped.
        send_default_pii: false,
        // Report every error. There is no volume argument for crash reporting
        // the way there is for spans: an error that happens once is the one
        // worth seeing.
        sample_rate: 1.0,
        // Performance tracing, off unless an operator asked for a rate — see
        // `config::TRACES_SAMPLE_RATE_ENV` for why this repository will not
        // pick one for them. When it IS on, every transaction leaves through
        // `ScrubbingTransport` rather than `before_send`, because sentry 0.47
        // has no `before_send_transaction`; see `scrub_envelope`.
        traces_sample_rate: traces.rate(),
        // The SDK's own default, written down rather than inherited. This is
        // the length of the "what happened just before the crash" timeline —
        // at `RUST_LOG=info` a busy request can spend a hundred breadcrumbs
        // quickly, and the number that gets an incident diagnosed is a
        // deliberate choice, not an accident of the SDK's default changing
        // under a release.
        max_breadcrumbs: 100,
        // Attach a stack trace to `capture_message` events too, so a
        // `tracing::error!` says where it came from rather than only what it
        // said. This moves the message text into the last exception's `value`
        // as well — `sanitize` scrubs every one of the three places the text
        // can be, precisely so this flag is free to change.
        attach_stacktrace: true,
        before_send: Some(std::sync::Arc::new(sanitize)),
        // The second scrubbing seam, and the one that catches what
        // `before_send` structurally cannot. See `ScrubbingTransport`.
        transport: Some(std::sync::Arc::new(ScrubbingTransportFactory)),
        // Bounded on the way out; see `FLUSH_TIMEOUT`.
        shutdown_timeout: FLUSH_TIMEOUT,
        ..Default::default()
    };

    let guard = sentry::init(options);
    (decision, Guard(Some(guard)))
}

/// Resolves the decision. There is no client in this build to install.
///
/// A configured DSN is downgraded to [`Silence::NotCompiled`] rather than
/// reported as `Report`, because the line this feeds must say what the process
/// will **do**. `analytics.md` records the same lesson: a build without the
/// feature that announces "reporting to …" sends an operator looking in the
/// wrong place for the rest of the incident.
#[cfg(not(feature = "crash-reporting"))]
pub fn init(deployment: Deployment, env: &dyn EnvSource) -> (Decision, Guard) {
    let decision = match config::resolve(deployment, env) {
        Decision::Report { .. } => Decision::Silent(Silence::NotCompiled),
        silent => silent,
    };
    (decision, Guard)
}

/// The `tracing` bridge: `error!` becomes an event, `warn!`/`info!` become
/// breadcrumbs on the next one, everything else is ignored.
///
/// Add it to the subscriber **inside** the same `EnvFilter` the `fmt` layer is
/// under, not beside it. Filtering it separately would enable INFO-level call
/// sites process-wide so that breadcrumbs could be collected, which is a
/// standing cost paid by every install for a benefit only a reporting one
/// gets. The consequence is worth stating plainly and is in the doc: at this
/// crate's default filter (`error`) a report carries the error and **no
/// breadcrumbs**; `RUST_LOG=info` collects them.
#[cfg(feature = "crash-reporting")]
pub fn tracing_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use sentry::integrations::tracing::EventFilter;

    sentry::integrations::tracing::layer::<S>().event_filter(|metadata: &tracing::Metadata<'_>| {
        match *metadata.level() {
            tracing::Level::ERROR => EventFilter::Event,
            tracing::Level::WARN | tracing::Level::INFO => EventFilter::Breadcrumb,
            _ => EventFilter::Ignore,
        }
    })
}

/// The same seam with no Sentry in the build: a layer that does nothing.
///
/// [`tracing_subscriber::layer::Identity`] rather than an `Option` or a
/// `#[cfg]` at the call site, so `.with(observability::tracing_layer())` in
/// `src/bin/opencompany.rs` compiles unchanged in both builds. The two return
/// types never have to unify because both are `impl Layer<S>`.
#[cfg(not(feature = "crash-reporting"))]
pub fn tracing_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::layer::Identity::new()
}

/// Sends one deliberate event, to prove the pipeline end to end.
///
/// Returns the event id, or `None` when no client is installed — the caller
/// must treat that as a failure and say so, because a test command that prints
/// a plausible id while reporting is switched off is worse than one that
/// refuses. Backs `opencompany sentry-test`.
#[cfg(feature = "crash-reporting")]
pub fn capture_test_event(message: &str) -> Option<String> {
    sentry::Hub::current().client()?;
    sentry::configure_scope(|scope| {
        scope.set_tag("test", "true");
        scope.set_tag("source", "sentry-test");
    });
    Some(sentry::capture_message(message, sentry::Level::Error).to_string())
}

/// Sends one deliberate event. There is nothing in this build to send it.
#[cfg(not(feature = "crash-reporting"))]
pub fn capture_test_event(message: &str) -> Option<String> {
    let _ = message;
    None
}

/// Facts about this host, attached to every event from here on.
///
/// Deliberately **not** a user. Sentry's `user` is the field its UI counts
/// uniques on, and the only stable per-person identifier this crate holds is an
/// email address — which is exactly what `send_default_pii: false` and
/// `analytics.md`'s "what is never collected" say must not leave. A crash
/// report answers "which install, which build, what broke"; "who was signed in"
/// is a question for the host's own journal.
pub mod scope {
    /// Tags every subsequent event with the instance's opaque id and storage
    /// backend.
    ///
    /// Called after the host registers its companies, for the reason
    /// `analytics::boot::install` is: neither fact is known before that, and an
    /// event tagged with a default is worse than one tagged with nothing.
    ///
    /// `instance_id` is the random 128-bit id from `app::instance` — the same
    /// opaque identity analytics uses, and for the same reason. It names a
    /// host, not a customer.
    #[cfg(feature = "crash-reporting")]
    pub fn identify(instance_id: &str, storage: &str) {
        sentry::configure_scope(|scope| {
            scope.set_tag("instance_id", instance_id);
            scope.set_tag("storage", storage);
        });
        tracing::debug!(instance_id, storage, "crash reporting: scope identified");
    }

    /// The same seam with no Sentry in the build: the diagnostic line only.
    #[cfg(not(feature = "crash-reporting"))]
    pub fn identify(instance_id: &str, storage: &str) {
        tracing::debug!(instance_id, storage, "crash reporting: scope identified");
    }
}

// ---------------------------------------------------------------------------
// before_send
// ---------------------------------------------------------------------------

/// The `before_send` hook: what leaves this process, and what does not.
///
/// Three jobs, in order of how much they matter:
///
/// 1. **Scrub every string that can carry a credential.** Not just the message:
///    `sentry-tracing` puts the rendered text in `message`, `attach_stacktrace`
///    moves it into the last exception's `value`, and a structured
///    `tracing::error!(field = %value)` puts each field in `extra`. All three
///    are the same text from a leak's point of view, so all three are scrubbed
///    — along with `logentry`, breadcrumbs and tags. The vendored runtime
///    scrubs only `message` and the exception values, which is the gap this
///    closes.
/// 2. **Drop what identifies the machine or the person.** `server_name` is the
///    host's name; `user` and `request` are never populated by this crate and
///    are cleared anyway, so a future integration cannot start filling them
///    without this function being the thing that has to change.
/// 3. **Drop frame locals and source context.** Nothing in the Rust SDK
///    populates them today, so this is defence against a future that does
///    rather than a live control — but a captured local is a captured
///    credential, and it is one line to make sure.
///
/// Never returns `None`: this is a scrubber, not a filter. Deciding which
/// errors are worth seeing is the operator's to make in their own project,
/// where they can see what they are suppressing.
/// Runs the scrubber over a string in place, allocating only when it changed.
#[cfg(feature = "crash-reporting")]
fn scrub_in_place(text: &mut String) {
    if let std::borrow::Cow::Owned(scrubbed) = redaction::scrub(text) {
        *text = scrubbed;
    }
}

/// Runs the scrubber over every string leaf of a JSON value.
///
/// The structured half of a `tracing` event, a breadcrumb's `data` and a span's
/// `data` are all arbitrary JSON, so the pass has to recurse rather than look
/// at the top level.
#[cfg(feature = "crash-reporting")]
fn scrub_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => scrub_in_place(text),
        serde_json::Value::Array(items) => items.iter_mut().for_each(scrub_json),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(scrub_json),
        _ => {}
    }
}

#[cfg(feature = "crash-reporting")]
fn sanitize(
    mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
    use sentry::protocol::{Context, Stacktrace};

    fn strip_frames(stacktrace: &mut Stacktrace) {
        for frame in &mut stacktrace.frames {
            frame.vars.clear();
            frame.pre_context.clear();
            frame.context_line = None;
            frame.post_context.clear();
        }
    }

    // (2) identity
    event.server_name = None;
    event.user = None;
    event.request = None;

    // (1) text
    if let Some(message) = event.message.as_mut() {
        scrub_in_place(message);
    }
    if let Some(entry) = event.logentry.as_mut() {
        scrub_in_place(&mut entry.message);
        entry.params.iter_mut().for_each(scrub_json);
    }
    for exception in &mut event.exception.values {
        if let Some(value) = exception.value.as_mut() {
            scrub_in_place(value);
        }
        if let Some(mechanism) = exception.mechanism.as_mut() {
            mechanism.data.clear();
        }
        // (3)
        if let Some(stacktrace) = exception.stacktrace.as_mut() {
            strip_frames(stacktrace);
        }
        if let Some(stacktrace) = exception.raw_stacktrace.as_mut() {
            strip_frames(stacktrace);
        }
    }
    if let Some(stacktrace) = event.stacktrace.as_mut() {
        strip_frames(stacktrace);
    }
    for breadcrumb in &mut event.breadcrumbs.values {
        if let Some(message) = breadcrumb.message.as_mut() {
            scrub_in_place(message);
        }
        breadcrumb.data.values_mut().for_each(scrub_json);
    }
    event.extra.values_mut().for_each(scrub_json);
    for value in event.tags.values_mut() {
        scrub_in_place(value);
    }
    // The typed contexts (`os`, `runtime`, `device`, …) are platform facts the
    // SDK derives, not user data. `Other` is the escape hatch anything could
    // put anything into, so it goes through the same pass.
    for context in event.contexts.values_mut() {
        if let Context::Other(fields) = context {
            fields.values_mut().for_each(scrub_json);
        }
    }

    Some(event)
}

// ---------------------------------------------------------------------------
// Transactions: the seam `before_send` cannot reach
// ---------------------------------------------------------------------------

/// Wraps `router` so each served request becomes a Sentry transaction, and so
/// a `sentry-trace` header from the console is continued rather than starting a
/// second, unrelated trace.
///
/// # Why this is a function over a `Router` rather than a layer
///
/// A tower `Layer` cannot be returned as `impl Layer` from two `#[cfg]`
/// branches without also naming every bound `Router::layer` puts on the
/// service it produces. Taking and returning the `Router` keeps all of that
/// inside one function, so the call site in `server::routes` needs no `#[cfg]`
/// — the same rule the rest of this module follows.
///
/// # Why it checks first
///
/// The layers are added **only** when this process is actually recording
/// transactions. With tracing off — the default, and what every install that
/// sets nothing but a DSN gets — the router is returned untouched, so a feature
/// nobody asked for costs nothing on the request path. That check has to happen
/// here rather than at `init`, because the router is built later; it reads the
/// live client's options, which is the same thing the sampler will read.
#[cfg(feature = "crash-reporting")]
pub fn instrument_http(router: axum::Router) -> axum::Router {
    use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};

    if !transactions_are_on() {
        return router;
    }
    router
        // Inner: starts the transaction, names it from the matched route
        // (`/api/v1/companies/{company}` rather than one transaction per
        // company id), continues an incoming `sentry-trace`, and sets the
        // status from the response code. `enable_pii` is deliberately NOT
        // called — that is what would attach the client IP.
        .layer(SentryHttpLayer::new().enable_transaction())
        // Outer: a Hub per request, so one request's scope, tags and
        // breadcrumbs cannot leak into a concurrent request's report. On a
        // server this is not optional.
        .layer(NewSentryLayer::<axum::extract::Request>::new_from_top())
}

/// The same seam with no Sentry in the build: the router, unchanged.
#[cfg(not(feature = "crash-reporting"))]
pub fn instrument_http(router: axum::Router) -> axum::Router {
    router
}

/// Whether the installed client is recording transactions.
#[cfg(feature = "crash-reporting")]
fn transactions_are_on() -> bool {
    sentry::Hub::current()
        .client()
        .is_some_and(|client| client.options().traces_sample_rate > 0.0)
}

/// Reduces a request to the parts a report may carry: the method, and the URL
/// with its userinfo, query string and fragment removed.
///
/// The query string is the point. A magic-link sign-in is `?code=<43 chars>`,
/// and `sentry-tower` attaches the request URL to the transaction it starts.
/// Its own `scrub_pii_from_url` removes userinfo only, so the code would
/// survive — and a sign-in code in a transaction is a sign-in code in whoever
/// can read the operator's Sentry project.
///
/// Headers, cookies and body are dropped rather than scrubbed: none of them
/// answers a question a transaction is read to answer.
#[cfg(feature = "crash-reporting")]
fn narrow_request(request: sentry::protocol::Request) -> sentry::protocol::Request {
    let url = request.url.map(|mut url| {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        url
    });
    sentry::protocol::Request {
        method: request.method,
        url,
        ..Default::default()
    }
}

/// The transaction equivalent of [`sanitize`].
///
/// # Why this exists at all
///
/// `sentry` 0.47 has **no `before_send_transaction`**. `before_send` is applied
/// in `Client::prepare_event`, and a transaction never goes through it:
/// `Transaction::finish` builds an envelope and hands it straight to
/// `Client::send_envelope`. So the moment `traces_sample_rate` is non-zero,
/// every transaction would leave this process without passing any of the
/// scrubbing this module exists for — which would quietly undo the property the
/// rest of the file is built around.
///
/// Checked against the pinned version rather than assumed; a later `sentry` may
/// grow the hook, at which point this can move into it unchanged.
#[cfg(feature = "crash-reporting")]
fn sanitize_transaction(
    mut transaction: sentry::protocol::Transaction<'static>,
) -> sentry::protocol::Transaction<'static> {
    use sentry::protocol::Context;

    // Identity, on the same terms as `sanitize`.
    transaction.server_name = None;
    transaction.user = None;
    transaction.request = transaction.request.take().map(narrow_request);

    // The name is the matched route when `sentry-tower` set it, which is safe
    // by construction — but a transaction started anywhere else may be named
    // from something less careful.
    if let Some(name) = transaction.name.as_mut() {
        scrub_in_place(name);
    }
    for span in &mut transaction.spans {
        if let Some(description) = span.description.as_mut() {
            scrub_in_place(description);
        }
        span.data.values_mut().for_each(scrub_json);
    }
    transaction.extra.values_mut().for_each(scrub_json);
    for value in transaction.tags.values_mut() {
        scrub_in_place(value);
    }
    for context in transaction.contexts.values_mut() {
        if let Context::Other(fields) = context {
            fields.values_mut().for_each(scrub_json);
        }
    }
    transaction
}

/// Scrubs the items of an outgoing envelope that no callback covers.
///
/// Returns the envelope **untouched** when it carries no transaction, which is
/// every envelope on an install with tracing off and also the only safe answer
/// for a raw envelope: `Envelope::into_items` yields nothing for one, so
/// rebuilding indiscriminately would silently empty it.
#[cfg(feature = "crash-reporting")]
fn scrub_envelope(envelope: sentry::Envelope) -> sentry::Envelope {
    use sentry::protocol::EnvelopeItem;

    if !envelope
        .items()
        .any(|item| matches!(item, EnvelopeItem::Transaction(_)))
    {
        return envelope;
    }
    let mut scrubbed = sentry::Envelope::new().with_headers(envelope.headers().clone());
    for item in envelope.into_items() {
        match item {
            EnvelopeItem::Transaction(transaction) => {
                scrubbed.add_item(sanitize_transaction(transaction));
            }
            other => scrubbed.add_item(other),
        }
    }
    scrubbed
}

/// The default transport, with [`scrub_envelope`] in front of it.
///
/// The last seam before bytes leave the process, and therefore the one place a
/// guarantee can be made about **every** envelope kind rather than about the
/// two the SDK happens to offer a callback for today. A future SDK that starts
/// emitting a new item type gets scrubbed by whatever this function learns,
/// instead of shipping unexamined because nobody noticed a new hook.
#[cfg(feature = "crash-reporting")]
struct ScrubbingTransportFactory;

#[cfg(feature = "crash-reporting")]
impl sentry::TransportFactory for ScrubbingTransportFactory {
    fn create_transport(
        &self,
        options: &sentry::ClientOptions,
    ) -> std::sync::Arc<dyn sentry::Transport> {
        std::sync::Arc::new(ScrubbingTransport {
            // The transport the `ureq` + `rustls` features selected. Wrapped,
            // never replaced: envelope framing, rate-limit handling and the
            // background worker stay the SDK's problem.
            inner: sentry::transports::DefaultTransportFactory.create_transport(options),
        })
    }
}

#[cfg(feature = "crash-reporting")]
struct ScrubbingTransport {
    inner: std::sync::Arc<dyn sentry::Transport>,
}

#[cfg(feature = "crash-reporting")]
impl sentry::Transport for ScrubbingTransport {
    fn send_envelope(&self, envelope: sentry::Envelope) {
        self.inner.send_envelope(scrub_envelope(envelope));
    }

    fn flush(&self, timeout: Duration) -> bool {
        self.inner.flush(timeout)
    }

    fn shutdown(&self, timeout: Duration) -> bool {
        self.inner.shutdown(timeout)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    const DSN: &str = "https://examplePublicKey@o0.ingest.sentry.io/0";

    #[test]
    fn an_install_that_configures_nothing_installs_no_client() {
        let (decision, guard) = init(Deployment::SelfHosted, &MapEnv::default());
        assert_eq!(decision, Decision::Silent(Silence::NoDsn));
        assert!(!guard.is_active());
        // Flushing a guard that holds nothing is a success, so a caller need
        // not ask whether reporting is on before draining.
        assert!(guard.flush(FLUSH_TIMEOUT));
    }

    #[test]
    fn opting_out_installs_no_client_even_with_a_dsn() {
        let (decision, guard) = init(
            Deployment::HostedTenant,
            &MapEnv::new([(config::DSN_ENV, DSN), (config::ENABLE_ENV, "off")]),
        );
        assert_eq!(decision, Decision::Silent(Silence::OptedOut));
        assert!(!guard.is_active());
    }

    /// The default build's answer to a configured DSN. `Report` here would be
    /// a line that says the opposite of what the process does.
    #[cfg(not(feature = "crash-reporting"))]
    #[test]
    fn a_build_without_the_feature_says_so() {
        let (decision, guard) = init(
            Deployment::HostedTenant,
            &MapEnv::new([(config::DSN_ENV, DSN)]),
        );
        assert_eq!(decision, Decision::Silent(Silence::NotCompiled));
        assert!(!guard.is_active());
        assert!(decision.describe().contains("crash-reporting"));
        // The seam still exists, and still refuses honestly.
        assert_eq!(capture_test_event("ping"), None);
    }

    #[cfg(feature = "crash-reporting")]
    mod gated {
        use super::*;
        use crate::observability::redaction::credential_shaped;
        use sentry::protocol::{Breadcrumb, Event, Exception, LogEntry, Value};

        /// One event carrying a credential in every place an event can carry
        /// one. If a new field is added to the protocol that can hold text,
        /// this is where it should be added.
        fn hostile_event() -> Event<'static> {
            let mut event = Event {
                message: Some("refresh failed: api_key=hunter2".into()),
                logentry: Some(LogEntry {
                    message: "posting to https://user:hunter2@collector.internal/1".into(),
                    params: vec![Value::String("Authorization: Bearer hunter2".into())],
                }),
                server_name: Some("build-host-42".into()),
                ..Default::default()
            };
            event.exception.values.push(Exception {
                ty: "Error".into(),
                value: Some(format!("token: {} rejected", credential_shaped("ghp_", 36))),
                ..Default::default()
            });
            event.breadcrumbs.values.push(Breadcrumb {
                message: Some(format!("using {}", credential_shaped("sk-proj-", 20))),
                data: [(
                    "url".to_string(),
                    Value::String("https://u:hunter2@h/1".into()),
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            });
            event
                .extra
                .insert("detail".into(), Value::String("password=hunter2".into()));
            event.extra.insert(
                "nested".into(),
                Value::Array(vec![Value::String("client_secret=hunter2".into())]),
            );
            event
                .tags
                .insert("origin".into(), credential_shaped("th_live_", 20));
            event
        }

        #[test]
        fn no_credential_survives_the_hook() {
            let sanitized = sanitize(hostile_event()).expect("the hook never drops an event");
            let rendered = serde_json::to_string(&sanitized).expect("an event serializes");
            for leaked in [
                "hunter2",
                "ghp_AAAA",
                "sk-proj-AAAA",
                "th_live_AAAA",
                "build-host-42",
            ] {
                assert!(!rendered.contains(leaked), "{leaked} survived: {rendered}");
            }
            // And the diagnostic around each one survived, or the report is
            // useless and an operator turns this off.
            assert!(rendered.contains("refresh failed"), "{rendered}");
            assert!(rendered.contains("rejected"), "{rendered}");
            assert!(rendered.contains("collector.internal"), "{rendered}");
        }

        #[test]
        fn the_hook_never_drops_an_event() {
            // Deciding which errors are worth seeing belongs in the operator's
            // own project, where they can see what they suppressed.
            assert!(sanitize(Event::default()).is_some());
        }

        #[test]
        fn frame_locals_and_source_context_are_stripped() {
            use sentry::protocol::{Frame, Stacktrace};

            let mut event = Event::default();
            event.exception.values.push(Exception {
                stacktrace: Some(Stacktrace {
                    frames: vec![Frame {
                        context_line: Some("let key = \"hunter2\";".into()),
                        pre_context: vec!["fn connect() {".into()],
                        post_context: vec!["}".into()],
                        vars: [("key".to_string(), Value::String("hunter2".into()))]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            });

            let sanitized = sanitize(event).expect("the hook never drops an event");
            let frame = &sanitized.exception.values[0]
                .stacktrace
                .as_ref()
                .expect("the stacktrace survives")
                .frames[0];
            assert!(frame.vars.is_empty());
            assert!(frame.pre_context.is_empty());
            assert!(frame.post_context.is_empty());
            assert_eq!(frame.context_line, None);
        }

        /// One transaction carrying something it must not send in every field
        /// a transaction has.
        fn hostile_transaction() -> sentry::protocol::Transaction<'static> {
            use sentry::protocol::Span;

            let mut transaction = sentry::protocol::Transaction {
                name: Some("GET /api/v1/companies/{company}".into()),
                server_name: Some("build-host-42".into()),
                user: Some(sentry::protocol::User {
                    email: Some("operator@example.com".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            transaction.request = Some(sentry::protocol::Request {
                method: Some("GET".into()),
                url: Some(
                    "https://key:hunter2@host/api/v1/companies/acme?code=magic-link-code#frag"
                        .parse()
                        .expect("a url"),
                ),
                headers: [("Cookie".to_string(), "oc_session=hunter2".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            });
            transaction.spans.push(Span {
                op: Some("http.client".into()),
                description: Some("POST https://u:hunter2@provider/v1?api_key=hunter2".into()),
                data: [(
                    "http.url".to_string(),
                    Value::String(format!("https://provider/v1?token={}", "hunter2")),
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            });
            transaction
                .tags
                .insert("origin".into(), credential_shaped("th_live_", 20));
            transaction
                .extra
                .insert("detail".into(), Value::String("password=hunter2".into()));
            transaction
        }

        #[test]
        fn no_credential_survives_a_transaction() {
            // `before_send` does not run on transactions — sentry 0.47 has no
            // `before_send_transaction`, and `Transaction::finish` posts an
            // envelope straight to the transport. This is the seam that closes
            // that, so it is tested on the same terms as the event hook.
            let sanitized = sanitize_transaction(hostile_transaction());
            let rendered = serde_json::to_string(&sanitized).expect("a transaction serializes");
            for leaked in [
                "hunter2",
                "magic-link-code",
                "build-host-42",
                "operator@example.com",
                "th_live_AAAA",
            ] {
                assert!(!rendered.contains(leaked), "{leaked} survived: {rendered}");
            }
        }

        #[test]
        fn a_transaction_keeps_what_makes_it_readable() {
            let sanitized = sanitize_transaction(hostile_transaction());
            // The route template is the whole point of a transaction name.
            assert_eq!(
                sanitized.name.as_deref(),
                Some("GET /api/v1/companies/{company}")
            );
            let request = sanitized.request.expect("the request survives, narrowed");
            assert_eq!(request.method.as_deref(), Some("GET"));
            assert_eq!(
                request.url.map(|url| url.to_string()),
                Some("https://host/api/v1/companies/acme".to_string())
            );
            // Headers and cookies are dropped rather than scrubbed: neither
            // answers a question a transaction is read to answer.
            assert!(request.headers.is_empty());
            assert_eq!(sanitized.spans[0].op.as_deref(), Some("http.client"));
            assert!(
                sanitized.spans[0]
                    .description
                    .as_deref()
                    .is_some_and(|text| text.contains("provider")),
                "{:?}",
                sanitized.spans[0].description
            );
        }

        #[test]
        fn the_transport_scrubs_a_transaction_envelope() {
            // The guarantee is made at the transport, so it is tested there:
            // an envelope in, an envelope out, with nothing in between that the
            // SDK could route around.
            let mut envelope = sentry::Envelope::new();
            envelope.add_item(hostile_transaction());
            let scrubbed = scrub_envelope(envelope);
            let mut rendered = Vec::new();
            scrubbed
                .to_writer(&mut rendered)
                .expect("an envelope serializes");
            let rendered = String::from_utf8(rendered).expect("utf-8");
            assert!(!rendered.contains("hunter2"), "{rendered}");
            assert!(!rendered.contains("magic-link-code"), "{rendered}");
        }

        #[test]
        fn an_envelope_with_no_transaction_is_passed_through_untouched() {
            // Rebuilding indiscriminately would empty a RAW envelope, because
            // `into_items` yields nothing for one.
            let mut envelope = sentry::Envelope::new();
            envelope.add_item(Event {
                message: Some("api_key=hunter2".into()),
                ..Default::default()
            });
            let before = format!("{envelope:?}");
            assert_eq!(format!("{:?}", scrub_envelope(envelope)), before);
        }

        #[test]
        fn identity_fields_are_cleared() {
            let mut event = Event {
                server_name: Some("build-host-42".into()),
                user: Some(sentry::protocol::User {
                    email: Some("operator@example.com".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            event.request = Some(sentry::protocol::Request {
                url: Some("https://host/api/v1/companies/acme".parse().expect("a url")),
                ..Default::default()
            });

            let sanitized = sanitize(event).expect("the hook never drops an event");
            assert_eq!(sanitized.server_name, None);
            assert!(sanitized.user.is_none());
            assert!(sanitized.request.is_none());
        }
    }
}
