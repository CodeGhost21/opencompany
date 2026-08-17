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

// ---------------------------------------------------------------------------
// The merge: what a company actually ends up with.
// ---------------------------------------------------------------------------

use crate::company::CompanyManifest;

fn manifest(toml_src: &str) -> CompanyManifest {
    toml::from_str(toml_src).expect("the fixture manifest parses")
}

fn ids(manifest: &CompanyManifest) -> Vec<&str> {
    manifest.agents.iter().map(|a| a.id.as_str()).collect()
}

#[test]
fn globals_land_after_the_company_roster_and_are_marked() {
    let mut m = manifest("[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n");
    m.apply_globals();

    assert_eq!(ids(&m)[0], "ceo", "the company's own roster comes first");
    for global in agents() {
        let merged = m
            .agents
            .iter()
            .find(|agent| agent.id == global.id)
            .expect("every global is merged in");
        assert!(merged.global, "a merged global must be marked as one");
    }
    // Which teammate orchestrates is still the company's decision.
    assert_eq!(crate::company::orchestrator_id(&m.agents), Some("ceo"));
}

#[test]
fn a_company_agent_supersedes_the_global_of_the_same_id() {
    let taken = &agents()[0].id;
    let mut m = manifest(&format!(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"{taken}\"\nrole = \"Ours\"\n"
    ));
    m.apply_globals();

    let matching: Vec<&crate::company::Agent> =
        m.agents.iter().filter(|a| &a.id == taken).collect();
    assert_eq!(matching.len(), 1, "no duplicate id");
    assert_eq!(matching[0].role, "Ours", "the company's definition wins");
    assert!(!matching[0].global);
}

#[test]
fn disable_drops_one_global_and_leaves_the_rest() {
    let dropped = &agents()[0].id;
    let mut m = manifest(&format!(
        "[company]\nname = \"X\"\n\n[globals]\ndisable = [\"agent:{dropped}\"]\n"
    ));
    m.apply_globals();

    assert!(!ids(&m).contains(&dropped.as_str()));
    assert_eq!(m.agents.len(), agents().len() - 1);
}

#[test]
fn applying_the_baseline_twice_changes_nothing() {
    // The property the whole `global` marker exists for: a stored manifest is
    // serialized back out with the merged roster in it and re-merged on the
    // next load, so a second pass must not duplicate anyone.
    let mut m = manifest("[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n");
    m.apply_globals();
    let once = ids(&m).join(",");
    m.apply_globals();
    assert_eq!(ids(&m).join(","), once);
}

#[test]
fn a_disable_written_later_removes_an_already_merged_global() {
    // The upgrade path: the company was stored with the baseline merged in, and
    // its manifest gains an opt-out afterwards.
    let dropped = agents()[0].id.clone();
    let mut m = manifest("[company]\nname = \"X\"\n");
    m.apply_globals();
    assert!(ids(&m).contains(&dropped.as_str()));

    m.globals.disable = vec![format!("agent:{dropped}")];
    m.apply_globals();
    assert!(!ids(&m).contains(&dropped.as_str()));
}

#[test]
fn a_stored_manifest_picks_up_the_baseline_on_load() {
    // A company provisioned before the baseline existed carries no globals in
    // its stored TOML; the store read path is where it gains them.
    let m = CompanyManifest::from_stored_toml(
        "[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
    )
    .expect("stored manifest parses");
    assert_eq!(m.agents.len(), 1 + agents().len());
}

#[test]
fn a_serialized_manifest_marks_globals_and_keeps_the_company_roster_plain() {
    let mut m = manifest("[company]\nname = \"X\"\n\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n");
    m.apply_globals();
    let out = toml::to_string(&m).expect("serializes");
    // `global = false` is skipped, so a company's own teammate round-trips
    // exactly as its author wrote it.
    assert_eq!(
        out.matches("global = true").count(),
        agents().len(),
        "{out}"
    );
    assert!(!out.contains("global = false"), "{out}");
}

#[test]
fn a_disable_entry_naming_nothing_is_a_validation_error() {
    let m = manifest("[company]\nname = \"X\"\n\n[globals]\ndisable = [\"agent:nobody\"]\n");
    let problems = m.validate();
    assert!(
        problems.iter().any(|p| p.contains("names no global")),
        "{problems:?}"
    );

    let m = manifest("[company]\nname = \"X\"\n\n[globals]\ndisable = [\"agents:researcher\"]\n");
    assert!(
        m.validate().iter().any(|p| p.contains("unknown kind")),
        "an unknown kind is refused rather than ignored"
    );

    let m = manifest("[company]\nname = \"X\"\n\n[globals]\ndisable = [\"researcher\"]\n");
    assert!(
        m.validate().iter().any(|p| p.contains("missing its kind")),
        "an unqualified id is refused"
    );
}

#[test]
fn a_valid_disable_entry_passes_validation() {
    let m = manifest(&format!(
        "[company]\nname = \"X\"\n\n[globals]\ndisable = [\"agent:{}\", \"workflow:{}\", \"skill:{}\"]\n",
        agents()[0].id,
        workflows()[0].id,
        skills()[0].slug,
    ));
    assert!(m.validate().is_empty(), "{:?}", m.validate());
}

#[test]
fn global_workflows_list_last_and_a_company_graph_of_the_same_id_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    let taken = &workflows()[0].id;
    std::fs::create_dir_all(dir.path().join("workflows")).expect("workflows dir");
    std::fs::write(
        dir.path().join("workflows").join(format!("{taken}.toml")),
        format!(
            "id = \"{taken}\"\nname = \"Ours\"\n\n[[node]]\nid = \"t\"\nkind = \"trigger\"\nname = \"T\"\n"
        ),
    )
    .expect("seed graph");

    let listed = crate::company::list_workflows_with_globals(Some(dir.path()), &[], &[]);
    let ours = listed
        .iter()
        .find(|file| &file.id == taken)
        .expect("the id resolves");
    assert_eq!(ours.name, "Ours", "the company's own graph wins");
    assert!(!ours.global);
    assert_eq!(
        listed.len(),
        workflows().len(),
        "the shadowed global is not listed twice"
    );

    let loaded = crate::company::load_workflow_with_globals(Some(dir.path()), &[], &[], taken)
        .expect("loads")
        .expect("found");
    assert_eq!(loaded.name, "Ours", "the loader agrees with the listing");
}

#[test]
fn a_disabled_global_workflow_neither_lists_nor_loads() {
    let dropped = &workflows()[0].id;
    let disable = vec![format!("workflow:{dropped}")];

    let listed = crate::company::list_workflows_with_globals(None, &[], &disable);
    assert!(listed.iter().all(|file| &file.id != dropped));

    let loaded =
        crate::company::load_workflow_with_globals(None, &[], &disable, dropped).expect("loads");
    assert!(
        loaded.is_none(),
        "a disabled global must not be runnable by naming it directly"
    );
}
