//! `Agents/<agent-id>/` — the reserved workspace folder each agent calls home
//! (issue #551).
//!
//! Before this module an agent had nowhere in the shared tree that was
//! recognisably *its own*: everything it produced landed in its private
//! per-agent sandbox or on a task artifact, neither of which the operator or
//! another agent browses. `Agents/` gives each roster member a named place in
//! the one tree both sides read, so "where did the CMO put the launch brief"
//! has an answer a human can navigate to.
//!
//! # What this is, and what it very deliberately is not
//!
//! It is an **organizational and attribution unit**, identified by path. It is
//! **not** a permission boundary. Agents write anywhere in the tree — that is
//! the settled design (a `workspace_write` has always been able to overwrite
//! any note, and gating *create* while *overwrite* stays free would protect
//! nothing, since overwriting is the strictly more destructive of the two).
//! What keeps the tree tidy is steering — the persona brief names
//! `Agents/<your id>/` as the default home for anything an agent produces —
//! plus the authorship stamps from issue #326, which make it visible after the
//! fact who put what where. Containment lives one level up, in company tenancy,
//! the explicit `workspace` write grant, the CAS token, and policy parking.
//!
//! # Fail-closed adoption
//!
//! Identity is by path, and nothing in the [`WorkspaceStore`] port enforces
//! unique sibling names, so provisioning is check-then-act. Every ambiguous
//! situation resolves the same way: **warn and skip**, never guess and never
//! overwrite.
//!
//! * Exactly one root folder named `Agents` → adopt it as-is.
//! * A root *file* named `Agents`, or several root nodes carrying the name →
//!   warn and do nothing at all. Creating a rival root would make the path
//!   permanently ambiguous, which the tool layer's resolver then refuses for
//!   every agent (see `harness::workspace_tools`).
//! * Same rule again, one level down, per `Agents/<agent-id>`.
//!
//! The whole function is idempotent, which is what lets it run on every boot
//! and on every harness roster rebuild without accumulating anything.

use std::collections::BTreeSet;

use crate::Result;
use crate::ports::types::CompanyId;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

/// The reserved root folder holding one subfolder per agent.
///
/// A literal, because identity here is by path: this is the name the persona
/// brief tells agents to look for and the name issue #552's published
/// deliverables land under.
pub const AGENTS_ROOT: &str = "Agents";

/// Adopt-or-create `Agents/` and `Agents/<id>` for every agent in `agent_ids`.
///
/// One `tree()` read, then only the creates that are actually missing. Safe to
/// call on every boot and every roster rebuild.
///
/// The `Agents` root is stamped [`WorkspaceOrigin::Seed`] — it is scaffolding
/// the runtime lays down, authored by no operator and no agent — while each
/// `Agents/<id>` folder is stamped [`WorkspaceOrigin::Agent`] for the agent it
/// belongs to, so the console can say whose folder it is without parsing the
/// path.
///
/// Errors from the tree read propagate; a failed *create* warns and moves on.
/// Provisioning a convenience folder must not take down a boot or a turn, and
/// the next call retries it.
pub async fn ensure_agent_folders<I, S>(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    agent_ids: I,
) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    // Deduplicated because the callers union a manifest roster with a live
    // overlay roster, which legitimately overlap.
    let ids: BTreeSet<String> = agent_ids
        .into_iter()
        .map(|id| id.as_ref().trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }

    let nodes = store.tree(company).await?;

    let Some(root_id) = ensure_root(store, company, &nodes).await? else {
        return Ok(());
    };

    for id in ids {
        // The id becomes a node name, and a name carrying a separator renders
        // an ambiguous or traversal-shaped path. The `fs` backend refuses such
        // names outright and the sqlite/mongodb backends do not, so the guard
        // lives here rather than being assumed of the store.
        if !is_legal_segment(&id) {
            tracing::warn!(
                company = %company,
                agent = %id,
                "[workspace] agent id is not a legal path segment; skipping its `{AGENTS_ROOT}/` folder"
            );
            continue;
        }

        let existing: Vec<&WorkspaceNode> = nodes
            .iter()
            .filter(|n| n.parent_id.as_deref() == Some(root_id.as_str()) && n.name == id)
            .collect();
        match existing.as_slice() {
            // Already provisioned (or the operator made it by hand) — adopt.
            [one] if one.kind == NodeKind::Folder => continue,
            [_] => {
                tracing::warn!(
                    company = %company,
                    agent = %id,
                    "[workspace] `{AGENTS_ROOT}/{id}` already exists as a file, not a folder; leaving it alone"
                );
                continue;
            }
            [] => {}
            many => {
                tracing::warn!(
                    company = %company,
                    agent = %id,
                    count = many.len(),
                    "[workspace] several nodes are named `{AGENTS_ROOT}/{id}`; leaving them alone"
                );
                continue;
            }
        }

        let origin = WorkspaceOrigin::Agent { id: id.clone() };
        let node = WorkspaceNode {
            id: crate::ports::generate_id(),
            name: id.clone(),
            kind: NodeKind::Folder,
            parent_id: Some(root_id.clone()),
            updated_at_millis: crate::ports::now_millis(),
            created_by: origin.clone(),
            updated_by: origin,
        };
        if let Err(e) = store.create(company, &node, None).await {
            tracing::warn!(
                company = %company,
                agent = %id,
                error = %e,
                "[workspace] could not create `{AGENTS_ROOT}/{id}`; will retry on the next rebuild"
            );
        }
    }

    Ok(())
}

