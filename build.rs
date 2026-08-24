//! Embeds every shipped company bundle's `agents/` directory into the binary.
//!
//! The roster moved out of `company.toml` and into `agents/*.toml`. The on-disk
//! path followed it, but a `DesktopPreset` carries only an `include_str!`'d
//! `company.toml`, and `include_str!` cannot glob a directory — so the embedded
//! path silently lost every teammate, and first-run seeded an empty company.
//!
//! Generating the table here rather than hand-listing 135 files in a macro is
//! the point: a teammate added to a bundle is embedded because it exists, not
//! because someone remembered to add a line. A forgotten entry would not fail
//! the build, it would ship a company missing one agent.
//!
//! Both `*.toml` rosters and the documents `prompt_files` names are embedded,
//! because `agent_file::resolve_prompt_files` reads those bodies at parse time.
//!
//! The same script embeds the **global baseline** — `globals/` plus the shared
//! `skills/` library — for a second reason: a platform-provisioned container has
//! no repository checkout beside it, so anything only readable from disk is
//! simply absent there. A baseline that every company gets except the hosted
//! ones is not a baseline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let companies = root.join("companies");

    embed_globals(&root);

    // Re-run when a bundle changes. Watching `companies/` alone is not enough:
    // cargo does not walk into it, so a new agent file inside an existing
    // bundle would not invalidate the build.
    println!("cargo:rerun-if-changed={}", companies.display());

    let mut bundles: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

    let Ok(entries) = std::fs::read_dir(&companies) else {
        // No `companies/` at all is not this script's business to fail on:
        // the crate still compiles, and the desktop tests are what assert the
        // catalog is present.
        write_table(&bundles);
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let bundle = entry.path();
        if !bundle.is_dir() {
            continue;
        }
        let Some(id) = bundle.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let agents_dir = bundle.join("agents");
        if !agents_dir.is_dir() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", agents_dir.display());

        let mut files = Vec::new();
        collect(&agents_dir, &agents_dir, &mut files);
        // Sorted by path, matching `agent_file::agent_file_paths`, because the
        // roster's order decides which teammate orchestrates when nobody is
        // tagged.
        files.sort_by(|a, b| a.0.cmp(&b.0));
        if !files.is_empty() {
            bundles.insert(id.to_string(), files);
        }
    }

    write_table(&bundles);
}

/// Every file under `agents/`, keyed by its path relative to that directory.
///
/// Recursive because `prompt_files` entries live in subdirectories beside the
/// agent that names them; the consumer is what restricts *roster* parsing to
/// the immediate directory.
fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, out);
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        // Forward slashes regardless of host, so the embedded keys match the
        // `prompt_files` entries authors write.
        let key = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/");
        match std::fs::read_to_string(&path) {
            Ok(body) => out.push((key, body)),
            // A non-UTF-8 file under `agents/` is not a roster file and not a
            // prompt document. Skipping it keeps a stray binary from failing
            // every build.
            Err(_) => continue,
        }
    }
}

/// Embeds `globals/` and the shared `skills/` library into `embedded_globals.rs`.
///
/// Absent directories are not this script's business to fail on, exactly as with
/// `companies/`: the crate still compiles, and `crate::globals` is what asserts
/// the baseline is non-empty.
fn embed_globals(root: &Path) {
    let globals = root.join("globals");
    let skills = root.join("skills");
    println!("cargo:rerun-if-changed={}", globals.display());
    println!("cargo:rerun-if-changed={}", skills.display());
    for sub in ["agents", "workflows", "ledgers"] {
        println!("cargo:rerun-if-changed={}", globals.join(sub).display());
    }

    let manifest = std::fs::read_to_string(globals.join("globals.toml")).unwrap_or_default();
    // The baseline's seed board cards. A file at the `globals/` root, so the
    // watch above already covers it — no `rerun-if-changed` of its own needed.
    let tasks = std::fs::read_to_string(globals.join("tasks.toml")).unwrap_or_default();

    let mut agents = Vec::new();
    collect(
        &globals.join("agents"),
        &globals.join("agents"),
        &mut agents,
    );
    agents.sort_by(|a, b| a.0.cmp(&b.0));

    let mut workflows = Vec::new();
    collect(
        &globals.join("workflows"),
        &globals.join("workflows"),
        &mut workflows,
    );
    workflows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut ledgers = Vec::new();
    collect(
        &globals.join("ledgers"),
        &globals.join("ledgers"),
        &mut ledgers,
    );
    ledgers.sort_by(|a, b| a.0.cmp(&b.0));

    // Every shared skill, keyed by slug: which of them the baseline *installs*
    // is `[skills].always`, read at runtime rather than here, so this script
    // never has to parse TOML to decide what to embed.
    let mut skill_docs: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&skills) {
        for entry in entries.filter_map(Result::ok) {
            let dir = entry.path();
            let Some(slug) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            println!("cargo:rerun-if-changed={}", dir.display());
            if let Ok(body) = std::fs::read_to_string(dir.join("SKILL.md")) {
                skill_docs.push((slug.to_string(), body));
            }
        }
    }
    skill_docs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("// @generated by build.rs — do not edit.\n");
    out.push_str(&format!(
        "/// `globals/globals.toml`, verbatim (empty when the directory is absent).\n\
         pub static EMBEDDED_GLOBALS_MANIFEST: &str = {manifest:?};\n"
    ));
    out.push_str(&format!(
        "/// `globals/tasks.toml`, verbatim (empty when the file is absent).\n\
         pub static EMBEDDED_GLOBAL_TASKS: &str = {tasks:?};\n"
    ));
    write_pairs(
        &mut out,
        "EMBEDDED_GLOBAL_AGENTS",
        "Every file under `globals/agents/`, keyed by path relative to it, sorted.",
        &agents,
    );
    write_pairs(
        &mut out,
        "EMBEDDED_GLOBAL_WORKFLOWS",
        "Every file under `globals/workflows/`, keyed by path relative to it, sorted.",
        &workflows,
    );
    write_pairs(
        &mut out,
        "EMBEDDED_GLOBAL_LEDGERS",
        "Every file under `globals/ledgers/`, keyed by path relative to it, sorted.",
        &ledgers,
    );
    write_pairs(
        &mut out,
        "EMBEDDED_SHARED_SKILLS",
        "Every shared-library `SKILL.md`, keyed by skill slug, sorted.",
        &skill_docs,
    );

    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded_globals.rs");
    std::fs::write(dest, out).unwrap();
}

fn write_pairs(out: &mut String, name: &str, doc: &str, pairs: &[(String, String)]) {
    out.push_str(&format!(
        "/// {doc}\npub static {name}: &[(&str, &str)] = &[\n"
    ));
    for (key, body) in pairs {
        out.push_str(&format!("    ({key:?}, {body:?}),\n"));
    }
    out.push_str("];\n");
}

fn write_table(bundles: &BTreeMap<String, Vec<(String, String)>>) {
    let mut out = String::from(
        "// @generated by build.rs — do not edit.\n\
         /// Every shipped bundle's `agents/` directory, keyed by company id.\n\
         /// Inner entries are paths relative to `agents/`, sorted.\n\
         pub static EMBEDDED_AGENT_BUNDLES: &[(&str, &[(&str, &str)])] = &[\n",
    );
    for (id, files) in bundles {
        out.push_str(&format!("    ({id:?}, &[\n"));
        for (name, body) in files {
            out.push_str(&format!("        ({name:?}, {body:?}),\n"));
        }
        out.push_str("    ]),\n");
    }
    out.push_str("];\n");

    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded_agents.rs");
    std::fs::write(dest, out).unwrap();
}
