//! The enable/disable decision: whether this process reports crashes at all,
//! where to, and under which release.
//!
//! Kept apart from the client for the reason `analytics::config` is: it is the
//! part that has to be *provably* right, and it can be. It is pure — an
//! [`EnvSource`] and a [`Deployment`] in, a [`Decision`] out — so every branch
//! of it is exercised by the default `cargo test`, with no network, no
//! `sentry::` type in scope, and no `crash-reporting` feature.

use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// The Sentry DSN. **Configuration, never a compiled-in constant.**
///
/// A DSN baked into a GPL-3.0 binary is a DSN every reader of the source can
/// post to, and the cost of that is not abstract: an ingest endpoint anyone can
/// write to is somebody's quota. It also decides *which organisation* an
/// install's crashes go to, which is not a decision this repository is entitled
/// to make on an operator's behalf.
pub const DSN_ENV: &str = "OPENCOMPANY_SENTRY_DSN";

/// Operator override: `off` forbids reporting and outranks everything else.
///
/// A DSN is already the switch — unset means silence — so this exists for the
/// case where the DSN is injected by something the operator does not edit (a
/// container manager, a shared `.env`) and they want it off anyway. `on` is
/// accepted and means nothing beyond "not off": there is nothing to force,
/// because without a DSN there is nowhere to send.
pub const ENABLE_ENV: &str = "OPENCOMPANY_SENTRY";

/// Overrides the `environment` tag. Defaults to the deployment kind.
pub const ENVIRONMENT_ENV: &str = "OPENCOMPANY_SENTRY_ENVIRONMENT";

/// The fraction of requests recorded as performance transactions, `0.0` to
/// `1.0`. **Absent means `0.0`**, and that is the point.
///
/// Errors and transactions are billed separately by Sentry, and a transaction
/// is emitted for every request rather than only when something goes wrong — so
/// a rate this repository chose on an operator's behalf would be a recurring
/// bill they did not ask for. The same argument [`DSN_ENV`] makes about whose
/// quota this is, one level down: having decided to report at all is not the
/// same as having decided to report *every request*.
///
/// It is also a much larger content surface than an error. A transaction
/// carries a span per outbound request, each with a URL — which is why
/// [`super::sanitize_transaction`] exists and why turning this on without it
/// would undo the care in `observability::redaction`.
pub const TRACES_SAMPLE_RATE_ENV: &str = "OPENCOMPANY_SENTRY_TRACES_SAMPLE_RATE";

/// A Sentry DSN.
///
/// A newtype rather than a bare `String`, on the `analytics::ProjectToken`
/// precedent and for the same reason: it must never be printed, logged or
/// serialized by accident. It derives **neither** `Debug` nor `Serialize` — the
/// hand-written `Debug` redacts — because `serde_json::to_value(&some_config)`
/// is exactly how a credential reaches a payload.
///
/// A DSN's public key is not a password (it ships in every browser bundle that
/// reports to the same project), but it is not nothing either: it authorizes
/// writes to somebody's quota, and treating it as printable is how it ends up
/// in a screenshot of a boot log. [`Dsn::loggable`] is the only shape that is
/// allowed out.
#[derive(Clone, PartialEq, Eq)]
pub struct Dsn(String);

impl Dsn {
    /// Wraps a DSN that [`parse_dsn`] has already accepted.
    fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The DSN, for the one caller that hands it to the client.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// `scheme://host/path` — the destination, with the public key removed.
    ///
    /// This is what a boot line is allowed to name. Naming nothing would be
    /// worse than naming a redacted form: "reporting" with no destination is
    /// unactionable when an operator has two projects and events are landing in
    /// the wrong one.
    pub fn loggable(&self) -> String {
        match url::Url::parse(&self.0) {
            Ok(mut url) => {
                // Both halves, in the order `analytics::boot::loggable_endpoint`
                // learned to do it: userinfo carries the public key here, and a
                // query string is where anyone fronting an ingest with their own
                // proxy puts a key of their own.
                let _ = url.set_username("");
                let _ = url.set_password(None);
                url.set_query(None);
                url.set_fragment(None);
                url.to_string()
            }
            // Unreachable: `parse_dsn` built this through the same parser. A
            // constant rather than the raw value, because the one thing this
            // function may never do is fall back to printing the DSN.
            Err(_) => "<unprintable dsn>".to_string(),
        }
    }
}

