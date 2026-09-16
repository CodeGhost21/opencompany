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

/// Seeds root folders that share `name` by writing the workspace index
/// directly.
///
/// The filesystem store refuses to *create* two siblings under one name,
/// because on that backend they would resolve to one path (issue #666).
/// The trees below are the ones that check what the scaffold does when it
/// nevertheless *finds* an ambiguous root — an index written before that
/// refusal existed, or one an id-keyed backend can still represent legally.
/// So the state is written rather than requested: going through `create`
/// would only re-assert the store's refusal and never reach the scaffold.
async fn seed_duplicate_roots(
    dir: &std::path::Path,
    company: &CompanyId,
    name: &str,
    ids: &[&str],
) {
    let index: std::collections::HashMap<String, WorkspaceNode> = ids
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                WorkspaceNode {
                    id: (*id).to_string(),
                    name: name.to_string(),
                    kind: NodeKind::Folder,
                    parent_id: None,
                    updated_at_millis: 1,
                    created_by: WorkspaceOrigin::Operator,
                    updated_by: WorkspaceOrigin::Operator,
                    mime: None,
                    size: None,
                    sha256: None,
                    adopted: false,
                },
            )
        })
        .collect();
    let bundle = crate::store::Bundle::new(dir.to_path_buf(), company);
    tokio::fs::create_dir_all(bundle.workspace_dir())
        .await
        .expect("workspace dir");
    tokio::fs::write(
        bundle.workspace_index_json(),
        serde_json::to_vec(&index).expect("index json"),
    )
    .await
    .expect("seed index");
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

fn scaffold_paths() -> Vec<&'static str> {
    vec![
        "agents",
        "artifacts",
        "artifacts/readme.md",
        "secrets",
        "secrets/readme.md",
    ]
}

/// The scaffold has an empty agent root plus the operator-only secrets
/// folder and its explanatory note. It never creates roster member folders
/// or the unused `desks/` root.
#[tokio::test]
async fn it_provisions_one_empty_system_root() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        paths(&nodes),
        scaffold_paths(),
        "`desks/` has no producer, so boot must not lay it down"
    );
    for node in nodes.iter().filter(|node| node.kind == NodeKind::Folder) {
        assert_eq!(
            node.created_by,
            WorkspaceOrigin::Seed,
            "{} is runtime scaffolding, not anybody's writing",
            node.name
        );
    }
    // Both notes, by path: two roots now carry a `readme.md`, so a
    // find-by-name would assert against whichever the store happened to
    // return first and pass while one of them held the other's text.
    for (path, expected) in [
        ("secrets/readme.md", SECRETS_README),
        ("artifacts/readme.md", ARTIFACTS_README),
    ] {
        let readme = nodes
            .iter()
            .find(|node| path_of(&nodes, node) == path)
            .unwrap_or_else(|| panic!("{path} is missing from the scaffold"));
        let (_, body) = ws.read(&company, &readme.id).await.unwrap().unwrap();
        assert_eq!(body, expected, "{path}");
    }
}

/// The scaffold takes no roster and asks for none: a company with no agents
/// at all still gets the shape of its workspace. (This reverses the earlier
/// eager design, where an empty roster deliberately created nothing —
/// there, a root with no children was a stray; here it is the point.)
#[tokio::test]
async fn a_company_with_no_roster_still_gets_the_agents_root() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("solo");

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    assert_eq!(tree_paths(&ws, &company).await, scaffold_paths());
}

/// The deliverables root is scaffolded; a teammate's folder beneath it is
/// not, and appears only when that teammate publishes.
///
/// The asymmetry is the whole design: the root says the company has
/// somewhere to put deliverables, a member folder says *this* teammate
/// delivered. An eager folder per roster member would make the second claim
/// on behalf of teammates that have produced nothing.
#[tokio::test]
async fn an_artifact_folder_is_minted_on_demand_beneath_a_scaffolded_root() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();
    assert!(
        !tree_paths(&ws, &company)
            .await
            .contains(&"artifacts/cmo".to_string()),
        "boot must not mint a folder for a teammate that has published nothing"
    );

    let first = ensure_artifact_folder(ws.as_ref(), &company, "cmo")
        .await
        .unwrap();
    let second = ensure_artifact_folder(ws.as_ref(), &company, "cmo")
        .await
        .unwrap();
    assert_eq!(first, second, "a second call minted a rival folder");

    let nodes = ws.tree(&company).await.unwrap();
    let mine = nodes.iter().find(|node| node.id == first).unwrap();
    assert_eq!(path_of(&nodes, mine), "artifacts/cmo");
    assert_eq!(mine.kind, NodeKind::Folder);
    assert_eq!(mine.created_by, agent("cmo"));
    assert!(
        !nodes
            .iter()
            .any(|node| path_of(&nodes, node) == "agents/cmo"),
        "publishing must not also mint the agent's scratch home"
    );
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

    assert_eq!(tree_paths(&ws, &company).await, scaffold_paths());
}

