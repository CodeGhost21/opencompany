//! Whether a company's teammates can think, and — when they cannot — which of
//! the two reasons it is (issue #1735).
//!
//! The host already knows this and never said it. `/setup` reports
//! `acp_in_build`, `harness_in_build`, `mcp_in_build` and `oauth_in_build`;
//! `…/capabilities` reports `media_in_build`, `search_in_build`,
//! `publish_in_build` and `mcp_in_build`. None of them describes cognition, so
//! the console had no way to tell a considered reply from
//! [`EchoBrain`](crate::brain::EchoBrain)'s `"You said: …"` — and chat rendered
//! the echo under the teammate's own avatar and name (issue #1734).
//!
//! This is deliberately **not** a fifth `*_in_build` boolean. Cognition is two
//! facts at once — whether an agent harness is reachable at all, and whether a
//! model is configured at runtime — and only the second is something an
//! operator can act on without a new build. A single flag collapses them, which
//! sends the operator who needs one settings page off looking for a new binary.
//!
//! The states are named for their **remedy**, not for the mechanism behind
//! them. "The harness is not compiled in" and "the harness is compiled in and
//! this host never attached a pool" are different mechanisms with the same
//! remedy — neither is reachable from a settings page — so they share
//! [`CognitionState::Unavailable`]. Splitting them would offer the operator a
//! distinction they cannot act on; folding either into
//! [`CognitionState::Unconfigured`] would promise a settings page that cannot
//! help, which is the failure `ops::inference`'s `harness_reachable` already
//! exists to stop (issues #266, #514).

use serde::Serialize;

/// Whether this company's teammates can think, and why not when they cannot.
///
/// **Derived, never stored.** [`cognition_state`] reads the brain the runtime is
/// actually holding and the feature set this binary was compiled with, so there
/// is no second copy of the answer to drift from the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CognitionState {
    /// A cognition path that runs a real model is live — the harness, the
    /// hosted Medulla brain, a sidecar, or a brain the embedder injected.
    ///
    /// Says nothing about whether that provider will answer: reachability is
    /// what `POST …/inference/test` probes. This is the narrower question of
    /// whether anything but the echo brain is in the socket.
    Configured,
    /// An agent harness is reachable on this host, but the company resolved no
    /// inference source at boot, so it is running the offline echo brain and
    /// answering every message with a canned line.
    ///
    /// Fixable in the app, at Settings → Inference. This is the state a fresh
    /// instance starts in, and the one the operator is most likely to mistake
    /// for the product being stupid.
    ///
    /// **Requires a harness that is actually attached**, not merely compiled
    /// in — see [`cognition_state`]. Reporting this for a runtime with no pool
    /// would send the operator to a settings page that cannot move them off the
    /// echo brain, which is the exact dead end `restart_pending` and
    /// `runner_gap_for` in [`crate::server::ops::inference`] refuse to walk an
    /// operator into.
    Unconfigured,
    /// No agent harness is reachable on this host, so no model configuration
    /// gets anywhere near one. Only a different build — or a host that wires a
    /// harness pool onto its runtimes — changes this.
    ///
    /// Two mechanisms land here and the operator can act on neither: the
    /// `openhuman` feature is not compiled into this binary, or it is and the
    /// embedder built its runtimes without calling
    /// [`crate::app::harness::attach`] (the failure that module exists for —
    /// the desktop shell shipped companies with no harness in a build that
    /// compiled one in).
    ///
    /// The console must say so plainly rather than offering a settings link
    /// that cannot help — the same rule `api/setup.ts` states for the
    /// `*_in_build` flags, which exist "so the flow can say 'not in this build'
    /// instead of offering a switch that does nothing".
    Unavailable,
}