/// Adopt the existing `Agents` root, or create it. `None` means "the tree is
/// ambiguous here — do nothing", which is the fail-closed answer.
async fn ensure_root(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    nodes: &[WorkspaceNode],
) -> Result<Option<String>> {
    let candidates: Vec<&WorkspaceNode> = nodes
        .iter()
        .filter(|n| n.parent_id.is_none() && n.name == AGENTS_ROOT)
        .collect();

    match candidates.as_slice() {
        [one] if one.kind == NodeKind::Folder => return Ok(Some(one.id.clone())),
        [_] => {
            tracing::warn!(
                company = %company,
                "[workspace] the workspace root already holds a FILE named `{AGENTS_ROOT}`; not provisioning agent folders"
            );
            return Ok(None);
        }
        [] => {}
        many => {
            tracing::warn!(
                company = %company,
                count = many.len(),
                "[workspace] several root nodes are named `{AGENTS_ROOT}`; not provisioning agent folders"
            );
            return Ok(None);
        }
    }

    let root = WorkspaceNode {
        id: crate::ports::generate_id(),
        name: AGENTS_ROOT.to_string(),
        kind: NodeKind::Folder,
        parent_id: None,
        updated_at_millis: crate::ports::now_millis(),
        // Runtime scaffolding, not anybody's writing.
        created_by: WorkspaceOrigin::Seed,
        updated_by: WorkspaceOrigin::Seed,
    };
    match store.create(company, &root, None).await {
        Ok(()) => Ok(Some(root.id)),
        Err(e) => {
            tracing::warn!(
                company = %company,
                error = %e,
                "[workspace] could not create the `{AGENTS_ROOT}` root; will retry on the next rebuild"
            );
            Ok(None)
        }
    }
}