/// An operator-made `Agents/` folder is adopted as-is rather than
/// duplicated — identity is by path, so a second root would make every
/// `agents/...` path permanently ambiguous.
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
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        None,
    )
    .await
    .unwrap();

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(paths(&nodes), scaffold_paths());
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
/// the operator's note with a rival folder of the same name — and creates
/// nothing else in its place.
#[tokio::test]
async fn a_root_file_is_left_alone_rather_than_shadowed() {
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
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        Some("# not a folder"),
    )
    .await
    .unwrap();

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        paths(&nodes),
        scaffold_paths(),
        "the collision must not be shadowed; unrelated scaffold still provisions"
    );
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
    let (dir, ws) = store().await;
    let company = CompanyId::new("acme");
    seed_duplicate_roots(dir.path(), &company, AGENTS_ROOT, &["dup-a", "dup-b"]).await;

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        nodes.iter().filter(|n| n.name == AGENTS_ROOT).count(),
        2,
        "an ambiguous root must not gain a third candidate"
    );
    assert_eq!(
        paths(&nodes),
        vec![
            "agents",
            "agents",
            "artifacts",
            "artifacts/readme.md",
            "secrets",
            "secrets/readme.md"
        ],
        "only the unrelated secrets scaffold may be created beside the collision"
    );
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
        vec![
            "agents",
            "agents/ceo",
            "artifacts",
            "artifacts/readme.md",
            "secrets",
            "secrets/readme.md"
        ]
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
        vec![
            "agents",
            "agents/cmo",
            "artifacts",
            "artifacts/readme.md",
            "secrets",
            "secrets/readme.md"
        ]
    );
}

/// A minter is also its own repair path: it creates the root when the
/// scaffold never ran, so a boot whose create fail-softed still ends up
/// with a usable `agents/` the first time an agent produces anything.
#[tokio::test]
async fn ensure_agent_folder_creates_the_root_it_needs() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");

    let id = ensure_agent_folder(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(paths(&nodes), vec!["agents", "agents/ceo"]);
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
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
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
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
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

/// Issue #1839: a folder a rival adopted survives that rival's rollback.
///
/// The residual half of #1801 removes a folder one caller minted and then
/// failed to write beneath. But a second caller can adopt the same folder in
/// the window — `adopt_or_create_folder` hands it back and stamps the lease —
/// and the minter's `rollback_empty_minted_folders` must then leave it
/// standing, because the adopter is about to write into it. The still-empty
/// guard alone could not tell the two apart; the lease is what does.
#[tokio::test]
async fn rollback_leaves_an_adopted_folder_but_sweeps_an_unadopted_one() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");

    // The minter creates `agents/cmo/` — its id is what a failed write would
    // roll back.
    let (adopted_id, created) = ensure_agent_folder_tracked(ws.as_ref(), &company, "cmo")
        .await
        .unwrap();
    assert!(created, "the first call minted the folder");

    // A rival publisher adopts the very same folder, taking the lease.
    let root_id = ws
        .tree(&company)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.name == AGENTS_ROOT)
        .unwrap()
        .id;
    let claim = ws
        .adopt_or_create_folder(&company, Some(&root_id), "cmo", agent("cmo"))
        .await
        .unwrap();
    assert!(!claim.was_created(), "the rival adopted, it did not mint");
    assert!(claim.node().adopted, "adoption took the lease");

    // A second minted folder nobody adopts is the genuine #1801 leak.
    let (leaked_id, _) = ensure_agent_folder_tracked(ws.as_ref(), &company, "cto")
        .await
        .unwrap();

    // The minter's write failed; it rolls back both folders it minted.
    rollback_empty_minted_folders(
        ws.as_ref(),
        &company,
        &[adopted_id.clone(), leaked_id.clone()],
    )
    .await;

    let names: Vec<String> = ws
        .tree(&company)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert!(
        names.contains(&"cmo".to_string()),
        "an adopted empty folder must survive the minter's rollback: {names:?}"
    );
    assert!(
        !names.contains(&"cto".to_string()),
        "but an unadopted empty minted folder is still swept: {names:?}"
    );
}