impl std::fmt::Debug for Dsn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Dsn(<redacted>)")
    }
}

/// Why a process is not reporting. Printed once at boot, so an operator who
/// *expected* crash reports can tell "switched off" from "misconfigured".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    /// The operator set `OPENCOMPANY_SENTRY=off`.
    OptedOut,
    /// No DSN is configured. **The default**, and the only state a fresh
    /// install or a CI runner is ever in.
    NoDsn,
    /// `OPENCOMPANY_SENTRY` was set to something this does not recognise.
    ///
    /// A separate reason from [`Self::OptedOut`] on purpose, on the lesson
    /// `analytics::config::Silence::Unreadable` records: an operator who typed
    /// `of` gets the outcome they meant *and* a line saying their value was not
    /// understood, rather than silence they cannot tell from a working opt-out.
    Unreadable,
    /// `OPENCOMPANY_SENTRY_DSN` is set to something that is not a Sentry DSN —
    /// no scheme, a scheme that is not `http`/`https`, no public key, no host,
    /// no project id, or bytes this process cannot read.
    ///
    /// Silence rather than a client that cannot send, because the failure this
    /// prevents is the one analytics was rebuilt to prevent: boot prints
    /// "reporting to …", a client is installed, and every envelope dies inside
    /// the transport behind a log line nobody has enabled. The operator reading
    /// their own logs has no reason to look again.
    ///
    /// The reason never quotes the value — see [`Dsn`].
    UnusableDsn,
    /// A DSN was configured and accepted, but this binary was compiled without
    /// the `crash-reporting` feature, so there is no client in it to install.
    ///
    /// The line reports what the process will **do**, not what was configured.
    /// Saying "reporting to …" here would be the exact opposite of the truth.
    NotCompiled,
}

impl Silence {
    /// The stable reason slug, for the boot line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptedOut => "operator opted out",
            Self::NoDsn => "no DSN is configured",
            Self::Unreadable => "the OPENCOMPANY_SENTRY value is not recognised",
            Self::UnusableDsn => "the OPENCOMPANY_SENTRY_DSN value is not a usable Sentry DSN",
            Self::NotCompiled => {
                "a DSN was configured, but this build was compiled without the `crash-reporting` feature"
            }
        }
    }
}

/// What this process will do about **performance tracing**, which is a
/// separate decision from whether it reports errors.
///
/// Separate because the costs are different in kind. An error event is rare and
/// is the thing an operator asked for; a transaction is emitted for every
/// served request whether or not anything went wrong, is billed on its own
/// quota, and carries a span — with a URL — for every outbound call the request
/// made. An operator who wants crash reports has not thereby asked for a
/// per-request feed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Traces {
    /// Record no transactions. **The default**, and what every install that
    /// sets only a DSN gets.
    Off,
    /// Record this fraction of requests. Always in `0.0 < rate <= 1.0`.
    Sampled(f32),
    /// [`TRACES_SAMPLE_RATE_ENV`] was set to something that is not a fraction
    /// between 0 and 1.
    ///
    /// A distinct state rather than a silent fall back to [`Self::Off`], on the
    /// lesson [`Silence::Unreadable`] records: an operator who typed `0,5` or
    /// `50%` gets the safe outcome *and* a line saying their value was not
    /// understood, instead of silence they cannot tell from a working default.
    Unreadable,
}

impl Traces {
    /// The rate to hand the client. Zero unless a rate was configured and read.
    pub fn rate(self) -> f32 {
        match self {
            Self::Sampled(rate) => rate,
            Self::Off | Self::Unreadable => 0.0,
        }
    }

    /// Whether any transaction will be recorded.
    pub fn is_on(self) -> bool {
        self.rate() > 0.0
    }

