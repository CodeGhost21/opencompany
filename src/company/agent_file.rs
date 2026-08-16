//! Agent definition files: `companies/<name>/agents/<id>.toml`.
//!
//! A company's roster may be written either inline in `company.toml` as
//! `[[agent]]` entries or — the richer form — as one file per teammate under an
//! `agents/` directory. The two are mutually exclusive per company: a bundle
//! that has both is a validation error rather than a silent precedence rule,
//! because a roster block the operator wrote and the runtime ignored is exactly
//! the failure this crate's manifest validation exists to prevent.
//!
//! The per-file form exists because a teammate is more than four fields once it
//! carries a custom prompt and its own briefing documents. Those do not fit
//! comfortably in an array-of-tables — a multi-line TOML string inside
//! `[[agent]]` is unreadable at roster length, and prose belongs beside the
//! agent it configures.
//!
//! This module parses those files, resolves each agent's
//! [`prompt_files`](crate::company::Agent::prompt_files) against the bundle, and
//! reports every problem at once in prosumer language — matching
//! [`super::manifest`] and [`super::workflow_file`].

use std::path::{Path, PathBuf};

use crate::company::Agent;
use crate::error::{OpenCompanyError, Result};

/// The bundle subdirectory holding one TOML file per roster teammate.
pub const AGENTS_DIR: &str = "agents";

/// Whether `dir` is a company bundle whose roster lives in `agents/*.toml`.
///
/// A present-but-empty `agents/` directory is **not** a bundle roster: it
/// carries no teammates, so treating it as authoritative would blank the roster
/// of a company whose `company.toml` still has a perfectly good one. An
/// unreadable directory answers `false` for the same reason — the caller then
/// parses `[[agent]]` as it always did, rather than failing the whole company
/// over a directory nothing has asked to read yet.
pub fn has_agent_files(dir: &Path) -> bool {
    !agent_file_paths(&dir.join(AGENTS_DIR)).is_empty()
}

/// Every `.toml` file directly inside `agents/`, sorted by file stem.
///
/// Sorted, not directory order, because the roster's order is load-bearing:
/// [`orchestrator_id`](crate::company::orchestrator_id) falls back to "the first
/// agent declared" when nobody is tagged `tier = "orchestrator"`, and readdir
/// order varies by filesystem. An unsorted read would make which teammate runs
/// the company depend on which machine parsed the bundle.
///
/// Only the immediate directory is read. Subdirectories are for the documents
/// `prompt_files` names, so descending into them would try to parse a `.toml`
/// briefing as a teammate.
fn agent_file_paths(agents_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    paths
}

/// The on-disk shape of one `agents/<id>.toml`.
///
/// Every field mirrors [`Agent`], because that is what this parses into: the
/// roster type does not fork by authoring format, so a field added for the
/// bundle form is immediately available to `[[agent]]` too, and there is exactly
/// one validator and one consumer for it.
///
/// `id` is optional here and comes from the filename. It is accepted in the body
/// only as a cross-check — see [`parse_agent_file`].
#[derive(Debug, serde::Deserialize)]
struct AgentFile {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    delegates_to: Vec<String>,
    #[serde(default)]
    context: Option<Vec<String>>,
    #[serde(default)]
    budget_usd_daily: Option<f64>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    prompt_files: Vec<String>,
    #[serde(default)]
    classes: Vec<String>,
}

/// Loads every agent definition under `<dir>/agents/`, in roster order.
///
/// Field-level validity (tier names, grant shapes, `delegates_to` targets) is
/// **not** checked here: those rules are cross-cutting — a `delegates_to` entry
/// has to name a desk declared in `company.toml` — so they belong to
/// [`CompanyManifest::validate`](crate::company::CompanyManifest::validate),
/// which sees the whole company. This function is responsible only for what it
/// alone can see: that each file parses, that its identity is coherent with its
/// filename, and that the documents it names exist and can be read.
pub fn load_agents(dir: &Path) -> Result<Vec<Agent>> {
    let agents_dir = dir.join(AGENTS_DIR);
    let mut agents = Vec::new();
    let mut problems = Vec::new();

    for path in agent_file_paths(&agents_dir) {
        match parse_agent_file(&agents_dir, &path) {
            Ok(agent) => agents.push(agent),
            Err(mut file_problems) => problems.append(&mut file_problems),
        }
    }

    // Duplicate ids cannot arise from distinct filenames, but an `id` key that
    // disagrees with its stem is rejected above, so by here every id *is* its
    // stem and uniqueness is a property of the filesystem. Nothing to check.

    if problems.is_empty() {
        Ok(agents)
    } else {
        Err(OpenCompanyError::ManifestInvalid {
            path: agents_dir,
            problems,
        })
    }
}