/// The desk minter is the same shape one root over — and since issue #645
/// it is the *only* thing that ever creates `desks/`. Deliberately run with
/// no scaffold at all: the first call must mint the root and the member
/// folder together, which is what lets boot stop laying down an empty root
/// nothing was filling.
///
/// The root it mints stamps `Seed`, exactly as the boot scaffold used to,
/// so no consumer can tell a lazily-minted root from the old eager one. The
/// desk folder stamps `Seed` too, because a desk is not an agent and
/// `WorkspaceOrigin` has no way to name one.
#[tokio::test]
async fn ensure_desk_folder_mints_the_desks_root_on_first_use() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");

    let first = ensure_desk_folder(ws.as_ref(), &company, "creative_studio")
        .await
        .unwrap();
    let second = ensure_desk_folder(ws.as_ref(), &company, "creative_studio")
        .await
        .unwrap();

    assert_eq!(first, second, "a second call minted a rival folder");
    assert_eq!(
        tree_paths(&ws, &company).await,
        vec!["desks", "desks/creative-studio"],
        "the root appears with its first occupant, and brings nothing else"
    );
    let nodes = ws.tree(&company).await.unwrap();
    let desk = nodes.iter().find(|n| n.id == first).unwrap();
    assert_eq!(desk.kind, NodeKind::Folder);
    assert_eq!(desk.created_by, WorkspaceOrigin::Seed);
    let root = nodes.iter().find(|n| n.name == DESKS_ROOT).unwrap();
    assert_eq!(root.kind, NodeKind::Folder);
    assert_eq!(
        root.created_by,
        WorkspaceOrigin::Seed,
        "a lazily-minted root must carry the stamp boot used to give it"
    );
}

/// The migration story for every company that booted before issue #645: its
/// `desks/` root already exists, and the scaffold must leave it completely
/// alone rather than notice it is no longer managed and tidy it away.
///
/// The scaffold only ever looks up the names in `SYSTEM_ROOTS`, so a
/// `desks/` node is not even inspected — id, authorship and contents all
/// survive untouched.
#[tokio::test]
async fn a_pre_existing_desks_root_survives_the_scaffold_untouched() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("legacy");
    ws.create(
        &company,
        &WorkspaceNode {
            id: "legacy-desks".to_string(),
            name: DESKS_ROOT.to_string(),
            kind: NodeKind::Folder,
            parent_id: None,
            updated_at_millis: 1,
            created_by: WorkspaceOrigin::Operator,
            updated_by: WorkspaceOrigin::Operator,
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        None,
    )
    .await
    .unwrap();

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        paths(&nodes),
        vec![
            "agents",
            "artifacts",
            "artifacts/readme.md",
            "desks",
            "secrets",
            "secrets/readme.md"
        ],
        "dropping `desks/` from the scaffold must not delete an existing one"
    );
    let desks = nodes.iter().find(|n| n.name == DESKS_ROOT).unwrap();
    assert_eq!(desks.id, "legacy-desks", "the existing root must be kept");
    assert_eq!(
        desks.created_by,
        WorkspaceOrigin::Operator,
        "an unmanaged root's authorship must not be rewritten"
    );
}

/// The un-managed counterpart to `several_nodes_sharing_a_root_name_are_
/// left_alone`: duplicate `Desks` nodes are not a collision the scaffold
/// has to resolve any more, they are simply none of its business — and the
/// root it *does* manage still provisions beside them.
#[tokio::test]
async fn duplicate_desks_nodes_do_not_disturb_the_scaffold() {
    let (dir, ws) = store().await;
    let company = CompanyId::new("acme");
    seed_duplicate_roots(dir.path(), &company, DESKS_ROOT, &["dup-a", "dup-b"]).await;

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        nodes.iter().filter(|n| n.name == DESKS_ROOT).count(),
        2,
        "an unmanaged name must be neither deduplicated nor added to"
    );
    assert_eq!(
        nodes.iter().filter(|n| n.name == AGENTS_ROOT).count(),
        1,
        "an odd name elsewhere is no reason to withhold a managed root"
    );
}

/// The two roots stay independent: minting a desk folder does not reach
/// into `agents/`, and vice versa.
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
        vec!["agents", "agents/shared", "desks", "desks/shared"]
    );
}