    /// The clause the boot line adds after the destination.
    fn describe(self) -> String {
        match self {
            Self::Off => "performance tracing off".to_string(),
            Self::Sampled(rate) => format!("tracing {}% of requests", rate * 100.0),
            Self::Unreadable => format!(
                "performance tracing off ({TRACES_SAMPLE_RATE_ENV} is not a number between 0 and 1)"
            ),
        }
    }
}

/// What this process will do about crash reporting.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Report nothing. No client is constructed, so nothing *can* be reported.
    Silent(Silence),
    /// Report to `dsn`, tagged with `environment` and `release`.
    Report {
        /// The operator's ingest endpoint.
        dsn: Dsn,
        /// The `environment` tag: the deployment kind unless overridden.
        environment: String,
        /// The `release` tag: `opencompany@<version>+<commit>`.
        release: String,
        /// Whether, and how much, to record performance transactions.
        traces: Traces,
    },
}

impl Decision {
    /// One line naming what the process will do and, when it will do nothing,
    /// why. Mirrors `analytics::boot::describe` — a `println!` rather than a
    /// `tracing::info!` for the same reason it is there: the CLI's default
    /// `EnvFilter` is `error`, so an `info!` would be exactly as silent as the
    /// misconfiguration it reports.
    pub fn describe(&self) -> String {
        match self {
            Self::Silent(reason) => format!("crash reporting: off ({})", reason.as_str()),
            Self::Report {
                dsn,
                environment,
                release,
                traces,
            } => format!(
                "crash reporting: reporting to {} as {release} ({environment}), {}",
                dsn.loggable(),
                traces.describe()
            ),
        }
    }
}

/// The canonical release tag: `opencompany@<version>[+<commit>]`.
///
/// Both halves are needed. [`crate::VERSION`] has read `0.1.0` for thousands of
/// commits, so a release built from it alone cannot tell two builds apart —
/// which is the whole question a stack trace raises. [`crate::BUILD_COMMIT`]
/// already resolves an explicit `OPENCOMPANY_BUILD_COMMIT`, then `git`, then
/// `GITHUB_SHA`, then the literal `"unknown"` (`src/build_stamp.rs`), so no
/// second environment variable is invented here for something the build already
/// stamps.
///
/// The `"unknown"` case is dropped rather than appended: `opencompany@0.1.0` is
/// an honest "this build cannot say which commit it is", and
/// `opencompany@0.1.0+unknown` is a release name that looks like a commit and
/// is not one. A `-dirty` suffix is kept — a build from a modified tree is a
/// different build, and the tag should say so.
pub fn release_tag() -> String {
    release_tag_from(crate::VERSION, crate::BUILD_COMMIT)
}

/// [`release_tag`] with its inputs supplied, so the `unknown` and `-dirty`
/// branches are reachable from a test rather than only from a build.
fn release_tag_from(version: &str, commit: &str) -> String {
    let commit = commit.trim();
    if commit.is_empty() || commit == "unknown" {
        format!("opencompany@{version}")
    } else {
        format!("opencompany@{version}+{commit}")
    }
}

/// Parses a candidate DSN, or `None` when it is not one.
///
/// Validated with `url`, the same parser the transport will hand it to, on the
/// rule issue #673 settled for a different call site: a second, hand-rolled
/// reader of a URL grammar is a bypass waiting to be found. On top of the parse
/// this asserts the four things that make a URL a *Sentry* DSN, because a URL
/// that parses and is not a DSN is precisely the input that resolves to
/// "reporting" and then never delivers:
///
/// 1. an `http`/`https` scheme — nothing else can be posted to;
/// 2. a non-empty username, which is the public key;
/// 3. a host;
/// 4. a path whose last segment is a project id.
///
/// A **password** is refused rather than stripped. The `https://key:secret@…`
/// form is a DSN from before 2016 whose secret half is no longer accepted by
/// any ingest, so a DSN carrying one is either a stale copy — silence with a
/// reason beats a client that 401s forever — or an operator who pasted a
/// credential into the wrong variable, which is worth refusing loudly.
fn parse_dsn(raw: &str) -> Option<Dsn> {
    let raw = raw.trim();
    let url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if url.host_str().is_none_or(str::is_empty) {
        return None;
    }
    let project = url.path().rsplit('/').next().unwrap_or_default();
    if project.is_empty() {
        return None;
    }
    Some(Dsn::new(raw))
}