/// Whether `name` is usable as a single workspace path segment.
///
/// Mirrors the `fs` backend's `reject_unsafe_name` and the agent tool layer's
/// `is_legal_segment`. Duplicated rather than shared because this module is in
/// the default build and the tool layer links only under the `openhuman`
/// feature; the rule is three lines and a shared home for it would drag the
/// whole harness into every build.
fn is_legal_segment(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::store::FsOps;

    fn agent(id: &str) -> WorkspaceOrigin {
        WorkspaceOrigin::Agent { id: id.to_string() }
    }

    async fn store() -> (tempfile::TempDir, Arc<dyn WorkspaceStore>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ops: Arc<dyn WorkspaceStore> = Arc::new(FsOps::new(dir.path()));
        (dir, ops)
    }

    /// A node's rendered `parent/child` path, for readable assertions.
    fn path_of(nodes: &[WorkspaceNode], node: &WorkspaceNode) -> String {
        match &node.parent_id {
            None => node.name.clone(),
            Some(parent) => match nodes.iter().find(|n| &n.id == parent) {
                Some(p) => format!("{}/{}", path_of(nodes, p), node.name),
                None => node.name.clone(),
            },
        }
    }

    fn paths(nodes: &[WorkspaceNode]) -> Vec<String> {
        let mut out: Vec<String> = nodes.iter().map(|n| path_of(nodes, n)).collect();
        out.sort();
        out
    }

    #[tokio::test]
    async fn it_provisions_a_root_and_one_folder_per_agent() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        ensure_agent_folders(ws.as_ref(), &company, ["ceo", "cmo"])
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Agents/ceo", "Agents/cmo"]);

        // The root is runtime scaffolding; each folder belongs to its agent.
        let root = nodes.iter().find(|n| n.name == AGENTS_ROOT).unwrap();
        assert_eq!(root.created_by, WorkspaceOrigin::Seed);
        assert_eq!(root.kind, NodeKind::Folder);
        let ceo = nodes.iter().find(|n| n.name == "ceo").unwrap();
        assert_eq!(ceo.created_by, agent("ceo"));
        assert_eq!(ceo.kind, NodeKind::Folder);
    }

    /// The property that lets this run on every boot and every roster rebuild.
    #[tokio::test]
    async fn it_is_idempotent() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        for _ in 0..3 {
            ensure_agent_folders(ws.as_ref(), &company, ["ceo"])
                .await
                .unwrap();
        }
        assert_eq!(
            paths(&ws.tree(&company).await.unwrap()),
            ["Agents", "Agents/ceo"]
        );
    }

    /// A roster that grows between boots gets the new folder and keeps the old
    /// ones — the `add_member` / `add_agent` case the harness call site covers.
    #[tokio::test]
    async fn a_later_agent_is_added_without_disturbing_the_first() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        ensure_agent_folders(ws.as_ref(), &company, ["ceo"])
            .await
            .unwrap();
        let before = ws.tree(&company).await.unwrap();
        let ceo_id = before.iter().find(|n| n.name == "ceo").unwrap().id.clone();

        ensure_agent_folders(ws.as_ref(), &company, ["ceo", "designer"])
            .await
            .unwrap();

        let after = ws.tree(&company).await.unwrap();
        assert_eq!(
            paths(&after),
            vec!["Agents", "Agents/ceo", "Agents/designer"]
        );
        assert_eq!(
            after.iter().find(|n| n.name == "ceo").unwrap().id,
            ceo_id,
            "the existing folder must be adopted, not replaced"
        );
    }

    /// An operator-made `Agents/` folder is adopted as-is rather than
    /// duplicated — identity is by path, so a second root would make every
    /// `Agents/...` path permanently ambiguous.
    #[tokio::test]
    async fn an_existing_agents_folder_is_adopted() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        let existing = WorkspaceNode {
            id: "hand-made".to_string(),
            name: AGENTS_ROOT.to_string(),
            kind: NodeKind::Folder,
            parent_id: None,
            updated_at_millis: 1,
            created_by: WorkspaceOrigin::Operator,
            updated_by: WorkspaceOrigin::Operator,
        };
        ws.create(&company, &existing, None).await.unwrap();

        ensure_agent_folders(ws.as_ref(), &company, ["ceo"])
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Agents/ceo"]);
        let root = nodes.iter().find(|n| n.name == AGENTS_ROOT).unwrap();
        assert_eq!(root.id, "hand-made", "the operator's folder must be reused");
        assert_eq!(
            root.created_by,
            WorkspaceOrigin::Operator,
            "adoption must not rewrite the operator's authorship"
        );
    }

    /// Fail-closed: a root *file* named `Agents` is a collision this module has
    /// no honest way to resolve, so it does nothing rather than shadowing the
    /// operator's note with a rival folder of the same name.
    #[tokio::test]
    async fn a_root_file_named_agents_blocks_provisioning() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ws.create(
            &company,
            &WorkspaceNode {
                id: "note".to_string(),
                name: AGENTS_ROOT.to_string(),
                kind: NodeKind::File,
                parent_id: None,
                updated_at_millis: 1,
                created_by: WorkspaceOrigin::Operator,
                updated_by: WorkspaceOrigin::Operator,
            },
            Some("# not a folder"),
        )
        .await
        .unwrap();

        ensure_agent_folders(ws.as_ref(), &company, ["ceo"])
            .await
            .unwrap();

        assert_eq!(
            paths(&ws.tree(&company).await.unwrap()),
            vec!["Agents"],
            "nothing may be created alongside the colliding root"
        );
    }

    /// Same rule one level down: an `Agents/ceo` *file* is left alone.
    #[tokio::test]
    async fn a_colliding_child_file_is_left_alone() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_agent_folders(ws.as_ref(), &company, ["cmo"])
            .await
            .unwrap();
        let root_id = ws
            .tree(&company)
            .await
            .unwrap()
            .into_iter()
            .find(|n| n.name == AGENTS_ROOT)
            .unwrap()
            .id;
        ws.create(
            &company,
            &WorkspaceNode {
                id: "ceo-note".to_string(),
                name: "ceo".to_string(),
                kind: NodeKind::File,
                parent_id: Some(root_id),
                updated_at_millis: 1,
                created_by: WorkspaceOrigin::Operator,
                updated_by: WorkspaceOrigin::Operator,
            },
            Some("# notes about the ceo"),
        )
        .await
        .unwrap();

        ensure_agent_folders(ws.as_ref(), &company, ["ceo"])
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Agents/ceo", "Agents/cmo"]);
        assert_eq!(
            nodes.iter().find(|n| n.name == "ceo").unwrap().kind,
            NodeKind::File,
            "the operator's note must not be shadowed by a folder of the same name"
        );
    }

    /// An id that is not a legal path segment would render an unaddressable or
    /// traversal-shaped path, so it is skipped rather than created.
    #[tokio::test]
    async fn an_illegal_agent_id_is_skipped_without_blocking_the_others() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        ensure_agent_folders(ws.as_ref(), &company, ["../escape", "ok", "", "."])
            .await
            .unwrap();

        assert_eq!(
            paths(&ws.tree(&company).await.unwrap()),
            vec!["Agents", "Agents/ok"]
        );
    }

    /// No roster, no scaffolding: an empty id list must not mint a stray
    /// `Agents/` folder in a workspace that has no agents to put in it.
    #[tokio::test]
    async fn an_empty_roster_creates_nothing() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_agent_folders(ws.as_ref(), &company, Vec::<String>::new())
            .await
            .unwrap();
        assert!(ws.is_empty(&company).await.unwrap());
    }

    /// The tree is company-scoped: provisioning one company leaves another's
    /// workspace untouched.
    #[tokio::test]
    async fn provisioning_is_per_company() {
        let (_dir, ws) = store().await;
        let acme = CompanyId::new("acme");
        let other = CompanyId::new("other");

        ensure_agent_folders(ws.as_ref(), &acme, ["ceo"])
            .await
            .unwrap();

        assert!(ws.is_empty(&other).await.unwrap());
    }
}
