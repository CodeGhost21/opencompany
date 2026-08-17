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
    assert!(!default_tool_allow().is_empty(), "no default tool belt");
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
fn the_default_tool_belt_is_the_wildcard_belt_without_search() {
    // The belt a company starts from, authored rather than hardcoded. `search`
    // stays out of it: it bills per call, and a company that never asked for
    // web search must never spend on it by default.
    assert_eq!(
        default_tool_allow(),
        vec!["*".to_string(), "media".to_string(), "composio".to_string()]
    );
}

#[test]
fn an_absent_default_tool_belt_falls_back_rather_than_grants_nothing() {
    // The fallback matters more than it looks: a company whose manifest has no
    // `[tools]` section takes this value, so an empty answer would ship a
    // company whose every agent has an empty tool belt.
    assert!(!default_tool_allow().is_empty());
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
