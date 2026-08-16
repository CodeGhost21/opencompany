//! Which workspace documents each role is told to reason from.
//!
//! Implements `docs/spec/runtime/orchestration/context-routing.md`. The rule it
//! exists to enforce:
//!
//! > **Context is authority.** A document routed into a role's system prompt is
//! > something that role is being told to reason from. Route it deliberately,
//! > and record why each exclusion is an exclusion.
//!
//! Two halves live here, and both are pure decisions over manifest data — no
//! store, no I/O — so they are compiled and tested in every build even though
//! the harness that spends the result is behind the `openhuman` feature:
//!
//! * [`routed_documents`] — the per-tier default table, and how an explicit
//!   `context` key overrides it.
//! * [`excluded_documents`] — the class-based exclusions, which are subtractive
//!   and apply to defaults and explicit lists alike.

use crate::company::Agent;

/// The one document every role is routed, whatever its tier or classes.
///
/// The company's method policy: how this company works, as distinct from what it
/// currently believes. Universal because a role that does not know the method
/// cannot follow it, and unlike every other document here it asserts nothing
/// about the work in progress, so no exclusion can apply to it.
pub const UNIVERSAL_DOCUMENT: &str = "METHOD.md";

/// The company's summarized picture: what is established, what is ruled out.
pub const BRIEF: &str = "BRIEF.md";
/// The evidence ledger — what already holds true, with its derivation.
pub const CLAIMS: &str = "CLAIMS.md";
/// The open-question tracker the orchestrator routes work from.
pub const THREADS: &str = "THREADS.md";
/// The assertion board: posts are asserted, not established.
pub const BOARD: &str = "BOARD.md";
/// Provisional working-out, kept out of any role that judges.
pub const SCRATCH: &str = "SCRATCH.md";

/// The documents a role is routed when its manifest declares no `context` key.
///
/// Keyed on the role's tier, per the spec's default table. Every row is a
/// default, never a floor or a ceiling: an explicit `context` — including an
/// empty one — always wins for that role.
///
/// **An agent with no `tier` takes the `reasoning` row.** `tier` is optional and
/// most roster entries omit it, so the table needs a defined fallback or it
/// covers almost nobody. `reasoning` is right because it is what an undeclared
/// teammate *is*: a worker doing the substantive job its description names.
/// Defaulting to `orchestrator` would hand every unlabelled agent the routing
/// picture, and defaulting to none would leave the ordinary case with no working
/// context at all.
fn tier_defaults(tier: Option<&str>) -> &'static [&'static str] {
    match tier {
        // Decides what happens next across the whole company, so it needs the
        // established picture plus both derived ledgers — without them it would
        // re-derive routing decisions from raw notes every cycle.
        Some("orchestrator") => &[BRIEF, CLAIMS, THREADS],
        // Talks to the operator or another company: needs the summarized picture
        // to speak from, not the derivation detail behind it.
        Some("frontend") => &[BRIEF],
        // Reads and summarizes raw workspace notes to *write* the brief. Routing
        // it the brief would be circular.
        Some("compress") => &[],
        // Runs over compressed history between cycles, not the live workspace: a
        // routed document would be stale by construction before the tick that
        // read it ran.
        Some("subconscious") => &[],
        // `reasoning`, and every agent that declares no tier: does the
        // substantive work a demand asks for, so it needs what is established
        // and what already holds — but not the open-question tracker, which is
        // the orchestrator's routing concern.
        _ => &[BRIEF, CLAIMS],
    }
}

/// The documents a role's classes forbid, whatever the routing table says.
///
/// Each rule prevents a specific observed failure, and each is subtractive: an
/// exclusion outranks both the tier default and an explicit `context` list,
/// because the point of declaring a class is that the exclusion cannot be lost
/// by someone editing a routing line.
///
/// * A role that **weighs evidence** must not be routed the assertion board. A
///   post is asserted, not established; a critic scoring a deliverable beside an
///   unevidenced sentence is one prompt away from scoring the sentence.
/// * A role that **judges** must not be routed the scratch. Provisional
///   working-out read as progress is what keeps a loop retrying.
/// * A role **acting on an operator directive** must not be routed the claim
///   ledger. A directive is asserted, and a role holding the evidence ledger
///   while carrying out an instruction is one prompt away from filing the
///   instruction as a finding.
pub fn excluded_documents(classes: &[String]) -> Vec<&'static str> {
    let mut excluded = Vec::new();
    for class in classes {
        match class.as_str() {
            "evidence" => excluded.push(BOARD),
            "judge" => excluded.push(SCRATCH),
            "directive" => excluded.push(CLAIMS),
            _ => {}
        }
    }
    excluded
}

