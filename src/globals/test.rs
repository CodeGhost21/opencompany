//! The baseline is shipped data, so these tests are about the data as much as
//! the code: a global that stops parsing, a workflow that names a teammate no
//! company has, or a skill slug that leaves the shared library would each ship
//! a company missing part of its baseline without failing anything else.

use super::*;

#[test]
fn the_baseline_parses_without_faults() {
    assert!(
        faults().is_empty(),
        "the shipped baseline has faults: {:?}",
        faults()
    );
}

#[test]
fn the_baseline_is_not_empty() {
    // `build.rs` tolerates an absent `globals/` so the crate still compiles
    // without one. That tolerance is exactly what would let the directory go
    // missing unnoticed, so the assertion lives here instead.
    assert!(!agents().is_empty(), "no global agents embedded");
    assert!(!workflows().is_empty(), "no global workflows embedded");
    assert!(!skills().is_empty(), "no global skills embedded");
    assert!(!tool_baseline().is_empty(), "no global tool baseline");
}

#[test]
fn no_global_agent_orchestrates() {
    for agent in agents() {
        assert_ne!(
            agent.tier.as_deref(),
            Some(crate::company::ORCHESTRATOR_TIER),
            "global agent `{}` would take every company's orchestrator seat",
            agent.id
        );
    }
}

#[test]
fn every_global_workflow_names_a_global_agent() {
    // A global graph runs in companies whose rosters it has never seen, so an
    // agent node naming a vertical's teammate is a graph that only works where
    // it was written.
    let ids: Vec<&str> = agents().iter().map(|agent| agent.id.as_str()).collect();
    for workflow in workflows() {
        for node in &workflow.nodes {
            let Some(agent) = node.agent.as_deref() else {
                continue;
            };
            assert!(
                ids.contains(&agent),
                "global workflow `{}` node `{}` names `{agent}`, which is not a global agent ({ids:?})",
                workflow.id,
                node.id
            );
        }
    }
}

#[test]
fn the_tool_baseline_grants_nothing_that_spends_or_leaves() {
    // The floor is a floor: no company can drop below it, so it must never
    // carry a namespace that spends money or reaches outside the company.
    for namespace in tool_baseline() {
        assert!(
            !["media", "composio", "search", "web", "*"].contains(&namespace.as_str()),
            "`{namespace}` cannot be in the global tool floor — a company must be able to withhold it"
        );
    }
}

#[test]
fn has_answers_for_each_kind_and_rejects_junk() {
    let agent = format!("agent:{}", agents()[0].id);
    let workflow = format!("workflow:{}", workflows()[0].id);
    let skill = format!("skill:{}", skills()[0].slug);
    assert!(has(&agent), "{agent}");
    assert!(has(&workflow), "{workflow}");
    assert!(has(&skill), "{skill}");

    assert!(!has("agent:nobody"));
    assert!(!has("agents:researcher"), "an unknown kind is not a match");
    assert!(!has("researcher"), "an unqualified id is not a match");
}

#[test]
fn disabled_matches_only_the_named_kind_and_id() {
    let disable = vec!["agent:researcher".to_string()];
    assert!(disabled(&disable, "agent", "researcher"));
    assert!(!disabled(&disable, "workflow", "researcher"));
    assert!(!disabled(&disable, "agent", "writer"));
    assert!(!disabled(&[], "agent", "researcher"));
}