/// The names the scaffold mints follow the workspace naming rule, so a
/// fresh company's tree is uniform from the first boot rather than mixing
/// `Agents/` with `playbooks/` the moment anybody puts something in it.
#[tokio::test]
async fn the_scaffolded_names_are_lowercase_and_dashed() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();
    ensure_agent_folder(ws.as_ref(), &company, "page_builder")
        .await
        .unwrap();

    let paths = tree_paths(&ws, &company).await;
    assert!(
        paths.contains(&"agents/page-builder".to_string()),
        "a snake_case roster id should mint a dashed folder: {paths:?}"
    );
    for path in &paths {
        assert_eq!(
            *path,
            crate::company::workspace_names::kebab_path(path),
            "the scaffold minted a name outside the rule: {path}"
        );
    }
}

/// A company created before the rule has `Agents/`, and must not grow a
/// second lowercase root beside it — that would put one agent's home in two
/// places, with neither view complete.
#[tokio::test]
async fn a_legacy_capitalised_root_is_adopted_not_duplicated() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    let legacy = ws
        .adopt_or_create_folder(&company, None, "Agents", WorkspaceOrigin::Operator)
        .await
        .unwrap()
        .into_node()
        .id;

    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();
    let home = ensure_agent_folder(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();

    let nodes = ws.tree(&company).await.unwrap();
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n.parent_id.is_none() && n.name.eq_ignore_ascii_case("agents"))
            .count(),
        1,
        "the legacy root should be adopted, not joined by a twin: {:?}",
        paths(&nodes)
    );
    assert_eq!(
        nodes.iter().find(|n| n.id == home).unwrap().parent_id,
        Some(legacy),
        "the member folder belongs under the root that already existed"
    );
}

/// The other half of the same upgrade: the member folder itself was named
/// by the roster id verbatim, which differs from its dashed form by a
/// character rather than by case, so `find` cannot see it.
#[tokio::test]
async fn a_legacy_member_folder_named_by_the_raw_id_is_adopted() {
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
    let legacy = ws
        .adopt_or_create_folder(
            &company,
            Some(&root_id),
            "page_builder",
            agent("page_builder"),
        )
        .await
        .unwrap()
        .into_node()
        .id;

    let adopted = ensure_agent_folder(ws.as_ref(), &company, "page_builder")
        .await
        .unwrap();

    assert_eq!(adopted, legacy, "one agent, one folder, across the upgrade");
    let nodes = ws.tree(&company).await.unwrap();
    assert!(
        !nodes.iter().any(|n| n.name == "page-builder"),
        "a rival dashed folder would split the agent's work: {:?}",
        paths(&nodes)
    );
}

// -- the created-vs-adopted signal and the compensating rollback (#1801) --

