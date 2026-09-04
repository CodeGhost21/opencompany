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

pub use config::{Decision, Dsn, Silence, release_tag};

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
        // No performance tracing. Spans carry route templates, argument values
        // and timings for every request, which is a far larger surface than
        // errors and is not what this feature is for.
        traces_sample_rate: 0.0,
        // Attach a stack trace to `capture_message` events too, so a
        // `tracing::error!` says where it came from rather than only what it
        // said. This moves the message text into the last exception's `value`
        // as well — `sanitize` scrubs every one of the three places the text
        // can be, precisely so this flag is free to change.
        attach_stacktrace: true,
        before_send: Some(std::sync::Arc::new(sanitize)),
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
#[cfg(feature = "crash-reporting")]
fn sanitize(
    mut event: sentry::protocol::Event<'static>,
) -> Option<sentry::protocol::Event<'static>> {
    use sentry::protocol::{Context, Stacktrace};

    fn scrub_in_place(text: &mut String) {
        if let std::borrow::Cow::Owned(scrubbed) = redaction::scrub(text) {
            *text = scrubbed;
        }
    }

    fn scrub_json(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => scrub_in_place(text),
            serde_json::Value::Array(items) => items.iter_mut().for_each(scrub_json),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(scrub_json),
            _ => {}
        }
    }

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
                value: Some(format!("token: {} rejected", credential_shaped("ghp_", 36)).into()),
                ..Default::default()
            });
            event.breadcrumbs.values.push(Breadcrumb {
                message: Some(format!("using {}", credential_shaped("sk-proj-", 20)).into()),
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