/// [`parse_dsn`], reachable from the gated tests in [`super`].
///
/// Those tests pin this grammar against the SDK's own parser
/// (`the_two_dsn_parsers_agree`), which needs to call it from a module that
/// has `sentry::` in scope — and this module deliberately does not.
//
// Gated on the feature as well as on `cfg(test)`: the only caller is the
// `the_two_dsn_parsers_agree` test, which needs `sentry::` in scope, so in a
// default build this would be an unused function and `-D dead-code` is on.
#[cfg(all(test, feature = "crash-reporting"))]
pub(crate) fn parse_dsn_for_test(raw: &str) -> Option<Dsn> {
    parse_dsn(raw)
}

/// Resolves this process's crash-reporting decision from the environment.
///
/// The order is the order of the switches' authority, and only the first
/// applicable one is consulted:
///
/// 1. `OPENCOMPANY_SENTRY=off` — silence, whatever else is set;
/// 2. an `OPENCOMPANY_SENTRY` value that is neither `on` nor `off` — silence,
///    naming the value as unreadable rather than guessing at it;
/// 3. no DSN — silence, and this is the default every install starts in;
/// 4. a DSN that is not usable — silence, naming that;
/// 5. otherwise, report.
///
/// Unlike `analytics::config::resolve`, the deployment kind does not gate
/// anything. It only names the `environment` tag. Analytics reports to a
/// collector *this* project runs, so who is allowed to report is the whole
/// question there; a crash report goes to an endpoint the operator configured
/// in their own organisation, and a self-hoster who sets a DSN has asked for
/// exactly one thing and should get it.
pub fn resolve(deployment: Deployment, env: &dyn EnvSource) -> Decision {
    // `get_os`, not `get`, on the lesson `Deployment::from_env` records: `get`
    // maps a non-Unicode value to `None`, which here would read as "nobody set
    // the switch" and fall through to reporting — telemetry turned ON by a
    // malformed variable, on the one switch that exists to turn it off. A
    // BLANK value is still absent, so a launcher that exports an empty
    // variable changes nothing.
    match env.get_os(ENABLE_ENV) {
        None => {}
        Some(raw) => match raw.to_str().map(|value| value.trim().to_ascii_lowercase()) {
            Some(value) if value.is_empty() => {}
            Some(value) if value == "off" => return Decision::Silent(Silence::OptedOut),
            Some(value) if value == "on" => {}
            _ => return Decision::Silent(Silence::Unreadable),
        },
    }

    let Some(raw) = env.get_os(DSN_ENV) else {
        return Decision::Silent(Silence::NoDsn);
    };
    let Some(raw) = raw.to_str() else {
        // Bytes this process cannot read are a *misconfigured* DSN, not an
        // absent one, and the two want different lines.
        return Decision::Silent(Silence::UnusableDsn);
    };
    if raw.trim().is_empty() {
        return Decision::Silent(Silence::NoDsn);
    }
    let Some(dsn) = parse_dsn(raw) else {
        return Decision::Silent(Silence::UnusableDsn);
    };

    Decision::Report {
        dsn,
        environment: environment(deployment, env),
        release: release_tag(),
        traces: traces(env),
    }
}

