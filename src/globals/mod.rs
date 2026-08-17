//! The global baseline: what every company has, whichever vertical it started
//! from.
//!
//! A `companies/<name>/` bundle describes one vertical. This module is the part
//! that is the same in all of them — a small roster, two workflow graphs, a few
//! installed skills, and the tool namespaces below which no company may drop.
//! The authored sources live in `globals/` beside `companies/`, and the
//! contract is `docs/spec/runtime/globals.md`.
//!
//! Everything here is **embedded at build time**, not read from disk. A
//! platform-provisioned tenant container carries no repository checkout — the
//! same reason `skills_root()` is `None` there and the shared skill registry is
//! empty — so a baseline resolved from the filesystem would be a baseline every
//! hosted company silently lacked. `build.rs` generates the tables; this module
//! parses them once and caches the result.
//!
//! Malformed input is a fault, never a panic and never an abort: a company must
//! still reach the rest of its baseline when one global is broken, exactly as
//! [`crate::ledger::registry::builtins`] treats a malformed built-in ledger.
//!
//! # Precedence
//!
//! A company always wins. Every merge point resolves an id collision toward the
//! company's own definition and never merges field-by-field:
//!
//! * roster — [`crate::company::CompanyManifest`] appends globals *after* the
//!   company's own agents, skipping any id the company already declares;
//! * workflows — the company's `workflows/<id>.toml`, then its console-authored
//!   overlay, then the global graph;
//! * skills — a company-bundle or console-authored skill of the same slug
//!   supersedes the installed global one;
//! * tools — `[tools].baseline` is unioned into `[tools].allow`, which is a
//!   floor and cannot narrow what a company granted.
//!
//! A company drops a global outright with `[globals].disable`, whose entries are
//! `<kind>:<id>` keys validated against what this module actually carries.

use std::sync::OnceLock;

use crate::company::{Agent, SkillDoc, WorkflowFile};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/embedded_globals.rs"));
}

#[cfg(test)]
mod test;

/// The `<kind>` half of a `[globals].disable` entry.
///
/// Closed, and each variant maps to exactly one surface below. A free-form kind
/// would let a typo (`agents:researcher`) read as a disable that never fires,
/// which is the one failure mode an opt-out list must not have.
pub const DISABLE_KINDS: &[&str] = &["agent", "workflow", "skill"];

/// The parsed `globals/globals.toml`.
#[derive(Debug, Default, serde::Deserialize)]
struct GlobalsManifest {
    #[serde(default)]
    tools: GlobalTools,
    #[serde(default)]
    skills: GlobalSkills,
}

#[derive(Debug, Default, serde::Deserialize)]
struct GlobalTools {
    #[serde(default)]
    baseline: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct GlobalSkills {
    #[serde(default)]
    always: Vec<String>,
}

/// Everything the baseline carries, parsed once.
#[derive(Debug, Default)]
struct Baseline {
    agents: Vec<Agent>,
    workflows: Vec<WorkflowFile>,
    skills: Vec<SkillDoc>,
    tool_baseline: Vec<String>,
    faults: Vec<String>,
}

fn baseline() -> &'static Baseline {
    static BASELINE: OnceLock<Baseline> = OnceLock::new();
    BASELINE.get_or_init(build)
}

fn build() -> Baseline {
    let mut faults = Vec::new();

    let manifest: GlobalsManifest = match toml::from_str(generated::EMBEDDED_GLOBALS_MANIFEST) {
        Ok(manifest) => manifest,
        Err(err) => {
            faults.push(format!("`globals/globals.toml` is not valid TOML — {err}"));
            GlobalsManifest::default()
        }
    };

    Baseline {
        agents: build_agents(&mut faults),
        workflows: build_workflows(&mut faults),
        skills: build_skills(&manifest.skills.always, &mut faults),
        tool_baseline: manifest.tools.baseline,
        faults,
    }
}

