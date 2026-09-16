/// The wire shape the console binds to.
///
/// `fold_asides` is worthless if the field reaches the browser under a
/// different name, and `tsc` cannot catch that: the DTO is Rust, the
/// interface is hand-written TypeScript, and nothing checks one against the
/// other. This is that check.

use super::operator_test_support_1::*;
use super::operator_test_support_2::*;
use super::operator_test_support_3::*;

pub(super) fn resolve_as(scope: &str, approval_id: &str, cookie: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}"))
        .header("content-type", "application/json");
    let builder = match cookie {
        Some(cookie) => builder.header("cookie", cookie),
        None => builder,
    };
    builder
        .body(Body::from(
            serde_json::json!({ "verdict": "deny" }).to_string(),
        ))
        .unwrap()
}

pub(super) fn extend_as(scope: &str, approval_id: &str, cookie: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}/extend"));
    let builder = match cookie {
        Some(cookie) => builder.header("cookie", cookie),
        None => builder,
    };
    builder.body(Body::empty()).unwrap()
}

/// CONC. Two operators on the same card — or one on a double click — reach
/// this route at the same time. The approval may settle once and buy one
/// permission; the loser must be told it was already decided rather than
/// minting a second grant against the same effect.
///
/// Distinct from [`a_second_resolve_reports_already_resolved_and_mints_nothing`],
/// which sends its second request only after the first has fully settled:
/// that one passes even if the parked-set take is a non-atomic
/// check-then-remove, because there is no window for the two to overlap in.
/// These two are in flight together.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub(super) async fn two_simultaneous_resolves_settle_once_and_mint_one_permission() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;
    c.release.notify_one();

    let approve = || {
        resolve_request(
            &c.approval_id,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        )
    };
    // Spawned onto a multi-threaded runtime, so these genuinely overlap
    // rather than being polled to completion one at a time. Eight rather
    // than two because the window a lost take opens is narrow: one pair can
    // miss it by scheduling luck, and a race this test cannot lose is worth
    // more than a tidier number.
    let racers: Vec<_> = (0..8)
        .map(|_| tokio::spawn(c.app.clone().oneshot(approve())))
        .collect();

    let mut settled = Vec::new();
    for racer in racers {
        let response = racer
            .await
            .expect("the request task did not panic")
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        settled.push(
            body["alreadyResolved"]
                .as_bool()
                .unwrap_or_else(|| panic!("a receipt says whether it settled: {body}")),
        );
    }
    assert_eq!(
        settled.iter().filter(|already| !**already).count(),
        1,
        "exactly one simultaneous resolve may settle the approval, got {settled:?}"
    );

    assert!(await_continuation(&c.runtime).await);
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "simultaneous approves must not buy more than one permission"
    );
}

pub(super) fn attachment_binary_node(id: &str, name: &str, mime: &str) -> WorkspaceNode {
    WorkspaceNode {
        id: id.to_string(),
        name: name.to_string(),
        kind: NodeKind::File,
        parent_id: None,
        updated_at_millis: 1_700_000_000_000,
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: Some(mime.to_string()),
        size: None,
        sha256: None,
        adopted: false,
    }
}

pub(super) fn attachment_folder_node(id: &str, name: &str) -> WorkspaceNode {
    WorkspaceNode {
        id: id.to_string(),
        name: name.to_string(),
        kind: NodeKind::Folder,
        parent_id: None,
        updated_at_millis: 1_700_000_000_000,
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: None,
        size: None,
        sha256: None,
        adopted: false,
    }
}

pub(super) fn attachment_note_node(id: &str, name: &str) -> WorkspaceNode {
    WorkspaceNode {
        kind: NodeKind::File,
        ..attachment_folder_node(id, name)
    }
}

/// Two companies on one host, each with its own signed-in admin — the
/// shape a cross-company (IDOR) question needs, since a single-company
/// host cannot tell "refused for crossing a boundary" from "there was
/// nothing else to reach". Mirrors
/// `graphql::bridge_scope_test::state_with_two_companies`.
pub(super) async fn state_with_two_companies(home: &std::path::Path) -> AppState {
    use crate::ports::store::CompanyStore;
    let store = FsCompanyStore::new(home.to_path_buf());
    let state = AppState::new(AppConfig::default());
    for name in ["acme", "globex"] {
        let id = CompanyId::new(name);
        let m = manifest();
        store
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: m.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_tool_grants: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                template_provenance: None,
                setup: None,
                name_confirmed: false,
                activation_completed_at: None,
                created_at_millis: None,
            })
            .await
            .unwrap();
        let runtime = RuntimeBuilder::new(home.to_path_buf(), m)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        state.registry().insert(id, Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, name).await;
    }
    state
}

pub(super) fn chat_with_attachments(company: &str, attachments: Vec<String>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/companies/{company}/chat"))
        .header("cookie", crate::server::test_support::fixed_cookie(company))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "text": "see attached", "attachments": attachments })
                .to_string(),
        ))
        .unwrap()
}

pub(super) async fn last_operator_message_attachments(
    runtime: &Arc<CompanyRuntime>,
    company: &CompanyId,
) -> Vec<Attachment> {
    let events = runtime
        .events()
        .read_from(company, EventSeq::new(0), usize::MAX)
        .await
        .unwrap();
    events
        .into_iter()
        .rev()
        .find_map(|stored| match stored.event {
            CompanyEvent::OperatorMessage { attachments, .. } => Some(attachments),
            _ => None,
        })
        .expect("an operator message was journaled")
}