/// The performance-tracing decision.
///
/// Absent, blank or unreadable bytes mean [`Traces::Off`] — the same "a
/// variable nobody set changes nothing" rule the enable switch follows, and for
/// the stronger reason that this one costs money. A value that parses but is
/// outside `0.0..=1.0` is [`Traces::Unreadable`] rather than clamped: `100` is
/// far more likely to mean "100%" than "1.0", and silently reading it as
/// `1.0`-after-clamping would record every request for an operator who thought
/// they had asked for something else.
///
/// An explicit `0` is [`Traces::Off`] rather than `Sampled(0.0)`, so the boot
/// line reads the same as it does for an operator who set nothing — which is
/// the same thing the process will do.
fn traces(env: &dyn EnvSource) -> Traces {
    let Some(raw) = env.get_os(TRACES_SAMPLE_RATE_ENV) else {
        return Traces::Off;
    };
    let Some(raw) = raw.to_str().map(str::trim) else {
        return Traces::Unreadable;
    };
    if raw.is_empty() {
        return Traces::Off;
    }
    match raw.parse::<f32>() {
        // `is_finite` rejects `NaN` and `inf`, both of which `parse` accepts
        // and neither of which is a sample rate.
        Ok(rate) if rate.is_finite() && rate == 0.0 => Traces::Off,
        Ok(rate) if rate.is_finite() && (0.0..=1.0).contains(&rate) => Traces::Sampled(rate),
        _ => Traces::Unreadable,
    }
}