/// The workspace documents to route into `agent`'s system prompt.
///
/// Resolution order:
///
/// 1. the universal document, always;
/// 2. the agent's explicit `context` list if it declared one, else its tier's
///    default row;
/// 3. minus anything its classes exclude.
///
/// `Some(vec![])` (an explicit `context = []`) and `None` (an omitted key) are
/// deliberately different: the first means "the universal document and nothing
/// else", the second means "take the default". `Agent::context` is
/// `Option<Vec<String>>` precisely so that distinction is representable.
///
/// Returned in routing order with duplicates removed, so a manifest that lists
/// the universal document explicitly does not get it twice.
pub fn routed_documents(agent: &Agent) -> Vec<String> {
    let excluded = excluded_documents(&agent.classes);

    let chosen: Vec<String> = match agent.context.as_deref() {
        Some(explicit) => explicit.to_vec(),
        None => tier_defaults(agent.tier.as_deref())
            .iter()
            .map(|doc| doc.to_string())
            .collect(),
    };

    let mut routed = Vec::with_capacity(chosen.len() + 1);
    let mut seen = std::collections::HashSet::new();
    for document in std::iter::once(UNIVERSAL_DOCUMENT.to_string()).chain(chosen) {
        let document = document.trim().to_string();
        if document.is_empty() {
            continue;
        }
        // The universal document is exempt from exclusion: it is method, not
        // assertion, so no class has a reason to withhold it — and a role
        // excluded from the method could not follow it.
        if document != UNIVERSAL_DOCUMENT && excluded.contains(&document.as_str()) {
            continue;
        }
        if seen.insert(document.clone()) {
            routed.push(document);
        }
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(tier: Option<&str>) -> Agent {
        Agent {
            id: "a".into(),
            role: "Role".into(),
            description: None,
            tier: tier.map(str::to_string),
            tools: Vec::new(),
            delegates_to: Vec::new(),
            context: None,
            budget_usd_daily: None,
            prompt: None,
            prompt_files: Vec::new(),
            prompt_files_resolved: Vec::new(),
            classes: Vec::new(),
        }
    }

    #[test]
    fn every_role_is_routed_the_universal_document() {
        for tier in [
            None,
            Some("orchestrator"),
            Some("reasoning"),
            Some("frontend"),
            Some("compress"),
            Some("subconscious"),
        ] {
            let routed = routed_documents(&agent(tier));
            assert!(
                routed.contains(&UNIVERSAL_DOCUMENT.to_string()),
                "tier {tier:?} → {routed:?}"
            );
        }
    }

    #[test]
    fn the_per_tier_default_table_matches_the_spec() {
        assert_eq!(
            routed_documents(&agent(Some("orchestrator"))),
            [UNIVERSAL_DOCUMENT, BRIEF, CLAIMS, THREADS]
        );
        assert_eq!(
            routed_documents(&agent(Some("reasoning"))),
            [UNIVERSAL_DOCUMENT, BRIEF, CLAIMS]
        );
        assert_eq!(
            routed_documents(&agent(Some("frontend"))),
            [UNIVERSAL_DOCUMENT, BRIEF]
        );
        assert_eq!(routed_documents(&agent(Some("compress"))), [UNIVERSAL_DOCUMENT]);
        assert_eq!(
            routed_documents(&agent(Some("subconscious"))),
            [UNIVERSAL_DOCUMENT]
        );
    }

    /// Most roster entries omit `tier`, so the fallback covers almost everybody.
    #[test]
    fn an_agent_with_no_tier_takes_the_reasoning_row() {
        assert_eq!(
            routed_documents(&agent(None)),
            routed_documents(&agent(Some("reasoning")))
        );
    }

    /// The distinction `Option<Vec<String>>` exists to represent.
    #[test]
    fn an_explicit_empty_context_is_not_the_same_as_an_omitted_one() {
        let mut explicit = agent(Some("orchestrator"));
        explicit.context = Some(Vec::new());
        assert_eq!(
            routed_documents(&explicit),
            [UNIVERSAL_DOCUMENT],
            "`context = []` means the universal document and nothing else"
        );

        assert_eq!(
            routed_documents(&agent(Some("orchestrator"))),
            [UNIVERSAL_DOCUMENT, BRIEF, CLAIMS, THREADS],
            "an omitted key takes the tier default"
        );
    }

    #[test]
    fn an_explicit_context_overrides_the_tier_default() {
        let mut a = agent(Some("orchestrator"));
        a.context = Some(vec!["GOAL.md".into()]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT, "GOAL.md"]);
    }

    #[test]
    fn a_role_that_weighs_evidence_is_never_routed_the_board() {
        let mut a = agent(Some("reasoning"));
        a.classes = vec!["evidence".into()];
        a.context = Some(vec![BRIEF.into(), BOARD.into()]);
        let routed = routed_documents(&a);
        assert!(!routed.contains(&BOARD.to_string()), "{routed:?}");
        assert!(routed.contains(&BRIEF.to_string()), "{routed:?}");
    }

    #[test]
    fn a_role_that_judges_is_never_routed_the_scratch() {
        let mut a = agent(Some("reasoning"));
        a.classes = vec!["judge".into()];
        a.context = Some(vec![SCRATCH.into()]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT]);
    }

    #[test]
    fn a_role_acting_on_a_directive_is_never_routed_the_claim_ledger() {
        let mut a = agent(Some("reasoning"));
        a.classes = vec!["directive".into()];
        // CLAIMS is in the `reasoning` default row, so this proves the exclusion
        // subtracts from defaults and not only from explicit lists.
        let routed = routed_documents(&a);
        assert!(!routed.contains(&CLAIMS.to_string()), "{routed:?}");
        assert!(routed.contains(&BRIEF.to_string()), "{routed:?}");
    }

    /// An exclusion outranks an explicit routing line — that is what makes a
    /// declared class a control rather than a suggestion somebody can edit away.
    #[test]
    fn an_exclusion_outranks_an_explicit_context_entry() {
        let mut a = agent(None);
        a.classes = vec!["judge".into()];
        a.context = Some(vec![SCRATCH.into(), BRIEF.into()]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT, BRIEF]);
    }

    #[test]
    fn several_classes_all_apply() {
        let mut a = agent(None);
        a.classes = vec!["judge".into(), "evidence".into(), "directive".into()];
        a.context = Some(vec![SCRATCH.into(), BOARD.into(), CLAIMS.into(), BRIEF.into()]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT, BRIEF]);
    }

    /// The method policy is exempt: it is how the company works, not something
    /// it asserts, and a role excluded from it could not follow it.
    #[test]
    fn no_class_can_withhold_the_universal_document() {
        let mut a = agent(None);
        a.classes = vec!["judge".into(), "evidence".into(), "directive".into()];
        a.context = Some(Vec::new());
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT]);
    }

    #[test]
    fn a_document_listed_twice_is_routed_once() {
        let mut a = agent(None);
        a.context = Some(vec![
            UNIVERSAL_DOCUMENT.into(),
            BRIEF.into(),
            BRIEF.into(),
        ]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT, BRIEF]);
    }

    #[test]
    fn blank_context_entries_are_ignored() {
        let mut a = agent(None);
        a.context = Some(vec!["".into(), "  ".into(), BRIEF.into()]);
        assert_eq!(routed_documents(&a), [UNIVERSAL_DOCUMENT, BRIEF]);
    }

    /// An unknown class imposes no exclusion. Manifest validation refuses one
    /// outright, so this only ever covers a record written by an older binary —
    /// where failing open on routing is right and failing closed would blank a
    /// working role's context.
    #[test]
    fn an_unknown_class_excludes_nothing() {
        assert!(excluded_documents(&["mystery".to_string()]).is_empty());
    }
}
