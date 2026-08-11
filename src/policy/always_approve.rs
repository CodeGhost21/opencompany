//! `[policy].always_approve` — the operator's override, and the one matcher
//! both approval paths read (issue #684).
//!
//! ## Two matchers, one list, two different answers
//!
//! `always_approve` is the list that, per
//! [`PolicyMode::Full`](crate::harness::policy::PolicyMode), *"wins over
//! everything else, including Full autonomy"*. It is what an operator is
//! pointed at when they ask how to make sure a company never sends money
//! without asking.
//!
//! Until this module it was matched in two places, against two namespaces,
//! under two different rules:
//!
//! * [`ManifestApprovalGate::evaluate`](crate::policy::gate::ManifestApprovalGate)
//!   — the **native-effect** path, where a hosted or sidecar brain emits an
//!   effect frame. Matched `entry == effect.kind()`: exact, no prefix.
//! * [`ApprovalPolicy::check`](crate::harness::policy::ApprovalPolicy) — the
//!   **harness tool-call** path, where an openhuman agent calls a tool.
//!   Matched the **tool name**, exact *or* leading dotted segment.
//!
//! So one operator list meant two things depending on which brain was running,
//! and `always_approve = ["payment"]` parked `payment.send` on one path and
//! silently did nothing on the other. The shipped default —
//! `payment.send` / `filing.submit` / `external.publish` — is dotted effect
//! kinds, so it reached the first matcher and was **inert on the second**,
//! which is the path every company using the openhuman toolbelt actually runs.
//!
//! ## They were never two namespaces
//!
//! The fix is not to pick a winner, because there is no genuine conflict to
//! resolve. [`ApprovalPolicy::effect_for`](crate::harness::policy::ApprovalPolicy::effect_for)
//! projects a flagged tool call onto an [`Effect`](crate::ports::types::Effect)
//! by making **the tool name the effect kind, verbatim**. A tool name already
//! *is* an effect kind — the single-segment, undotted case of one. The two
//! matchers were reading the same namespace through two different rules, and
//! the rules disagreeing is the whole defect.
//!
//! So there is one namespace and, now, one matcher: [`matches`]. Both call
//! sites read it. Both syntaxes keep working, because they were always one
//! syntax.
//!
//! ## What can and cannot be validated (and why the default is empty)
//!
//! [`resolves`] answers "can this entry ever match anything this build
//! produces?", so [`Manifest::validate`](crate::company::Manifest) can reject a
//! typo loudly instead of handing back a list that silently gates nothing.
//!
//! It is **exact on one half of the namespace and heuristic on the other**, and
//! that asymmetry is a property of the system rather than a shortcut here:
//!
//! * Tool names are a **closed** set — [`declared_tools`] enumerates every one
//!   the crate can wire, and a coverage test fails if a live tool is missing
//!   from it. An entry naming a tool can be checked exactly.
//! * Effect kinds are **open by specification**. `docs/spec/integrations/medulla.md`
//!   types the wire field as `"string(1..64)"`, and `effect_from_frame` copies
//!   `frame.kind` verbatim, so the brain names its own effects. There is no set
//!   to check against, and there cannot be one without breaking every
//!   legitimate hosted-brain configuration that gates a kind this repo has
//!   never heard of.
//!
//! The best available check on that half is `effect_group_for`: a kind carrying
//! no recognised consequence word classifies as
//! [`EffectGroup::Other`](crate::ports::types::EffectGroup::Other), which
//! `evaluate_supervised` waves through — so it cannot be the thing an operator
//! meant to gate.
//!
//! **It is a floor, and a low one.** `effect_group_for` matches bare substrings
//! from a short vocabulary, and two of them are short enough to catch a great
//! deal: `pay` (so `paymnt.send` classifies as `Spend` — the typo resolves) and
//! `sign` (so `design.review` classifies as `Sign`). It reliably rejects only
//! an entry carrying no consequence substring at all — `pubish_artifact`,
//! `web_serch`. That is a real class: a mistyped **tool name** is the operator
//! error this validation actually catches, and it is the common one, because
//! tool names are what an operator has to type exactly.
//!
//! Do not read this check as a proof that a configured entry will fire. It
//! proves only that the entry is not obvious nonsense.
//!
//! **That limit is the reason [`DEFAULT_ALWAYS_APPROVE`](crate::company::DEFAULT_ALWAYS_APPROVE)
//! ships empty.** A default the runtime cannot prove will ever fire must not be
//! shipped as though it were protection: that is exactly the state issue #684
//! describes, where every company running the default believed payments and
//! publishing were gated and none of them were.