/// The `environment` tag.
///
/// Defaults to the deployment kind — `desktop`, `self-hosted`, `hosted-tenant`
/// — rather than to `production`/`development`, because that is the distinction
/// this crate already models (`src/app/deployment.rs`) and a second, parallel
/// notion of "which environment am I" would drift from it. An operator running
/// staging and production tenants overrides it per deployment.
///
/// Lower-cased and trimmed so `Production` and `production ` are one value
/// rather than three rows in a filter.
fn environment(deployment: Deployment, env: &dyn EnvSource) -> String {
    env.get(ENVIRONMENT_ENV)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| deployment.as_str().to_string())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    const DSN: &str = "https://examplePublicKey@o0.ingest.sentry.io/0";

    fn resolved(pairs: &[(&str, &str)]) -> Decision {
        resolve(Deployment::SelfHosted, &MapEnv::new(pairs.to_vec()))
    }

    #[test]
    fn an_install_that_configures_nothing_is_silent() {
        assert_eq!(resolved(&[]), Decision::Silent(Silence::NoDsn));
    }

    #[test]
    fn a_configured_dsn_reports() {
        let Decision::Report {
            dsn,
            environment,
            release,
            traces,
        } = resolved(&[(DSN_ENV, DSN)])
        else {
            panic!("a configured DSN reports");
        };
        assert_eq!(dsn.expose(), DSN);
        assert_eq!(environment, "self-hosted");
        assert_eq!(release, release_tag());
        // Reporting errors is not agreeing to a per-request transaction feed.
        assert_eq!(traces, Traces::Off);
    }

    #[test]
    fn performance_tracing_is_off_until_a_rate_is_asked_for() {
        for pairs in [
            vec![(DSN_ENV, DSN)],
            vec![(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "")],
            vec![(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "  ")],
            // An explicit zero reads the same as never having set it, because
            // the process does the same thing.
            vec![(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "0")],
            vec![(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "0.0")],
        ] {
            let Decision::Report { traces, .. } = resolved(&pairs) else {
                panic!("a configured DSN reports: {pairs:?}");
            };
            assert_eq!(traces, Traces::Off, "{pairs:?}");
            assert_eq!(traces.rate(), 0.0);
            assert!(!traces.is_on());
        }
    }

    #[test]
    fn a_rate_between_zero_and_one_is_taken_as_asked() {
        for (raw, expected) in [("1", 1.0f32), ("1.0", 1.0), ("0.1", 0.1), (" 0.25 ", 0.25)] {
            let Decision::Report { traces, .. } =
                resolved(&[(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, raw)])
            else {
                panic!("a configured DSN reports: {raw}");
            };
            assert_eq!(traces, Traces::Sampled(expected), "{raw}");
            assert!(traces.is_on(), "{raw}");
        }
    }

    #[test]
    fn a_rate_that_is_not_a_fraction_is_refused_rather_than_clamped() {
        // `100` almost certainly means "100%", and clamping it to 1.0 would
        // record every request for an operator who meant nothing of the kind.
        // `-1`, `abc` and `50%` are typos. All of them land on `Off`, and all
        // of them say so in the boot line.
        for raw in [
            "100", "50%", "-1", "-0.5", "1.5", "abc", "0,5", "NaN", "inf",
        ] {
            let Decision::Report { traces, .. } =
                resolved(&[(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, raw)])
            else {
                panic!("a configured DSN reports: {raw}");
            };
            assert_eq!(traces, Traces::Unreadable, "{raw}");
            assert_eq!(traces.rate(), 0.0, "{raw}");
            assert!(!traces.is_on(), "{raw}");
        }
    }

    #[test]
    fn the_boot_line_says_what_tracing_will_do() {
        // An operator who set a rate and got a typo has to be able to tell that
        // from one who set nothing, which is the whole reason `Unreadable` is
        // a separate state.
        let off = resolved(&[(DSN_ENV, DSN)]).describe();
        assert!(off.contains("performance tracing off"), "{off}");

        let on = resolved(&[(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "0.1")]).describe();
        assert!(on.contains("tracing 10% of requests"), "{on}");

        let typo = resolved(&[(DSN_ENV, DSN), (TRACES_SAMPLE_RATE_ENV, "50%")]).describe();
        assert!(typo.contains("performance tracing off"), "{typo}");
        assert!(typo.contains(TRACES_SAMPLE_RATE_ENV), "{typo}");
    }

    #[test]
    fn off_outranks_a_configured_dsn() {
        assert_eq!(
            resolved(&[(DSN_ENV, DSN), (ENABLE_ENV, "off")]),
            Decision::Silent(Silence::OptedOut)
        );
        // Case and surrounding whitespace are the operator's, not a typo.
        assert_eq!(
            resolved(&[(DSN_ENV, DSN), (ENABLE_ENV, "  OFF ")]),
            Decision::Silent(Silence::OptedOut)
        );
    }

    #[test]
    fn on_does_not_conjure_a_destination() {
        // There is nothing for `on` to force: without a DSN there is nowhere to
        // send, and the reason must say that rather than "opted out".
        assert_eq!(
            resolved(&[(ENABLE_ENV, "on")]),
            Decision::Silent(Silence::NoDsn)
        );
    }

    #[test]
    fn a_misspelt_switch_is_silence_and_says_so() {
        // The typo that matters: an operator who meant `off` and typed `of`
        // must not keep reporting. Both directions resolve to silence, and the
        // reason distinguishes them.
        let decision = resolved(&[(DSN_ENV, DSN), (ENABLE_ENV, "of")]);
        assert_eq!(decision, Decision::Silent(Silence::Unreadable));
        assert!(decision.describe().contains("not recognised"));
    }

    #[test]
    fn a_blank_switch_is_absent_rather_than_unreadable() {
        // A launcher that exports an empty variable changes nothing.
        let Decision::Report { .. } = resolved(&[(DSN_ENV, DSN), (ENABLE_ENV, "")]) else {
            panic!("a blank switch leaves the DSN in charge");
        };
    }

    #[test]
    fn a_blank_dsn_is_absent_rather_than_unusable() {
        assert_eq!(
            resolved(&[(DSN_ENV, "   ")]),
            Decision::Silent(Silence::NoDsn)
        );
    }

    #[test]
    fn shapes_that_are_not_a_dsn_are_refused() {
        // Each of these parses as *something*, or nearly does, and each would
        // resolve to a client that never delivers.
        for candidate in [
            // No scheme — how anyone writes a proxy host the first time.
            "o0.ingest.sentry.io/0",
            // A scheme nothing can POST to.
            "ftp://key@o0.ingest.sentry.io/0",
            // No public key.
            "https://o0.ingest.sentry.io/0",
            // No project id.
            "https://key@o0.ingest.sentry.io/",
            // No host.
            "https://key@/0",
            // The pre-2016 secret-half form: no ingest accepts it, so this is
            // either stale or a credential pasted into the wrong variable.
            "https://key:secret@o0.ingest.sentry.io/0",
            // Not a URL at all.
            "not a dsn",
            "",
        ] {
            assert_eq!(
                resolve(Deployment::SelfHosted, &MapEnv::new([(DSN_ENV, candidate)])),
                match candidate {
                    "" => Decision::Silent(Silence::NoDsn),
                    _ => Decision::Silent(Silence::UnusableDsn),
                },
                "{candidate} must not resolve to reporting"
            );
        }
    }

    #[test]
    fn the_environment_tag_defaults_to_the_deployment_kind() {
        for (deployment, expected) in [
            (Deployment::Desktop, "desktop"),
            (Deployment::SelfHosted, "self-hosted"),
            (Deployment::HostedTenant, "hosted-tenant"),
        ] {
            let Decision::Report { environment, .. } =
                resolve(deployment, &MapEnv::new([(DSN_ENV, DSN)]))
            else {
                panic!("a configured DSN reports");
            };
            assert_eq!(environment, expected);
        }
    }

    #[test]
    fn the_environment_tag_is_overridable_and_normalized() {
        let Decision::Report { environment, .. } = resolve(
            Deployment::HostedTenant,
            &MapEnv::new([(DSN_ENV, DSN), (ENVIRONMENT_ENV, "  Staging ")]),
        ) else {
            panic!("a configured DSN reports");
        };
        assert_eq!(environment, "staging");
    }

    #[test]
    fn the_boot_line_never_carries_the_public_key() {
        let decision = resolved(&[(DSN_ENV, DSN)]);
        let line = decision.describe();
        assert!(!line.contains("examplePublicKey"), "{line}");
        assert!(line.contains("https://o0.ingest.sentry.io/0"), "{line}");
        assert!(line.contains("self-hosted"), "{line}");
    }

    #[test]
    fn a_dsn_never_prints_itself() {
        // `{:?}` is how a credential reaches a log without anyone deciding to
        // put it there — a `dbg!`, a `#[derive(Debug)]` on a struct that holds
        // one, a `tracing::error!(?config)`.
        let Decision::Report { dsn, .. } = resolved(&[(DSN_ENV, DSN)]) else {
            panic!("a configured DSN reports");
        };
        assert_eq!(format!("{dsn:?}"), "Dsn(<redacted>)");
        let debugged = format!("{:?}", resolved(&[(DSN_ENV, DSN)]));
        assert!(!debugged.contains("examplePublicKey"), "{debugged}");
    }

    #[test]
    fn a_proxied_dsn_loses_its_query_string_too() {
        // An ingest fronted by an authenticated proxy carries its key in the
        // two places a URL can hold one. Both have to go.
        let dsn = Dsn::new("https://key@proxy.internal/api/2?auth=hunter2#frag");
        let loggable = dsn.loggable();
        assert!(!loggable.contains("key@"), "{loggable}");
        assert!(!loggable.contains("hunter2"), "{loggable}");
        assert_eq!(loggable, "https://proxy.internal/api/2");
    }

    #[test]
    fn the_release_tag_names_the_commit_when_the_build_knows_one() {
        assert_eq!(
            release_tag_from("0.1.0", "d31e532f7c8a"),
            "opencompany@0.1.0+d31e532f7c8a"
        );
        // A modified tree is a different build and the tag says so.
        assert_eq!(
            release_tag_from("0.1.0", "d31e532f7c8a-dirty"),
            "opencompany@0.1.0+d31e532f7c8a-dirty"
        );
        // `unknown` is dropped rather than appended: a release name that looks
        // like a commit and is not one is worse than an honest absence.
        assert_eq!(release_tag_from("0.1.0", "unknown"), "opencompany@0.1.0");
        assert_eq!(release_tag_from("0.1.0", "  "), "opencompany@0.1.0");
    }

    #[test]
    fn the_real_release_tag_is_well_formed() {
        let tag = release_tag();
        assert!(tag.starts_with("opencompany@"), "{tag}");
        assert!(tag.contains(crate::VERSION), "{tag}");
        assert!(!tag.contains("unknown"), "{tag}");
    }

    #[test]
    fn every_silence_names_a_reason() {
        for reason in [
            Silence::OptedOut,
            Silence::NoDsn,
            Silence::Unreadable,
            Silence::UnusableDsn,
            Silence::NotCompiled,
        ] {
            let line = Decision::Silent(reason).describe();
            assert!(line.starts_with("crash reporting: off ("), "{line}");
            assert!(line.ends_with(')'), "{line}");
        }
    }
}