/// The tracked minter reports whether *this* call created the member folder:
/// `true` the first time, `false` once it is only adopting what stands. That
/// signal is what lets a failed write know which folder it, and only it,
/// brought into existence.
#[tokio::test]
async fn ensure_agent_folder_tracked_reports_created_then_adopted() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let (first, created) = ensure_agent_folder_tracked(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();
    assert!(created, "the first call mints the member folder");

    let (second, created_again) = ensure_agent_folder_tracked(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();
    assert!(!created_again, "a second call adopts rather than minting");
    assert_eq!(first, second, "and hands back the same folder");
}

/// A minted folder that never received the write it was made for is swept
/// when the caller rolls back — leaving no empty `agents/<id>/` for the
/// Repair button. The reserved root it hangs off is scaffolding and stays.
#[tokio::test]
async fn rollback_removes_a_minted_folder_that_stayed_empty() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();

    let (home, created) = ensure_agent_folder_tracked(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();
    assert!(created);
    assert!(
        tree_paths(&ws, &company)
            .await
            .contains(&"agents/ceo".to_string())
    );

    rollback_empty_minted_folders(ws.as_ref(), &company, &[home]).await;

    let paths = tree_paths(&ws, &company).await;
    assert!(
        !paths.contains(&"agents/ceo".to_string()),
        "an empty minted folder must be swept when its write never landed: {paths:?}"
    );
    assert!(
        paths.contains(&"agents".to_string()),
        "the scaffolded root must survive the rollback: {paths:?}"
    );
}

/// The over-deletion guard: a folder that gained a child in the window — a
/// concurrent create, or the very write the caller thought had failed — is
/// left exactly as it stands, and its child is never deleted out from under
/// it by a recursive sweep.
#[tokio::test]
async fn rollback_keeps_a_minted_folder_that_gained_a_child() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    ensure_workspace_scaffold(ws.as_ref(), &company)
        .await
        .unwrap();
    let (home, _) = ensure_agent_folder_tracked(ws.as_ref(), &company, "ceo")
        .await
        .unwrap();

    ws.create(
        &company,
        &WorkspaceNode {
            id: "kept-note".to_string(),
            name: "brief.md".to_string(),
            kind: NodeKind::File,
            parent_id: Some(home.clone()),
            updated_at_millis: 1,
            created_by: WorkspaceOrigin::Operator,
            updated_by: WorkspaceOrigin::Operator,
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        Some("# keep me"),
    )
    .await
    .unwrap();

    rollback_empty_minted_folders(ws.as_ref(), &company, &[home]).await;

    let paths = tree_paths(&ws, &company).await;
    assert!(
        paths.contains(&"agents/ceo".to_string()),
        "a folder that gained a child must survive: {paths:?}"
    );
    assert!(
        paths.contains(&"agents/ceo/brief.md".to_string()),
        "and its child must not be deleted out from under it: {paths:?}"
    );
}

/// A store double that, the first time `tree` is called, hands back a
/// snapshot exactly like the real one below it — and then, *after*
/// capturing that snapshot but before returning it, writes a child into
/// `inject_child_under` directly against the wrapped store. This puts the
/// wrapped store one write ahead of whatever the caller does with the
/// snapshot it receives — precisely the shape of the race review found: a
/// concurrent adopter's write landing in the window between a `tree()`
/// read and a later `delete()` built from it.
///
/// Every other method forwards straight through; only `tree`'s first call
/// carries the injected write, so a second `tree()` call (e.g. inside
/// `delete_if_empty`'s own fresh check) sees it, but the snapshot handed
/// to the *caller* of the first call never does.
struct InjectChildAfterFirstTree {
    inner: Arc<dyn WorkspaceStore>,
    inject_child_under: String,
    injected: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl WorkspaceStore for InjectChildAfterFirstTree {
    async fn tree(&self, company: &CompanyId) -> Result<Vec<WorkspaceNode>> {
        let snapshot = self.inner.tree(company).await?;
        if !self
            .injected
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            self.inner
                .create(
                    company,
                    &WorkspaceNode {
                        id: "raced-in-note".to_string(),
                        name: "raced-in.md".to_string(),
                        kind: NodeKind::File,
                        parent_id: Some(self.inject_child_under.clone()),
                        updated_at_millis: 1,
                        created_by: WorkspaceOrigin::Operator,
                        updated_by: WorkspaceOrigin::Operator,
                        mime: None,
                        size: None,
                        sha256: None,
                        adopted: false,
                    },
                    Some("landed mid-rollback"),
                )
                .await
                .expect("inject concurrent child");
        }
        Ok(snapshot)
    }

    async fn read(
        &self,
        company: &CompanyId,
        id: &str,
    ) -> Result<Option<(WorkspaceNode, String)>> {
        self.inner.read(company, id).await
    }
    async fn read_capped(
        &self,
        company: &CompanyId,
        id: &str,
        max_bytes: u64,
    ) -> Result<Option<(WorkspaceNode, String, u64)>> {
        self.inner.read_capped(company, id, max_bytes).await
    }

    async fn write_with_revision(
        &self,
        company: &CompanyId,
        id: &str,
        content: &str,
        author: WorkspaceOrigin,
        expected_updated_at: Option<u64>,
    ) -> Result<WorkspaceNode> {
        self.inner
            .write_with_revision(company, id, content, author, expected_updated_at)
            .await
    }

    async fn create(
        &self,
        company: &CompanyId,
        node: &WorkspaceNode,
        content: Option<&str>,
    ) -> Result<()> {
        self.inner.create(company, node, content).await
    }

    async fn adopt_or_create_folder(
        &self,
        company: &CompanyId,
        parent: Option<&str>,
        name: &str,
        origin: WorkspaceOrigin,
    ) -> Result<crate::ports::workspace::FolderClaim> {
        self.inner
            .adopt_or_create_folder(company, parent, name, origin)
            .await
    }

    async fn create_binary(
        &self,
        company: &CompanyId,
        node: &WorkspaceNode,
        bytes: &[u8],
    ) -> Result<WorkspaceNode> {
        self.inner.create_binary(company, node, bytes).await
    }

    async fn write_binary(
        &self,
        company: &CompanyId,
        id: &str,
        bytes: &[u8],
        mime: Option<&str>,
        author: WorkspaceOrigin,
    ) -> Result<WorkspaceNode> {
        self.inner
            .write_binary(company, id, bytes, mime, author)
            .await
    }

    async fn read_bytes(
        &self,
        company: &CompanyId,
        id: &str,
    ) -> Result<Option<(WorkspaceNode, crate::ports::workspace::BlobStream)>> {
        self.inner.read_bytes(company, id).await
    }

    async fn rename_move(
        &self,
        company: &CompanyId,
        id: &str,
        name: Option<&str>,
        parent: Option<Option<&str>>,
    ) -> Result<WorkspaceNode> {
        self.inner.rename_move(company, id, name, parent).await
    }

    async fn swap_files(
        &self,
        company: &CompanyId,
        expected_id: Option<&str>,
        replacement_id: &str,
        name: &str,
    ) -> Result<Option<WorkspaceNode>> {
        self.inner
            .swap_files(company, expected_id, replacement_id, name)
            .await
    }

    async fn delete(&self, company: &CompanyId, id: &str) -> Result<bool> {
        self.inner.delete(company, id).await
    }

    // Forwarded explicitly, exactly like the production decorators
    // (`WorkspaceAnnouncer`, `QuotaEnforcedWorkspace`, `DerivedGuardWorkspace`)
    // — so this test exercises the wrapped store's own `delete_if_empty`
    // (here, `FsOps`'s single-lock override) rather than the default trait
    // method re-deriving the check at this wrapper's level.
    async fn delete_if_empty(&self, company: &CompanyId, id: &str) -> Result<bool> {
        self.inner.delete_if_empty(company, id).await
    }

    async fn is_empty(&self, company: &CompanyId) -> Result<bool> {
        self.inner.is_empty(company).await
    }
}

/// The race itself: a child lands under a minted-but-empty folder in the
/// window between `rollback_empty_minted_folders`'s own `tree()` read and
/// the `delete` it would have issued from that stale read. Before the
/// fix, `rollback_empty_minted_folders` decided emptiness from that same
/// stale snapshot and called the unconditional `delete`, which recursed
/// through the folder and erased the concurrently-landed child with it —
/// this test fails on that code, asserting the child survives. After the
/// fix, the decision is `delete_if_empty`'s own fresh re-check, which sees
/// the child and refuses to remove the folder.
#[tokio::test]
async fn rollback_does_not_erase_a_child_that_lands_mid_rollback() {
    let (_dir, real) = store().await;
    let company = CompanyId::new("acme");
    ensure_workspace_scaffold(real.as_ref(), &company)
        .await
        .unwrap();
    let (home, _) = ensure_agent_folder_tracked(real.as_ref(), &company, "ceo")
        .await
        .unwrap();

    let racy: Arc<dyn WorkspaceStore> = Arc::new(InjectChildAfterFirstTree {
        inner: real.clone(),
        inject_child_under: home.clone(),
        injected: std::sync::atomic::AtomicBool::new(false),
    });

    rollback_empty_minted_folders(racy.as_ref(), &company, &[home]).await;

    let paths = tree_paths(&real, &company).await;
    assert!(
        paths.contains(&"agents/ceo".to_string()),
        "a folder a concurrent write landed a child into mid-rollback must survive: {paths:?}"
    );
    assert!(
        paths.contains(&"agents/ceo/raced-in.md".to_string()),
        "the concurrently-landed child must not be erased by the rollback: {paths:?}"
    );
}

/// A reserved root is never a rollback target even when handed in: an empty
/// `agents/` is ordinary boot scaffolding, and the next boot re-lays it.
#[tokio::test]
async fn rollback_never_removes_a_reserved_root() {
    let (_dir, ws) = store().await;
    let company = CompanyId::new("acme");
    let root = ws
        .adopt_or_create_folder(&company, None, AGENTS_ROOT, WorkspaceOrigin::Seed)
        .await
        .unwrap()
        .into_node()
        .id;

    rollback_empty_minted_folders(ws.as_ref(), &company, &[root]).await;

    assert!(
        tree_paths(&ws, &company)
            .await
            .contains(&"agents".to_string()),
        "an empty reserved root must never be swept"
    );
}