use crate::policy::consequence::declared_tools;
use crate::ports::types::EffectGroup;

/// Whether `target` is gated by the operator's `always_approve` list.
///
/// `target` is an effect kind — which, on the harness path, is the tool name
/// (see the module docs for why those are the same thing).
///
/// Matches the exact entry or a **leading dotted segment**, so `payment` gates
/// `payment.send` but not `payments_report`. The segment boundary is load
/// bearing: a bare `starts_with` would let `pay` gate `payroll.export`, which
/// is a different capability an operator did not name.
///
/// The comparison is **ASCII-case-insensitive**. Tool names reach the harness
/// matcher from openhuman's own registry, and
/// [`consequence_of`](crate::policy::consequence::consequence_of) already
/// lowercases defensively before consulting the declaration table — so a name
/// whose case differs from the operator's spelling would classify correctly and
/// then slip the override. That is the same silent-miss this module exists to
/// end, and the fail-safe direction is to match rather than to skip.
pub fn matches(always_approve: &[String], target: &str) -> bool {
    let target = target.trim();
    always_approve
        .iter()
        .any(|entry| gates(entry.trim(), target))
}

/// Whether one `always_approve` entry gates `target`. Allocation-free: this
/// runs on every gated tool call, and the old form built a `String` per entry
/// per call to test a prefix.
fn gates(entry: &str, target: &str) -> bool {
    if entry.is_empty() {
        return false;
    }
    if target.eq_ignore_ascii_case(entry) {
        return true;
    }
    // Leading dotted segment: `payment` gates `payment.send`.
    //
    // The `.` check runs before the slice, which is what makes the slice safe:
    // a byte equal to `.` is ASCII, and an ASCII byte in valid UTF-8 is always
    // a char boundary, so `target[..entry.len()]` cannot split a code point.
    target.len() > entry.len()
        && target.as_bytes()[entry.len()] == b'.'
        && target[..entry.len()].eq_ignore_ascii_case(entry)
}

/// Whether `entry` can ever match something this build is able to produce.
///
/// Two producers of gate targets exist, and an entry naming neither is a typo
/// or a name for a capability this build does not have:
///
/// 1. the declaration table — the entry names a declared tool, checked exactly;
/// 2. the effect-kind classifier — the entry carries a consequence word, so a
///    brain emitting it would produce a gateable [`EffectGroup`].
///
/// See the module docs for why (2) is a floor rather than a proof, and why
/// that is not fixable here.
pub fn resolves(entry: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    // (1) A declared tool this entry would gate — the exact name, or any tool
    // sitting under it as a dotted segment. The same rule `matches` applies,
    // asked in reverse: "is there anything this entry could gate?"
    if declared_tools().any(|tool| gates(entry, tool)) {
        return true;
    }
    // (2) A consequence-bearing effect kind a brain may emit over the wire.
    //     `effect_group_for` lowercases internally.
    crate::brain::medulla::effects::effect_group_for(entry) != EffectGroup::Other
}