impl CognitionState {
    /// The stable wire label, for tests and diagnostics that would otherwise
    /// re-spell the serde renaming by hand.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Unconfigured => "unconfigured",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Derives [`CognitionState`] from the brain a runtime actually holds and the
/// feature set this binary carries.
///
/// `path` is [`Cognition::path`](crate::ports::brain::Cognition::path) — read
/// off the live brain, not off the stored config, because a config that
/// resolves is not the same fact as a brain that was built from it. A company
/// configured after boot keeps the brain it started with until its runtime is
/// rebuilt, and reporting the config would tell that operator their teammates
/// can think while they are still being echoed at.
///
/// `harness_reachable` is whether an agent harness pool is actually attached to
/// this runtime — `crate::server::ops::inference::harness_reachable`, the same
/// predicate `restart_pending` and `runner_gap_for` gate their "configure
/// inference" and "restart" advice on, rather than a second copy of it.
///
/// **Not `cfg!(feature = "openhuman")`.** The feature says the harness was
/// compiled in; it does not say this company's runtime was ever handed a pool.
/// An embedder that builds a [`RuntimeBuilder`](crate::runtime::RuntimeBuilder)
/// without [`crate::app::harness::attach`] gets exactly that — an `openhuman`
/// binary whose companies sit on the echo brain with no harness behind them,
/// which is the shipped bug `app::harness` was written to end. Deriving from
/// the feature alone reports [`CognitionState::Unconfigured`] there and points
/// the operator at Settings → Inference, which cannot move that runtime off the
/// echo brain no matter what they save.
///
/// Passed in rather than read here so the three-way matrix is testable without
/// a runtime per arm — in particular the `unavailable` arm, which a lane that
/// enables the feature could otherwise reach only by constructing a
/// harness-less runtime.
///
/// The echo brain is the only path that runs no model
/// ([`ECHO_PATH`](crate::ports::brain::ECHO_PATH)), so every other label —
/// `harness`, `hosted`, `sidecar`, `custom` — is cognition of some kind and
/// reports [`CognitionState::Configured`]. Matching on the one degraded path
/// rather than allow-listing the working ones is what keeps a brain added later
/// from defaulting to "cannot think".
pub fn cognition_state(path: &str, harness_reachable: bool) -> CognitionState {
    if path != crate::ports::brain::ECHO_PATH {
        return CognitionState::Configured;
    }
    if harness_reachable {
        CognitionState::Unconfigured
    } else {
        CognitionState::Unavailable
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ports::brain::{ECHO_PATH, HARNESS_PATH};

    /// The whole matrix, both reachability settings, in one place — including
    /// the `unavailable` arm that a lane enabling the feature could otherwise
    /// reach only by constructing a harness-less runtime.
    #[test]
    fn the_three_states_are_derived_from_the_path_and_harness_reachability() {
        assert_eq!(
            cognition_state(HARNESS_PATH, true),
            CognitionState::Configured,
        );
        assert_eq!(cognition_state("hosted", true), CognitionState::Configured);
        assert_eq!(cognition_state("sidecar", true), CognitionState::Configured);
        assert_eq!(cognition_state("custom", true), CognitionState::Configured);
        assert_eq!(
            cognition_state(ECHO_PATH, true),
            CognitionState::Unconfigured,
            "a harness is attached, so a provider really is one settings page away",
        );
        assert_eq!(
            cognition_state(ECHO_PATH, false),
            CognitionState::Unavailable,
            "no harness behind this runtime: no configuration reaches a model",
        );
    }

    /// The regression this exists for: a build with no harness, sitting on the
    /// echo brain, must never report itself as able to think. That is the state
    /// the console renders `"You said: …"` under a teammate's name in.
    #[test]
    fn a_build_with_no_harness_never_reports_itself_configured() {
        assert_ne!(
            cognition_state(ECHO_PATH, false),
            CognitionState::Configured,
        );
        assert_ne!(cognition_state(ECHO_PATH, true), CognitionState::Configured,);
    }

    /// A harness that is compiled in but never attached must not be sold to the
    /// operator as a settings problem (codex review of PR #1740).
    ///
    /// This is the case `cfg!(feature = "openhuman")` alone gets wrong. An
    /// embedder that skips [`crate::app::harness::attach`] — the shipped
    /// desktop-shell bug that module was written to end — leaves an `openhuman`
    /// binary whose companies hold no pool. Saying `unconfigured` there points
    /// the operator at Settings → Inference, and nothing they save moves that
    /// runtime off the echo brain. The input is reachability precisely so this
    /// arm exists; asserting it here is what stops a later "simplification"
    /// back to the feature flag.
    #[test]
    fn a_compiled_in_harness_that_is_not_attached_is_not_a_settings_problem() {
        assert_eq!(
            cognition_state(ECHO_PATH, false),
            CognitionState::Unavailable,
        );
        assert_ne!(
            cognition_state(ECHO_PATH, false),
            CognitionState::Unconfigured,
            "no attached pool: Settings → Inference is a dead end here",
        );
    }

    /// A path this module has never heard of is cognition until proven
    /// otherwise. The alternative — allow-listing the working paths — would
    /// report "cannot think" for the next brain someone adds, and the symptom
    /// (a banner on a company that is thinking perfectly well) points nowhere
    /// near this function.
    #[test]
    fn an_unknown_path_is_treated_as_cognition() {
        assert_eq!(
            cognition_state("some-brain-added-later", false),
            CognitionState::Configured,
        );
    }

    #[test]
    fn the_wire_labels_match_the_serde_renaming() {
        for state in [
            CognitionState::Configured,
            CognitionState::Unconfigured,
            CognitionState::Unavailable,
        ] {
            assert_eq!(
                serde_json::to_value(state).expect("serialize"),
                serde_json::Value::String(state.as_str().to_string()),
            );
        }
    }
}