fn build_agents(faults: &mut Vec<String>) -> Vec<Agent> {
    let files = generated::EMBEDDED_GLOBAL_AGENTS;
    let names = crate::company::agent_file::embedded_roster_names(files);
    match crate::company::agent_file::load_agents_from(
        // Only ever used to label a parse failure; the authored path is what a
        // reader needs, and no filesystem path exists in a packaged install.
        std::path::Path::new("globals"),
        &names,
        &|rel| {
            files
                .iter()
                .find(|(name, _)| *name == rel)
                .map(|(_, body)| (*body).to_string())
                .ok_or(std::io::ErrorKind::NotFound)
        },
    ) {
        Ok(agents) => agents
            .into_iter()
            .filter(|agent| {
                // A global tagged `orchestrator` would take the company's own
                // roster's job in every company at once, since `orchestrator_id`
                // prefers a tagged agent over declaration order. Refused here
                // rather than trusted to review.
                let tagged = agent.tier.as_deref() == Some(crate::company::ORCHESTRATOR_TIER);
                if tagged {
                    faults.push(format!(
                        "global agent `{}` is tagged `tier = \"orchestrator\"` — a global teammate cannot orchestrate a company it has never seen, so it is dropped.",
                        agent.id
                    ));
                }
                !tagged
            })
            .collect(),
        Err(err) => {
            faults.push(format!("a global agent is malformed: {err}"));
            Vec::new()
        }
    }
}

fn build_workflows(faults: &mut Vec<String>) -> Vec<WorkflowFile> {
    let mut out = Vec::new();
    for (name, body) in generated::EMBEDDED_GLOBAL_WORKFLOWS {
        if name.contains('/') || !name.ends_with(".toml") {
            continue;
        }
        match crate::company::parse_workflow(body) {
            Ok(workflow) => out.push(workflow),
            Err(err) => faults.push(format!("global workflow `{name}` is malformed: {err}")),
        }
    }
    out
}

fn build_skills(always: &[String], faults: &mut Vec<String>) -> Vec<SkillDoc> {
    let mut out = Vec::new();
    for slug in always {
        let Some((_, body)) = generated::EMBEDDED_SHARED_SKILLS
            .iter()
            .find(|(candidate, _)| candidate == slug)
        else {
            faults.push(format!(
                "`[skills].always` names `{slug}`, which is not in the shared library (`skills/{slug}/SKILL.md`)."
            ));
            continue;
        };
        match crate::company::parse_skill_md(slug, body) {
            Ok(doc) => out.push(doc),
            Err(err) => faults.push(format!("global skill `{slug}` is malformed: {err}")),
        }
    }
    out
}

/// The global roster, in filename order, never including an orchestrator.
pub fn agents() -> &'static [Agent] {
    &baseline().agents
}

/// The global workflow graphs, in filename order.
pub fn workflows() -> &'static [WorkflowFile] {
    &baseline().workflows
}

/// The shared-library skills installed in every company.
pub fn skills() -> &'static [SkillDoc] {
    &baseline().skills
}

/// The tool namespaces every company grants on top of its own `[tools].allow`.
pub fn tool_baseline() -> &'static [String] {
    &baseline().tool_baseline
}

/// Every problem found while parsing the baseline — empty in a healthy build.
///
/// Surfaced rather than swallowed so a broken global is visible in the same
/// place a broken built-in ledger is, instead of looking like a company that
/// simply has no researcher.
pub fn faults() -> &'static [String] {
    &baseline().faults
}

/// Whether `key` (`<kind>:<id>`) names a global that exists.
///
/// The validator behind `[globals].disable`: an entry that matches nothing is a
/// manifest error, because the alternative is an opt-out an operator wrote,
/// believed, and never got.
pub fn has(key: &str) -> bool {
    let Some((kind, id)) = key.split_once(':') else {
        return false;
    };
    match kind {
        "agent" => agents().iter().any(|agent| agent.id == id),
        "workflow" => workflows().iter().any(|workflow| workflow.id == id),
        "skill" => skills().iter().any(|skill| skill.slug == id),
        _ => false,
    }
}

/// Whether `disable` (the manifest's `[globals].disable`) drops `<kind>:<id>`.
pub fn disabled(disable: &[String], kind: &str, id: &str) -> bool {
    let key = format!("{kind}:{id}");
    disable.iter().any(|entry| entry == &key)
}
