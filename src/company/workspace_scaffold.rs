//! The workspace's system roots — `Agents/` and `Desks/` — and the folders
//! minted beneath them the first time somebody actually produces something
//! (issue #551).
//!
//! Before this module an agent had nowhere in the shared tree that was
//! recognisably *its own*: everything it produced landed in its private
//! per-agent sandbox or on a task artifact, neither of which the operator or
//! another agent browses. `Agents/` gives each roster member a named place in
//! the one tree both sides read, so "where did the CMO put the launch brief"
//! has an answer a human can navigate to. `Desks/` is the same idea one level
//! up, for work a desk produces rather than one teammate (issue #552 wires the
//! producer).
//!
//! # Eager roots, lazy members
//!
//! The two halves are provisioned on deliberately different schedules:
//!
//! * The **roots** are scaffolding. `Agents/` and `Desks/` are laid down on
//!   every boot by [`ensure_workspace_scaffold`], empty, whether or not the
//!   company has a roster — they are part of what a workspace *is*, the same
//!   way the template-seeded `Playbooks/` and `Standards/` are, and an operator
//!   opening the Workspace tab on a brand-new company should see the shape of
//!   the place rather than a void.
//! * A **member folder** is not scaffolding, it is a container for something.
//!   `Agents/<agent-id>/` and `Desks/<desk-id>/` are therefore minted on demand
//!   — by [`ensure_agent_folder`] / [`ensure_desk_folder`], at the moment that
//!   agent or desk first produces a task, artifact or note. An eager folder per
//!   roster member fills the tree with empty directories for teammates who have
//!   never done anything, which is noise that grows with the roster and tells
//!   the operator nothing.
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
//! unique sibling names, so every lookup here is check-then-act. Ambiguity
//! always resolves the same way: **never guess and never overwrite**.
//!
//! * Exactly one folder carrying the name → adopt it as-is, authorship and all.
//! * A *file* carrying the name, or several nodes carrying it → refuse to touch
//!   it. Creating a rival would make the path permanently ambiguous, which the
//!   tool layer's resolver then refuses for every agent (see
//!   `harness::workspace_tools`).
//!
//! How a refusal is *reported* differs by caller, because the callers differ:
//!
//! * [`ensure_workspace_scaffold`] runs at boot with nobody waiting on a
//!   result, so it warns and skips — a convenience folder must not take down a
//!   boot, and the next boot retries. A tree read that fails still propagates:
//!   that is the store being broken, not the tree being odd.
//! * [`ensure_agent_folder`] / [`ensure_desk_folder`] are called *by a producer
//!   that needs the id back*, so there is nothing honest to fail soft to. They
//!   return the collision as an error and let the caller decide.
//!
//! Every function here is idempotent, which is what lets the scaffold run on
//! every boot and a minter run on every publish without accumulating anything.

use crate::Result;
use crate::error::OpenCompanyError;
use crate::ports::types::CompanyId;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

/// The reserved root folder holding one subfolder per agent that has produced
/// something.
///
/// A literal, because identity here is by path: this is the name the persona
/// brief tells agents to look for and the name issue #552's published
/// deliverables land under.
pub const AGENTS_ROOT: &str = "Agents";

/// The reserved root folder holding one subfolder per desk that has produced
/// something.
pub const DESKS_ROOT: &str = "Desks";

/// The system roots every company's workspace gets, in creation order.
///
/// Deliberately *not* derived from the manifest: these exist because a
/// workspace has them, not because a particular company has agents or desks.
/// Public so a caller that has to tell scaffolding apart from content — the
/// re-seed tests, a future console filter — can ask rather than hard-code the
/// pair.
pub const SYSTEM_ROOTS: [&str; 2] = [AGENTS_ROOT, DESKS_ROOT];

