//! The enable/disable decision: whether this process reports at all, and where.
//!
//! Kept apart from both the transport and the payload because it is the part
//! that has to be *provably* right. It is pure — an [`EnvSource`] and a
//! [`Deployment`] in, a [`Decision`] out — so every branch of it is tested in
//! the default build, with no network and no feature flag.

use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// Operator override: `on` forces reporting, `off` forbids it.
pub const ENABLE_ENV: &str = "OPENCOMPANY_ANALYTICS";
/// The Mixpanel project token. **Configuration, never a compiled-in constant** —
/// a token baked into a public binary is a token everyone has.
pub const TOKEN_ENV: &str = "OPENCOMPANY_ANALYTICS_TOKEN";
/// Overrides the collector URL. Exists so a test can point at a local server,
/// and so a deployment can front Mixpanel with its own proxy.
pub const ENDPOINT_ENV: &str = "OPENCOMPANY_ANALYTICS_ENDPOINT";

/// Where events go when nothing overrides it.
pub const DEFAULT_ENDPOINT: &str = "https://api.mixpanel.com/track";

/// A Mixpanel project token.
///
/// A newtype rather than a bare `String` for one reason: it must never be
/// printed, logged, or serialized by accident. It derives **neither** `Debug`
/// nor `Serialize` — the hand-written `Debug` redacts — because
/// `serde_json::to_value(&some_config)` is precisely how a credential reaches a
/// payload (issue #1741, `SecretValue`). Nothing in this module ever serializes
/// a config struct; the token is read out explicitly, once, at the moment a
/// request body is built.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectToken(String);

impl ProjectToken {
    /// Wraps a token read from configuration.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The token, for the one caller that puts it on the wire.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ProjectToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProjectToken(<redacted>)")
    }
}

/// Why a process is not reporting. Logged once at boot, so an operator who
/// *expected* analytics can tell "switched off" from "misconfigured".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    /// The operator set `OPENCOMPANY_ANALYTICS=off`.
    OptedOut,
    /// Not a hosted tenant, and nobody opted in. **The default.**
    NotHosted,
    /// Reporting was asked for, but no project token is configured.
    NoToken,
    /// `OPENCOMPANY_ANALYTICS` was set to something this does not recognise.
    ///
    /// A separate reason from [`Self::OptedOut`] on purpose: an operator who
    /// typed `of` gets the outcome they meant *and* a boot line saying their
    /// value was not understood, rather than silence they cannot distinguish
    /// from a working opt-out.
    Unreadable,
}

impl Silence {
    /// The stable reason slug, for the boot log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptedOut => "operator opted out",
            Self::NotHosted => "not a hosted tenant and no explicit opt-in",
            Self::NoToken => "no project token is configured",
            Self::Unreadable => "the OPENCOMPANY_ANALYTICS value is not recognised",
        }
    }
}

/// What this process will do.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Send nothing. No client is constructed, so nothing *can* be sent.
    Silent(Silence),
    /// Report to `endpoint` under `token`.
    Report {
        /// The collector URL.
        endpoint: String,
        /// The project token.
        token: ProjectToken,
    },
}

impl Decision {
    /// Whether this decision reports.
    pub fn reports(&self) -> bool {
        matches!(self, Self::Report { .. })
    }
}

/// Resolves the decision.
///
/// The order matters and is the whole policy:
///
/// 1. `OPENCOMPANY_ANALYTICS=off` wins over everything. An operator switching
///    it off must not be overruled by a deployment kind, a token, or a future
///    default.
/// 2. A value that is set but unrecognised resolves to **silence**, whatever
///    the deployment. The deployment default is reserved for a switch that is
///    *absent*. Falling an unreadable value through to the default meant a
///    hosted tenant whose operator typed `OPENCOMPANY_ANALYTICS=of` kept
///    reporting — a typo in the opt-out direction silently ignored, which is
///    the one direction that must never be silently ignored.
/// 3. Otherwise reporting is on **only** for [`Deployment::HostedTenant`], or
///    when an operator explicitly sets `OPENCOMPANY_ANALYTICS=on`. Decision 1
///    of #1739: silence is the default and reporting is the exception, so a
///    self-hosted or desktop install that has said nothing sends nothing.
/// 4. A token is required. Without one there is nowhere to report to, and
///    guessing is not an option — see [`TOKEN_ENV`].
pub fn resolve(deployment: Deployment, env: &dyn EnvSource) -> Decision {
    // Blank is absent, for the same reason a blank token is: a variable set to
    // whitespace is a variable nobody meant to set. See [`non_blank`].
    let switch = env
        .get(ENABLE_ENV)
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty());

    match switch.as_deref() {
        Some("off" | "false" | "0" | "no") => return Decision::Silent(Silence::OptedOut),
        Some("on" | "true" | "1" | "yes") => {}
        // Set, but not a spelling of yes or no. Both directions of that typo
        // are now silence: it was never an opt-in, and — since it reached a
        // hosted tenant's deployment default and kept reporting — it must not
        // be a failed opt-*out* either. Silence is the safe answer to "I cannot
        // tell what you asked for", and the boot line says which value it could
        // not read.
        Some(_) => return Decision::Silent(Silence::Unreadable),
        None => {
            if deployment != Deployment::HostedTenant {
                return Decision::Silent(Silence::NotHosted);
            }
        }
    }

    let Some(token) = non_blank(env, TOKEN_ENV) else {
        return Decision::Silent(Silence::NoToken);
    };

    Decision::Report {
        endpoint: non_blank(env, ENDPOINT_ENV).unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        token: ProjectToken::new(token),
    }
}