/// Every entry in `always_approve` that [`resolves`] rejects, in the operator's
/// own spelling, for [`Manifest::validate`](crate::company::Manifest).
pub fn unresolved(always_approve: &[String]) -> Vec<&str> {
    always_approve
        .iter()
        .map(String::as_str)
        .filter(|entry| !resolves(entry))
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::company::DEFAULT_ALWAYS_APPROVE;

    fn list(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|e| e.to_string()).collect()
    }

    /// The bug in issue #684, pinned from the harness side: a dotted effect
    /// kind must gate a tool call of the same name.
    ///
    /// Before this module the harness matcher and the gate matcher were
    /// separate functions, and this list reached only one of them.
    #[test]
    fn an_entry_gates_the_same_name_on_either_path() {
        assert!(matches(&list(&["publish_artifact"]), "publish_artifact"));
        assert!(matches(&list(&["payment.send"]), "payment.send"));
    }

    /// The divergence the shared matcher removes: leading-segment matching was
    /// the harness rule and the gate was exact-only, so `["payment"]` gated a
    /// tool call and silently missed the identically-named native effect.
    #[test]
    fn a_leading_segment_gates_everything_under_it() {
        assert!(matches(&list(&["payment"]), "payment.send"));
        assert!(matches(&list(&["payment"]), "payment.refund"));
    }

    /// The segment boundary, which is what stops the prefix rule from being a
    /// bare `starts_with` that gates capabilities the operator never named.
    #[test]
    fn a_prefix_that_is_not_a_whole_segment_does_not_gate() {
        assert!(!matches(&list(&["pay"]), "payroll.export"));
        assert!(!matches(&list(&["payment"]), "payments_report"));
    }

    #[test]
    fn case_and_surrounding_space_do_not_defeat_the_override() {
        assert!(matches(&list(&["  Publish_Artifact "]), "publish_artifact"));
        assert!(matches(&list(&["payment.send"]), "PAYMENT.SEND"));
    }

    /// An empty entry must not become a wildcard. `"".starts_with(..)` logic is
    /// exactly how a list of typos turns into "gate everything", which would be
    /// fail-safe but would also brick every company that had one.
    #[test]
    fn an_empty_entry_gates_nothing() {
        assert!(!matches(&list(&["", "   "]), "publish_artifact"));
        assert!(!matches(&[], "publish_artifact"));
    }

    #[test]
    fn a_declared_tool_resolves() {
        assert!(resolves("publish_artifact"));
        assert!(resolves("web_search"));
        assert!(resolves("shell"));
    }

    #[test]
    fn a_consequence_bearing_effect_kind_resolves() {
        // No tool of these names exists — and none needs to. A hosted brain
        // names its own effects, and each of these classifies into a group
        // `evaluate_supervised` parks.
        assert!(resolves("payment.send"));
        assert!(resolves("filing.submit"));
        assert!(resolves("external.publish"));
    }

    /// The fail-closed case: an entry that names neither a tool nor a
    /// consequence-bearing kind can never fire, and the operator must be told
    /// rather than handed a list that silently gates nothing.
    ///
    /// These are all **mistyped tool names**, which is the operator error this
    /// check actually catches — and the common one, because a tool name is the
    /// thing an operator has to spell exactly.
    #[test]
    fn a_mistyped_tool_name_does_not_resolve() {
        assert!(!resolves("pubish_artifact"));
        assert!(!resolves("web_serch"));
        assert!(!resolves("totally_made_up"));
        assert!(!resolves(""));
    }

    /// The floor, pinned as a limit rather than left to be discovered.
    ///
    /// `effect_group_for` matches bare substrings, and `pay` and `sign` are
    /// short enough to catch words that were never meant to be consequence
    /// vocabulary. So a typo'd *effect kind* generally still resolves, and this
    /// validation must not be described — in docs, in a review, or to an
    /// operator — as proving that a configured entry will fire.
    ///
    /// Asserted rather than commented so that tightening `effect_group_for`
    /// later fails here and makes somebody update the claim.
    #[test]
    fn the_effect_kind_half_of_the_check_is_a_low_floor() {
        // `pay` is a substring of `paymnt`, so the typo classifies as `Spend`.
        assert!(resolves("paymnt.send"));
        // `sign` is a substring of `design`.
        assert!(resolves("design.review"));
    }

    #[test]
    fn unresolved_reports_the_operators_own_spelling() {
        let entries = list(&["web_search", "pubish_artifact", "publish_artifact"]);
        assert_eq!(unresolved(&entries), vec!["pubish_artifact"]);
        assert!(unresolved(&list(&["web_search"])).is_empty());
    }

    /// The drift guard issue #684 asks for, and the reason the default is
    /// empty.
    ///
    /// **The loop is vacuous today, deliberately and visibly**: the default
    /// ships `[]` because nothing in this build emits the three kinds it used
    /// to name, and the one real name behind them (`publish_artifact`) must not
    /// be defaulted — issue #658 ruled that `full` publishes unattended. The
    /// assertion below the loop is what makes this test non-vacuous now: it
    /// pins the emptiness as a decision, so restoring an entry has to come
    /// through here and face the loop.
    #[test]
    fn every_default_entry_resolves_and_the_default_is_empty() {
        for entry in DEFAULT_ALWAYS_APPROVE {
            assert!(
                resolves(entry),
                "the shipped default gates `{entry}`, which names nothing this \
                 build can produce — issue #684 all over again"
            );
        }
        assert!(
            DEFAULT_ALWAYS_APPROVE.is_empty(),
            "the default is empty on purpose (issue #684): a default that \
             cannot be proven to fire must not ship as though it were \
             protection. Adding an entry is a product decision — see #658 for \
             why `publish_artifact` in particular is not it."
        );
    }
}