/// Adopt-or-create the `Agents/` and `Desks/` roots for `company`.
///
/// One `tree()` read, then only the creates that are actually missing. Safe to
/// call on every boot: it depends on nothing but the company id, so an existing
/// company picks the roots up the next time it starts.
///
/// Both roots are stamped [`WorkspaceOrigin::Seed`] — they are scaffolding the
/// runtime lays down, authored by no operator and no agent. Nothing is created
/// *inside* them here; see [`ensure_agent_folder`] / [`ensure_desk_folder`].
///
/// Errors from the tree read propagate; a failed or ambiguous *create* warns
/// and moves on, and the next boot retries it.
pub async fn ensure_workspace_scaffold(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
) -> Result<()> {
    let nodes = store.tree(company).await?;

    for root in SYSTEM_ROOTS {
        // Each root is resolved independently: a colliding `Agents` note in the
        // workspace root is no reason to withhold `Desks/`.
        match find(&nodes, None, root) {
            Found::Folder(_) => {}
            Found::Collision(why) => tracing::warn!(
                company = %company,
                "[workspace] {why}; not provisioning the `{root}` root"
            ),
            Found::Free => {
                if let Err(e) =
                    create_folder(store, company, root, None, WorkspaceOrigin::Seed).await
                {
                    tracing::warn!(
                        company = %company,
                        error = %e,
                        "[workspace] could not create the `{root}` root; will retry on the next boot"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Adopt-or-create `Agents/<agent_id>/`, returning its node id.
///
/// The lazy half of the feature: call this at the moment `agent_id` first
/// produces something that needs a home, not when it joins the roster. Creates
/// the `Agents` root too if the scaffold has not run (or could not create it),
/// so one call is enough to get a usable parent id.
///
/// The folder is stamped [`WorkspaceOrigin::Agent`] for the agent it belongs
/// to, so the console can say whose folder it is without parsing the path.
///
/// Idempotent: a second call on the same agent returns the same id and creates
/// nothing.
pub async fn ensure_agent_folder(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    agent_id: &str,
) -> Result<String> {
    let agent_id = agent_id.trim();
    ensure_member_folder(
        store,
        company,
        AGENTS_ROOT,
        agent_id,
        WorkspaceOrigin::Agent {
            id: agent_id.to_string(),
        },
    )
    .await
}

/// Adopt-or-create `Desks/<desk_id>/`, returning its node id.
///
/// [`ensure_agent_folder`]'s counterpart for a desk — call it when a desk first
/// produces an artifact. Nothing in this PR calls it; issue #552's publish path
/// is the first producer.
///
/// Stamped [`WorkspaceOrigin::Seed`] rather than an author, because a desk is
/// not one: [`WorkspaceOrigin`] names the seed, the operator, or a single
/// agent, and claiming `Agent { id: <desk-id> }` would attribute the folder to
/// a teammate that does not exist. The desk's *contents* still carry the real
/// agent that wrote each of them.
pub async fn ensure_desk_folder(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    desk_id: &str,
) -> Result<String> {
    ensure_member_folder(
        store,
        company,
        DESKS_ROOT,
        desk_id.trim(),
        WorkspaceOrigin::Seed,
    )
    .await
}

/// The shared body of [`ensure_agent_folder`] and [`ensure_desk_folder`]:
/// resolve `root`, then resolve `id` beneath it, creating what is missing.
async fn ensure_member_folder(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    root: &str,
    id: &str,
    origin: WorkspaceOrigin,
) -> Result<String> {
    // The id becomes a node name, and a name carrying a separator renders an
    // ambiguous or traversal-shaped path. The `fs` backend refuses such names
    // outright and the sqlite/mongodb backends do not, so the guard lives here
    // rather than being assumed of the store.
    if !is_legal_segment(id) {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "`{id}` is not a legal workspace path segment, so it cannot name a folder under \
             `{root}/`"
        )));
    }

    let nodes = store.tree(company).await?;

    let root_id = match find(&nodes, None, root) {
        Found::Folder(id) => id,
        Found::Free => create_folder(store, company, root, None, WorkspaceOrigin::Seed).await?,
        Found::Collision(why) => return Err(OpenCompanyError::Conflict(why)),
    };

    match find(&nodes, Some(&root_id), id) {
        Found::Folder(existing) => Ok(existing),
        Found::Free => create_folder(store, company, id, Some(root_id), origin).await,
        Found::Collision(why) => Err(OpenCompanyError::Conflict(why)),
    }
}

/// What a lookup for one named node under one parent found.
enum Found {
    /// Exactly one folder carries the name — adopt it, by id.
    Folder(String),
    /// Nothing carries the name; it is free to create.
    Free,
    /// A *file* carries the name, or several nodes do. Never resolvable, with
    /// the reason phrased for a log line or an error body.
    Collision(String),
}

/// Look for a node named `name` whose parent is `parent` (`None` = the
/// workspace root).
fn find(nodes: &[WorkspaceNode], parent: Option<&str>, name: &str) -> Found {
    let matches: Vec<&WorkspaceNode> = nodes
        .iter()
        .filter(|node| node.parent_id.as_deref() == parent && node.name == name)
        .collect();

    match matches.as_slice() {
        [one] if one.kind == NodeKind::Folder => Found::Folder(one.id.clone()),
        [_] => Found::Collision(format!(
            "`{name}` already exists as a file, not a folder, so it is left alone"
        )),
        [] => Found::Free,
        many => Found::Collision(format!(
            "{count} nodes are named `{name}`, so the path is ambiguous",
            count = many.len()
        )),
    }
}

/// Create one folder and hand back its id.
async fn create_folder(
    store: &dyn WorkspaceStore,
    company: &CompanyId,
    name: &str,
    parent_id: Option<String>,
    origin: WorkspaceOrigin,
) -> Result<String> {
    let node = WorkspaceNode {
        id: crate::ports::generate_id(),
        name: name.to_string(),
        kind: NodeKind::Folder,
        parent_id,
        updated_at_millis: crate::ports::now_millis(),
        created_by: origin.clone(),
        updated_by: origin,
    };
    store.create(company, &node, None).await?;
    Ok(node.id)
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

    async fn tree_paths(ws: &Arc<dyn WorkspaceStore>, company: &CompanyId) -> Vec<String> {
        paths(&ws.tree(company).await.unwrap())
    }

    /// The scaffold is exactly two empty roots — and, crucially, nothing else.
    /// A folder per roster member is what this feature stopped doing: a member
    /// folder means "this teammate produced something", so an empty one is a
    /// claim the tree has no business making.
    #[tokio::test]
    async fn it_provisions_two_empty_system_roots() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Desks"]);
        for node in &nodes {
            assert_eq!(node.kind, NodeKind::Folder, "{} is not a folder", node.name);
            assert_eq!(
                node.created_by,
                WorkspaceOrigin::Seed,
                "{} is runtime scaffolding, not anybody's writing",
                node.name
            );
        }
    }

    /// The scaffold takes no roster and asks for none: a company with no agents
    /// at all still gets the shape of its workspace. (This reverses the earlier
    /// eager design, where an empty roster deliberately created nothing —
    /// there, a root with no children was a stray; here it is the point.)
    #[tokio::test]
    async fn a_company_with_no_roster_still_gets_both_roots() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("solo");

        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        assert_eq!(tree_paths(&ws, &company).await, vec!["Agents", "Desks"]);
    }

    /// The property that lets this run on every boot.
    #[tokio::test]
    async fn it_is_idempotent() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        for _ in 0..3 {
            ensure_workspace_scaffold(ws.as_ref(), &company)
                .await
                .unwrap();
        }

        assert_eq!(tree_paths(&ws, &company).await, vec!["Agents", "Desks"]);
    }

    /// An operator-made `Agents/` folder is adopted as-is rather than
    /// duplicated — identity is by path, so a second root would make every
    /// `Agents/...` path permanently ambiguous.
    #[tokio::test]
    async fn an_existing_root_folder_is_adopted() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ws.create(
            &company,
            &WorkspaceNode {
                id: "hand-made".to_string(),
                name: AGENTS_ROOT.to_string(),
                kind: NodeKind::Folder,
                parent_id: None,
                updated_at_millis: 1,
                created_by: WorkspaceOrigin::Operator,
                updated_by: WorkspaceOrigin::Operator,
            },
            None,
        )
        .await
        .unwrap();

        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Desks"]);
        let root = nodes.iter().find(|n| n.name == AGENTS_ROOT).unwrap();
        assert_eq!(root.id, "hand-made", "the operator's folder must be reused");
        assert_eq!(
            root.created_by,
            WorkspaceOrigin::Operator,
            "adoption must not rewrite the operator's authorship"
        );
    }

    /// Fail-closed: a root *file* named `Agents` is a collision this module has
    /// no honest way to resolve, so it leaves it alone rather than shadowing
    /// the operator's note with a rival folder of the same name. The other root
    /// is unaffected — one odd name is no reason to withhold the rest.
    #[tokio::test]
    async fn a_root_file_blocks_only_its_own_root() {
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

        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Desks"]);
        assert_eq!(
            nodes.iter().find(|n| n.name == AGENTS_ROOT).unwrap().kind,
            NodeKind::File,
            "the operator's note must not be shadowed by a folder of the same name"
        );
    }

    /// Several root nodes sharing a reserved name is the other unresolvable
    /// shape: adding a third would make it worse, so nothing is created.
    #[tokio::test]
    async fn several_nodes_sharing_a_root_name_are_left_alone() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        for id in ["dup-a", "dup-b"] {
            ws.create(
                &company,
                &WorkspaceNode {
                    id: id.to_string(),
                    name: DESKS_ROOT.to_string(),
                    kind: NodeKind::Folder,
                    parent_id: None,
                    updated_at_millis: 1,
                    created_by: WorkspaceOrigin::Operator,
                    updated_by: WorkspaceOrigin::Operator,
                },
                None,
            )
            .await
            .unwrap();
        }

        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(
            nodes.iter().filter(|n| n.name == DESKS_ROOT).count(),
            2,
            "an ambiguous root must not gain a third candidate"
        );
        assert_eq!(nodes.iter().filter(|n| n.name == AGENTS_ROOT).count(), 1);
    }

    /// The tree is company-scoped: scaffolding one company leaves another's
    /// workspace untouched.
    #[tokio::test]
    async fn scaffolding_is_per_company() {
        let (_dir, ws) = store().await;
        let acme = CompanyId::new("acme");
        let other = CompanyId::new("other");

        ensure_workspace_scaffold(ws.as_ref(), &acme).await.unwrap();

        assert!(ws.is_empty(&other).await.unwrap());
    }

    // -- the lazy minters ---------------------------------------------------

    /// The property #552's publish path depends on: minting on every publish
    /// must be free after the first one, and must hand back the *same* parent
    /// id so two deliverables land in one folder rather than two.
    #[tokio::test]
    async fn ensure_agent_folder_is_idempotent_and_stable() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let first = ensure_agent_folder(ws.as_ref(), &company, "ceo")
            .await
            .unwrap();
        let second = ensure_agent_folder(ws.as_ref(), &company, "ceo")
            .await
            .unwrap();

        assert_eq!(first, second, "a second call minted a rival folder");
        assert_eq!(
            tree_paths(&ws, &company).await,
            vec!["Agents", "Agents/ceo", "Desks"]
        );
        let nodes = ws.tree(&company).await.unwrap();
        let ceo = nodes.iter().find(|n| n.name == "ceo").unwrap();
        assert_eq!(ceo.kind, NodeKind::Folder);
        assert_eq!(ceo.created_by, agent("ceo"));
    }

    /// One agent producing something must not conjure folders for the rest of
    /// the roster — that is the whole difference from the eager design.
    #[tokio::test]
    async fn minting_one_agent_folder_leaves_the_roster_alone() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        ensure_agent_folder(ws.as_ref(), &company, "cmo")
            .await
            .unwrap();

        assert_eq!(
            tree_paths(&ws, &company).await,
            vec!["Agents", "Agents/cmo", "Desks"]
        );
    }

    /// A minter is also its own repair path: it creates the root when the
    /// scaffold never ran, so a boot whose create fail-softed still ends up
    /// with a usable `Agents/` the first time an agent produces anything.
    #[tokio::test]
    async fn ensure_agent_folder_creates_the_root_it_needs() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        let id = ensure_agent_folder(ws.as_ref(), &company, "ceo")
            .await
            .unwrap();

        let nodes = ws.tree(&company).await.unwrap();
        assert_eq!(paths(&nodes), vec!["Agents", "Agents/ceo"]);
        let root = nodes.iter().find(|n| n.name == AGENTS_ROOT).unwrap();
        assert_eq!(root.created_by, WorkspaceOrigin::Seed);
        assert_eq!(nodes.iter().find(|n| n.id == id).unwrap().name, "ceo");
    }

    /// An operator's hand-made `Agents/ceo` is adopted, not duplicated.
    #[tokio::test]
    async fn ensure_agent_folder_adopts_an_existing_folder() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_workspace_scaffold(ws.as_ref(), &company)
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
                id: "hand-made".to_string(),
                name: "ceo".to_string(),
                kind: NodeKind::Folder,
                parent_id: Some(root_id),
                updated_at_millis: 1,
                created_by: WorkspaceOrigin::Operator,
                updated_by: WorkspaceOrigin::Operator,
            },
            None,
        )
        .await
        .unwrap();

        let id = ensure_agent_folder(ws.as_ref(), &company, "ceo")
            .await
            .unwrap();

        assert_eq!(id, "hand-made");
        assert_eq!(
            ws.tree(&company)
                .await
                .unwrap()
                .iter()
                .find(|n| n.id == "hand-made")
                .unwrap()
                .created_by,
            WorkspaceOrigin::Operator,
            "adoption must not rewrite the operator's authorship"
        );
    }

    /// The minter has a caller waiting on an id, so a collision it cannot
    /// resolve is an error rather than a warn-and-carry-on — there is no id to
    /// hand back and pretending otherwise would strand the caller's write.
    #[tokio::test]
    async fn a_colliding_member_file_is_an_error_not_a_silent_skip() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_workspace_scaffold(ws.as_ref(), &company)
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

        let err = ensure_agent_folder(ws.as_ref(), &company, "ceo")
            .await
            .expect_err("a colliding note must not resolve to a folder id");
        assert!(err.to_string().contains("ceo"), "{err}");
        assert_eq!(
            ws.tree(&company)
                .await
                .unwrap()
                .iter()
                .find(|n| n.name == "ceo")
                .unwrap()
                .kind,
            NodeKind::File,
            "the operator's note must not be shadowed by a folder of the same name"
        );
    }

    /// An id that is not a legal path segment would render an unaddressable or
    /// traversal-shaped path, so it is refused before anything is created.
    #[tokio::test]
    async fn an_illegal_id_is_refused_and_creates_nothing() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        for id in ["../escape", "", ".", "a/b", "a\\b"] {
            ensure_agent_folder(ws.as_ref(), &company, id)
                .await
                .expect_err("`{id}` is not a legal path segment");
        }

        assert!(ws.is_empty(&company).await.unwrap());
    }

    /// The desk minter is the same shape one root over. It stamps `Seed` rather
    /// than an author: a desk is not an agent, and `WorkspaceOrigin` has no way
    /// to name one.
    #[tokio::test]
    async fn ensure_desk_folder_mints_under_the_desks_root() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");
        ensure_workspace_scaffold(ws.as_ref(), &company)
            .await
            .unwrap();

        let first = ensure_desk_folder(ws.as_ref(), &company, "creative_studio")
            .await
            .unwrap();
        let second = ensure_desk_folder(ws.as_ref(), &company, "creative_studio")
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            tree_paths(&ws, &company).await,
            vec!["Agents", "Desks", "Desks/creative_studio"]
        );
        let nodes = ws.tree(&company).await.unwrap();
        let desk = nodes.iter().find(|n| n.id == first).unwrap();
        assert_eq!(desk.kind, NodeKind::Folder);
        assert_eq!(desk.created_by, WorkspaceOrigin::Seed);
    }

    /// The two roots stay independent: minting a desk folder does not reach
    /// into `Agents/`, and vice versa.
    #[tokio::test]
    async fn the_two_roots_do_not_leak_into_each_other() {
        let (_dir, ws) = store().await;
        let company = CompanyId::new("acme");

        ensure_agent_folder(ws.as_ref(), &company, "shared")
            .await
            .unwrap();
        ensure_desk_folder(ws.as_ref(), &company, "shared")
            .await
            .unwrap();

        assert_eq!(
            tree_paths(&ws, &company).await,
            vec!["Agents", "Agents/shared", "Desks", "Desks/shared"]
        );
    }
}