/// A configured value, trimmed, or `None` when there is nothing left of it.
///
/// [`EnvSource::get`] already drops an *empty* value, but not a whitespace-only
/// one, and the difference is not academic: a token or an endpoint mounted from
/// a file arrives with a trailing newline more often than not. Untrimmed, a
/// hosted tenant whose token is `"\n"` resolves to [`Decision::Report`], the
/// boot line says "reporting to …", and every batch is refused by the collector
/// — the failure mode #1739 added that line to prevent. A blank `ENDPOINT_ENV`
/// is worse, because it replaces [`DEFAULT_ENDPOINT`] with a URL that cannot
/// parse, so nothing is sent and nothing says why.
///
/// The same trim-and-filter the rest of the tree applies to environment values
/// (`src/bin/opencompany.rs`).
fn non_blank(env: &dyn EnvSource, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    fn token_env(pairs: &[(&str, &str)]) -> MapEnv {
        let mut all = vec![(TOKEN_ENV, "not-a-real-token")];
        all.extend_from_slice(pairs);
        MapEnv::new(all)
    }

    /// **The decision the GPL posture rests on.** A self-hosted instance that
    /// has been handed a token — which is the easiest way to get this wrong,
    /// because a token looks like consent — still sends nothing.
    #[test]
    fn a_self_hosted_instance_is_silent_even_with_a_token() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &token_env(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_desktop_instance_is_silent_even_with_a_token() {
        assert_eq!(
            resolve(Deployment::Desktop, &token_env(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_hosted_tenant_with_a_token_reports() {
        let decision = resolve(Deployment::HostedTenant, &token_env(&[]));
        assert!(decision.reports(), "{decision:?}");
        match decision {
            Decision::Report { endpoint, token } => {
                assert_eq!(endpoint, DEFAULT_ENDPOINT);
                assert_eq!(token.expose(), "not-a-real-token");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A hosted tenant with no token is misconfigured, not reporting to
    /// nowhere — and the reason says which.
    #[test]
    fn a_hosted_tenant_without_a_token_is_silent() {
        assert_eq!(
            resolve(Deployment::HostedTenant, &MapEnv::default()),
            Decision::Silent(Silence::NoToken)
        );
    }

    /// `off` outranks the deployment kind. The platform can switch a tenant off
    /// without rebuilding it.
    #[test]
    fn off_outranks_a_hosted_deployment() {
        assert_eq!(
            resolve(Deployment::HostedTenant, &token_env(&[(ENABLE_ENV, "off")])),
            Decision::Silent(Silence::OptedOut)
        );
    }

    /// The self-hoster's opt-in, which is the only way a non-hosted install ever
    /// reports.
    #[test]
    fn a_self_hoster_can_opt_in() {
        assert!(resolve(Deployment::SelfHosted, &token_env(&[(ENABLE_ENV, "on")])).reports());
    }

    /// A typo must not opt anybody in.
    #[test]
    fn a_misspelled_switch_does_not_opt_in() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &token_env(&[(ENABLE_ENV, "onn")])),
            Decision::Silent(Silence::Unreadable)
        );
    }

    /// **And a typo must not fail to opt anybody out.** This is the direction
    /// that used to leak: an unreadable value fell through to the deployment
    /// default, so a hosted tenant whose operator meant `off` and typed `of`
    /// carried on reporting, with a boot line that said "reporting to …" and
    /// gave them no reason to look again.
    #[test]
    fn a_misspelled_opt_out_does_not_keep_a_hosted_tenant_reporting() {
        for typo in ["of", "offf", "disabled", "0.0", "nope"] {
            let decision = resolve(Deployment::HostedTenant, &token_env(&[(ENABLE_ENV, typo)]));
            assert_eq!(
                decision,
                Decision::Silent(Silence::Unreadable),
                "{typo:?} must not leave a hosted tenant reporting"
            );
            assert!(!decision.reports(), "{typo:?}");
        }
    }

    /// The near-miss control: `off` really is matched case-insensitively and
    /// after trimming, so the test above is finding typos rather than finding
    /// every value that is not lowercase and bare.
    #[test]
    fn an_off_switch_is_trimmed_and_case_folded() {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &token_env(&[(ENABLE_ENV, "  ofF\n")])
            ),
            Decision::Silent(Silence::OptedOut)
        );
    }

    /// The control for the two above: an **absent** switch still falls to the
    /// deployment default, in both directions. Without this, "everything is
    /// silent now" would pass the tests above just as well.
    #[test]
    fn an_absent_switch_still_falls_to_the_deployment_default() {
        assert!(resolve(Deployment::HostedTenant, &token_env(&[])).reports());
        assert_eq!(
            resolve(Deployment::SelfHosted, &token_env(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    /// A whitespace-only switch is an absent switch, not an unreadable one —
    /// consistent with the token and endpoint, and it must not flip a hosted
    /// tenant into silence just because a launcher exported an empty variable.
    #[test]
    fn a_blank_switch_is_treated_as_absent() {
        assert!(
            resolve(Deployment::HostedTenant, &token_env(&[(ENABLE_ENV, "   ")])).reports(),
            "a blank switch must not read as unreadable"
        );
    }

    /// A token that is only whitespace is not a token. This is not a theoretical
    /// value: a secret mounted from a file arrives with a trailing newline, and
    /// a hosted tenant handed a blank one must read as **misconfigured** rather
    /// than as reporting — otherwise boot prints "reporting to …" and every
    /// batch is silently refused by the collector.
    ///
    /// `EnvSource::get` already drops an *empty* value, so the whitespace-only
    /// case is the one that needs this and the one asserted here.
    #[test]
    fn a_blank_token_is_no_token() {
        for blank in ["   ", "\n", "\t\n "] {
            assert_eq!(
                resolve(Deployment::HostedTenant, &MapEnv::new([(TOKEN_ENV, blank)])),
                Decision::Silent(Silence::NoToken),
                "a token of {blank:?} must not read as configured"
            );
        }
    }

    /// And a token that merely *arrived* with surrounding whitespace is used,
    /// trimmed, rather than put on the wire with a newline in it.
    #[test]
    fn a_token_is_trimmed() {
        match resolve(
            Deployment::HostedTenant,
            &MapEnv::new([(TOKEN_ENV, "  not-a-real-token\n")]),
        ) {
            Decision::Report { token, .. } => assert_eq!(token.expose(), "not-a-real-token"),
            other => panic!("{other:?}"),
        }
    }

    /// A blank endpoint falls back to the default rather than replacing it with
    /// a URL that cannot parse — the shape of this bug that says nothing at all
    /// at boot, because the line still reads "reporting to".
    #[test]
    fn a_blank_endpoint_falls_back_to_the_default() {
        match resolve(
            Deployment::HostedTenant,
            &token_env(&[(ENDPOINT_ENV, "  \n")]),
        ) {
            Decision::Report { endpoint, .. } => assert_eq!(endpoint, DEFAULT_ENDPOINT),
            other => panic!("{other:?}"),
        }
    }

    /// The positive control for the two above, and deliberately **insensitive**
    /// to the trim: no surrounding whitespace, so this test passes both with the
    /// filter and without it. Without such a control, "every test in the group
    /// fails when I revert the fix" would be evidence that the group asserts the
    /// implementation rather than the behaviour.
    #[test]
    fn a_configured_endpoint_still_overrides() {
        match resolve(
            Deployment::HostedTenant,
            &token_env(&[(ENDPOINT_ENV, "http://127.0.0.1:9/track")]),
        ) {
            Decision::Report { endpoint, .. } => assert_eq!(endpoint, "http://127.0.0.1:9/track"),
            other => panic!("{other:?}"),
        }
    }

    /// The token is a credential: it must not be printable by accident, because
    /// the accident is a `{:?}` in a log line nobody reviewed.
    #[test]
    fn a_token_is_not_printable() {
        let token = ProjectToken::new("not-a-real-token");
        let printed = format!("{token:?}");
        assert!(
            !printed.contains("not-a-real-token"),
            "the Debug impl leaked the token: {printed}"
        );

        let decision = Decision::Report {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            token,
        };
        let printed = format!("{decision:?}");
        assert!(
            !printed.contains("not-a-real-token"),
            "the Debug impl leaked the token through the decision: {printed}"
        );
    }
}