/// Parses one agent file, returning every problem it has rather than the first.
fn parse_agent_file(agents_dir: &Path, path: &Path) -> std::result::Result<Agent, Vec<String>> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    let label = format!("agent file `{AGENTS_DIR}/{stem}.toml`");

    let text = std::fs::read_to_string(path)
        .map_err(|err| vec![format!("{label} could not be read — {err}.")])?;
    let file: AgentFile = toml::from_str(&text)
        .map_err(|err| vec![format!("{label} is not valid TOML — {}.", err.message())])?;

    let mut problems = Vec::new();

    if !super::manifest::is_snake_case(&stem) {
        problems.push(format!(
            "{label} has an invalid filename — the file name is the agent's id, so use snake_case (lowercase letters, digits, and underscores, starting with a letter)."
        ));
    }

    // An `id` key is redundant with the filename but harmless to write, so it is
    // accepted when it agrees and rejected when it does not. Silently preferring
    // one over the other would leave an operator renaming a file and wondering
    // why nothing changed — or renaming the key and wondering the same.
    if let Some(declared) = file.id.as_deref()
        && declared != stem
    {
        problems.push(format!(
            "{label} declares `id = \"{declared}\"` but its filename says `{stem}` — the filename is the id, so rename the file or drop the `id` key."
        ));
    }

    let role = file.role.unwrap_or_default();
    if role.trim().is_empty() {
        problems.push(format!("{label} is missing a `role`."));
    }

    let prompt_files_resolved = match resolve_prompt_files(agents_dir, &label, &file.prompt_files) {
        Ok(resolved) => resolved,
        Err(mut file_problems) => {
            problems.append(&mut file_problems);
            Vec::new()
        }
    };

    if !problems.is_empty() {
        return Err(problems);
    }

    Ok(Agent {
        id: stem,
        role,
        description: file.description,
        tier: file.tier,
        tools: file.tools,
        delegates_to: file.delegates_to,
        context: file.context,
        budget_usd_daily: file.budget_usd_daily,
        prompt: file.prompt,
        prompt_files: file.prompt_files,
        prompt_files_resolved,
        classes: file.classes,
    })
}

/// Reads each `prompt_files` entry relative to `agents/`, refusing any path that
/// leaves that directory.
///
/// The traversal check is done on the **path components**, before touching the
/// filesystem, rather than by canonicalizing and comparing prefixes. Canonical
/// comparison would resolve symlinks, which makes whether a bundle is valid
/// depend on how the checkout was laid out on the reading machine; a company
/// that parses on one host must parse on every host. Absolute paths and `..`
/// are both rejected outright — an agent's briefing lives beside the agent.
fn resolve_prompt_files(
    agents_dir: &Path,
    label: &str,
    entries: &[String],
) -> std::result::Result<Vec<(String, String)>, Vec<String>> {
    let mut resolved = Vec::new();
    let mut problems = Vec::new();

    for entry in entries {
        let rel = Path::new(entry);
        let escapes = rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));
        if escapes {
            problems.push(format!(
                "{label} names `prompt_files` entry `{entry}`, which points outside `{AGENTS_DIR}/` — a prompt document must live beside the agent that uses it."
            ));
            continue;
        }

        let path = agents_dir.join(rel);
        match std::fs::read_to_string(&path) {
            Ok(body) => resolved.push((entry.clone(), body)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => problems.push(format!(
                "{label} names `prompt_files` entry `{entry}`, which does not exist — create `{AGENTS_DIR}/{entry}` or remove the entry."
            )),
            Err(err) => problems.push(format!(
                "{label} could not read `prompt_files` entry `{entry}` — {err}."
            )),
        }
    }

    if problems.is_empty() {
        Ok(resolved)
    } else {
        Err(problems)
    }
}
