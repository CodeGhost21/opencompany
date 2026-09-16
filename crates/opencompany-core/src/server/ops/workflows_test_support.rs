//! Shared fixtures for the `workflows` ops test-file cluster.
//!
//! This module holds the const/fn/struct fixtures that used to live in a
//! single inline `#[cfg(test)] mod tests { ... }` block in `workflows.rs`
//! before it was mechanically split into the sibling `workflows_*_tests.rs`
//! files declared at the tail of `workflows.rs`. Splitting scattered the
//! shared helpers across files that can no longer see each other, so they
//! are collected back here for every split file to import.
//!
//! Layout mirrors the original nesting: fixtures shared by every test file
//! live at the top level (`pub(super)`, reachable from any sibling test
//! file one level up via `super::workflows_test_support::*`); fixtures used
//! only by the hosted-mode cluster or only by the running cluster live in
//! their own nested module (`pub(crate)`, since a `pub(super)` two levels
//! down would only reach back up to this module, not out to the sibling
//! test files that need it).
#![cfg(test)]

use super::*;
use crate::company::{list_workflows_union, load_workflow_union};


/// The listed rows this company itself has, with the global baseline
/// filtered out. Every company lists the baseline graphs; these tests are
/// about what this one created, deleted, or declared.
///
/// This is an **id heuristic**, not provenance: `WorkflowSummary` carries
/// no `global` flag, so a row is classified as "the baseline's" purely by
/// id membership in `crate::globals::workflows()`. A company definition of
/// the *same* id supersedes the global one and would be wrongly excluded
/// here — none of the fixtures below give a company workflow a colliding
/// id, so the gap does not fire in this suite; see
/// `write_test::workflow_create_of_an_id_matching_a_global_wins_by_content`
/// for that case asserted directly, without this helper.
pub(super) fn own_rows(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("array response")
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            !crate::globals::workflows().iter().any(|w| w.id == id)
        })
        .collect()
}


pub(super) const DEMO: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    summary = "Kicks it off."
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
    summary = "Does the thing."
    agent = "assistant"
    [[node]]
    id = "done"
    kind = "output"
    name = "Report"
    [[edge]]
    from = "start"
    to = "worker"
    [[edge]]
    from = "worker"
    to = "done"
    label = "ok"
"#;


/// Writes `DEMO` to `<dir>/workflows/demo.toml` and returns `dir`.
pub(super) fn seed_demo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("demo.toml"), DEMO).unwrap();
    dir
}

pub(crate) mod hosted_mode {
        pub(in crate::server::ops::workflows) use super::own_rows;
        pub(crate) use axum::body::{Body, to_bytes};
        pub(crate) use axum::http::{Request, StatusCode};
        pub(crate) use tower::ServiceExt;

        pub(in crate::server::ops::workflows) use super::super::{
            DEFAULT_RUN_LIMIT, MAX_RUN_ARTIFACTS, select_run_page,
        };
        pub(crate) use super::super::WorkflowRunOutcome;
        pub(crate) use crate::ports::types::{CompanyEvent, WorkflowNodeStatus};
        pub(crate) use crate::ports::workflow_verdict::WorkflowRunVerdict;
        pub(crate) use crate::company::CompanyManifest;
        pub(crate) use crate::ports::CompanyStore;
        pub(crate) use crate::ports::types::{CompanyId, CompanyRecord};
        pub(crate) use crate::runtime::RuntimeBuilder;
        pub(crate) use crate::server::router;
        pub(crate) use crate::store::FsCompanyStore;
        pub(crate) use crate::{AppConfig, AppState};


        pub(crate) fn home() -> tempfile::TempDir {
            tempfile::Builder::new()
                .prefix("oc-workflows-hosted-")
                .tempdir()
                .expect("tempdir")
        }


        /// A manifest declaring one enabled workflow — mirrors what a
        /// platform tenant provisions with, minus any `workflows/` directory
        /// on disk (there isn't one: hosted tenants have no source dir).
        pub(crate) fn manifest_with_enabled() -> CompanyManifest {
            toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[workflows]\nenabled = [\"demo\"]\n",
            )
            .unwrap()
        }


        /// Builds a running company whose runtime has **no source directory**
        /// (built without `with_seed_dir`, matching how the platform builds a
        /// provisioned tenant) but whose persisted record declares an enabled
        /// workflow — the exact hosted-mode gap #70 reports.
        pub(crate) async fn state_with_hosted_company(home: &std::path::Path) -> AppState {
            state_with_hosted_company_lifecycle(home, "running").await
        }


        /// The same fixture at a chosen lifecycle, so a paused company is
        /// reachable without a second copy of the record literal.
        pub(crate) async fn state_with_hosted_company_lifecycle(
            home: &std::path::Path,
            lifecycle: &str,
        ) -> AppState {
            let store = FsCompanyStore::new(home.to_path_buf());
            let id = CompanyId::new("acme");
            store
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    overlay_retired_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    id: id.clone(),
                    manifest: manifest_with_enabled(),
                    ledger: Vec::new(),
                    lifecycle: lifecycle.to_string(),
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
            let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest_with_enabled())
                .with_id(id.clone())
                .build()
                .await
                .unwrap();
            assert!(
                runtime.source_dir().is_none(),
                "test setup must simulate hosted mode: no source dir"
            );
            let state = AppState::new(AppConfig::default());
            state.registry().insert(id, std::sync::Arc::new(runtime));
            crate::server::test_support::seed_fixed_admin(&state, "acme").await;
            state
        }


        /// A hosted tenant's record with no manifest-enabled workflows — the
        /// blank-slate a real tenant starts from before it creates anything.
        pub(crate) fn empty_manifest() -> CompanyManifest {
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
        }


        /// Same as [`state_with_hosted_company`] but with nothing enabled, and
        /// returning the store so a test can rebuild state from it.
        pub(crate) async fn hosted_state(home: &std::path::Path) -> (AppState, FsCompanyStore, CompanyId) {
            let store = FsCompanyStore::new(home.to_path_buf());
            let id = CompanyId::new("acme");
            store
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    overlay_retired_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    id: id.clone(),
                    manifest: empty_manifest(),
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
            let state = state_over(home, &id, true).await;
            (state, store, id)
        }


        /// Builds an `AppState` whose runtime for `id` has **no source
        /// directory** — the hosted shape — over the store rooted at `home`.
        ///
        /// `seed_admin` seeds the fixed admin + session; a *rebuild* over the
        /// same home must pass `false` (the durable user store already has that
        /// admin, and its session survives with it).
        pub(crate) async fn state_over(home: &std::path::Path, id: &CompanyId, seed_admin: bool) -> AppState {
            let runtime = RuntimeBuilder::new(home.to_path_buf(), empty_manifest())
                .with_id(id.clone())
                .build()
                .await
                .unwrap();
            assert!(
                runtime.source_dir().is_none(),
                "test setup must simulate hosted mode: no source dir"
            );
            let state = AppState::new(AppConfig::default());
            state
                .registry()
                .insert(id.clone(), std::sync::Arc::new(runtime));
            if seed_admin {
                crate::server::test_support::seed_fixed_admin(&state, "acme").await;
            }
            state
        }


        /// The graph body the console posts.
        pub(crate) fn create_body() -> serde_json::Value {
            serde_json::json!({
                "id": "greeter",
                "name": "Greeter",
                "description": "Say hi.",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
            })
        }


        pub(crate) fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
            let builder = Request::builder()
                .method(method)
                .uri(uri)
                .header("cookie", crate::server::test_support::fixed_cookie("acme"));
            match body {
                Some(json) => builder
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json).unwrap()))
                    .unwrap(),
                None => builder.body(Body::empty()).unwrap(),
            }
        }


        pub(crate) async fn json_body(response: axum::response::Response) -> serde_json::Value {
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        }


        // ------------------------------------------------------------------
        // `POST …/workflows/validate` — the author-time verdict, no save (#1074)
        // ------------------------------------------------------------------

        pub(crate) async fn post_validate(
            state: &AppState,
            body: serde_json::Value,
        ) -> axum::response::Response {
            router(state.clone())
                .oneshot(request(
                    "POST",
                    "/api/v1/company/workflows/validate",
                    Some(body),
                ))
                .await
                .unwrap()
        }


        pub(crate) async fn post_create_on(
            state: &AppState,
            body: serde_json::Value,
        ) -> axum::response::Response {
            router(state.clone())
                .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
                .await
                .unwrap()
        }


        /// `create_body` with an extra node nothing points at — the reachability
        /// rule (`crate::company::workflow_file`), which is one of the two a
        /// client cannot pre-empt without re-implementing it.
        pub(crate) fn body_with_an_unreachable_node() -> serde_json::Value {
            let mut body = create_body();
            body["nodes"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(
                    { "id": "orphan", "kind": "output", "name": "Orphan" }
                ));
            body
        }


        /// `create_body` with a `condition` node whose branch carries `label`,
        /// and `onError` set on the condition when `on_error` is given.
        pub(crate) fn body_with_condition(label: &str, on_error: Option<&str>) -> serde_json::Value {
            let mut gate = serde_json::json!({
                "id": "gate",
                "kind": "condition",
                "name": "Gate",
                "config": { "field": "=item.approved" }
            });
            if let Some(on_error) = on_error {
                gate["onError"] = serde_json::Value::String(on_error.to_string());
            }
            serde_json::json!({
                "id": "greeter",
                "name": "Greeter",
                "description": "Say hi.",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    gate,
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [
                    { "from": "start", "to": "gate" },
                    { "from": "gate", "to": "done", "label": label }
                ]
            })
        }


        /// A company whose runtime HAS a source directory holding one seed
        /// workflow at `workflows/child.toml` — the self-hosted / local `serve`
        /// shape. `hosted_state` deliberately has none, so the seed-file half of
        /// the `sub_workflow` existence probe is unreachable from it.
        pub(crate) async fn seeded_state(home: &std::path::Path) -> (AppState, tempfile::TempDir) {
            let source = tempfile::Builder::new()
                .prefix("oc-workflows-source-")
                .tempdir()
                .expect("tempdir");
            let workflows = source.path().join("workflows");
            std::fs::create_dir_all(&workflows).unwrap();
            std::fs::write(
                workflows.join("child.toml"),
                "id = \"child\"\nname = \"Child\"\n[[node]]\nid = \"start\"\n\
                 kind = \"trigger\"\nname = \"Start\"\n",
            )
            .unwrap();

            let store = FsCompanyStore::new(home.to_path_buf());
            let id = CompanyId::new("acme");
            store
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    id: id.clone(),
                    manifest: empty_manifest(),
                    ledger: Vec::new(),
                    lifecycle: "running".to_string(),
                    overlay_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    overlay_retired_agents: Vec::new(),
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
                    setup: Default::default(),
                    name_confirmed: false,
                    activation_completed_at: None,
                    created_at_millis: None,
                })
                .await
                .unwrap();
            let runtime = RuntimeBuilder::new(home.to_path_buf(), empty_manifest())
                .with_id(id.clone())
                .with_seed_dir(source.path())
                .build()
                .await
                .unwrap();
            assert!(
                runtime.source_dir().is_some(),
                "this fixture only proves anything with a source directory"
            );
            let state = AppState::new(AppConfig::default());
            state
                .registry()
                .insert(id.clone(), std::sync::Arc::new(runtime));
            crate::server::test_support::seed_fixed_admin(&state, "acme").await;
            (state, source)
        }


        /// A graph whose `sub_workflow` node runs `child` — which exists ONLY as
        /// a seed file, so the probe can only see it with a source directory.
        pub(crate) fn body_with_sub_workflow() -> serde_json::Value {
            serde_json::json!({
                "id": "parent",
                "name": "Parent",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    {
                        "id": "child_run",
                        "kind": "sub_workflow",
                        "name": "Run the child",
                        "config": { "workflow_id": "child" }
                    }
                ],
                "edges": [ { "from": "start", "to": "child_run" } ]
            })
        }


        // --- Save-time channel-destination guard (issue #981) ---------------

        /// A hosted tenant WITH a desk, so it has one real delivery channel.
        /// `hosted_state`'s manifest declares none, which makes its deliverable
        /// set empty — fine for the nowhere-to-deliver case below, useless for
        /// telling an accepted target from a refused one.
        pub(crate) fn desk_manifest() -> CompanyManifest {
            toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[group_chat]]\nid = \"engineering\"\nname = \"Engineering\"\nmembers = [\"ceo\"]\n",
            )
            .unwrap()
        }


        /// `hosted_state` over [`desk_manifest`] — a running company whose
        /// deliverable set is exactly `["engineering"]`.
        pub(crate) async fn desk_state(home: &std::path::Path) -> AppState {
            let store = FsCompanyStore::new(home.to_path_buf());
            let id = CompanyId::new("acme");
            store
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    overlay_retired_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    id: id.clone(),
                    manifest: desk_manifest(),
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
            let runtime = RuntimeBuilder::new(home.to_path_buf(), desk_manifest())
                .with_id(id.clone())
                .build()
                .await
                .unwrap();
            assert_eq!(
                runtime.deliverable_channel_ids(),
                vec!["operator".to_string(), "engineering".to_string()],
                "the fixture must have the operator channel plus exactly one desk channel, or \
                 these tests prove nothing"
            );
            let state = AppState::new(AppConfig::default());
            state
                .registry()
                .insert(id.clone(), std::sync::Arc::new(runtime));
            crate::server::test_support::seed_fixed_admin(&state, "acme").await;
            state
        }


        /// [`create_body`] with the output node routing its report to `target`
        /// on `kind`.
        pub(crate) fn body_with_destination(kind: &str, target: Option<&str>) -> serde_json::Value {
            let mut destination = serde_json::json!({ "kind": kind });
            if let Some(target) = target {
                destination["target"] = serde_json::Value::String(target.to_string());
            }
            let mut body = create_body();
            body["nodes"][1]["destination"] = destination;
            body
        }


        pub(crate) async fn post_create(state: AppState, body: serde_json::Value) -> axum::response::Response {
            router(state)
                .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
                .await
                .unwrap()
        }


        /// [`create_body`] with `done` turned into an `agent` node naming the
        /// roster teammate [`desk_manifest`] declares (`ceo`), carrying a
        /// declared `postcondition` — the shape a real create/edit sends.
        pub(crate) fn body_with_postcondition() -> serde_json::Value {
            let mut body = create_body();
            body["nodes"][1]["kind"] = serde_json::json!("agent");
            body["nodes"][1]["agent"] = serde_json::json!("ceo");
            body["nodes"][1]["postcondition"] = serde_json::json!({ "require": "non_empty" });
            body
        }


        /// Journals a `WorkflowRunFinished` naming a `run_id`, the shape
        /// `journaled_run_failure` scans for — distinct from `journal_run` above,
        /// which always journals `run_id: None` for the delivery-history tests.
        #[cfg(feature = "openhuman")]
        pub(crate) async fn journal_run_with_id(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            run_id: &str,
            error: &str,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowRunFinished {
                        workflow_id: workflow_id.to_string(),
                        scheduled: false,
                        run_id: Some(run_id.to_string()),
                        deliveries: Vec::new(),
                        pending_approvals: Vec::new(),
                        error: Some(error.to_string()),
                        cancelled: false,
                        notices: Vec::new(),
                        board: Vec::new(),
                        // Added by #881/#880 after this fixture was written. A
                        // failed run parks nothing and blocks nothing, so both
                        // are empty here — see `Settled::from`'s Err arm, which
                        // makes the same choice for the same reason.
                        blocked_nodes: Vec::new(),
                        approvals: Vec::new(),
                    },
                )
                .await
                .expect("append");
        }


        /// A create body whose trigger carries a cron.
        pub(crate) fn scheduled_create_body() -> serde_json::Value {
            serde_json::json!({
                "id": "digest",
                "name": "Digest",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * *" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
            })
        }


        // ── Issue #228: the run-history read ────────────────────────────────

        /// Journals a finished-run outcome directly on the company's event log,
        /// the way both entry points do via `record_run_finished`.
        pub(crate) async fn journal_run(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            scheduled: bool,
            deliveries: Vec<crate::ports::DeliveryReport>,
            error: Option<&str>,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowRunFinished {
                        workflow_id: workflow_id.to_string(),
                        scheduled,
                        run_id: None,
                        deliveries,
                        pending_approvals: Vec::new(),
                        error: error.map(str::to_string),
                        cancelled: false,
                        notices: Vec::new(),
                        board: Vec::new(),
                        blocked_nodes: Vec::new(),
                        approvals: Vec::new(),
                    },
                )
                .await
                .expect("append");
        }


        /// A report that reached its destination.
        pub(crate) fn sent_row(node: &str) -> crate::ports::DeliveryReport {
            crate::ports::DeliveryReport {
                node: node.to_string(),
                kind: "owner".to_string(),
                target: Some("ada@example.com".to_string()),
                status: crate::ports::DeliveryStatus::Sent,
                detail: "emailed the company's admin".to_string(),
                reason: crate::ports::DeliveryReason::OwnerEmailed,
            }
        }


        pub(crate) fn undelivered_row(node: &str) -> crate::ports::DeliveryReport {
            crate::ports::DeliveryReport {
                node: node.to_string(),
                kind: "email".to_string(),
                target: Some("ada@example.com".to_string()),
                status: crate::ports::DeliveryStatus::Skipped,
                detail: "this recipient has never written to the company".to_string(),
                reason: crate::ports::DeliveryReason::RecipientNotEstablished,
            }
        }


        // ------------------------------------------------------------------
        // `GET …/workflows/runs/{rid}/artifacts` — the files one run produced,
        // joined through `origin_run_id` (issue #1684).
        // ------------------------------------------------------------------

        /// Builds a board card, optionally stamped with the run that opened it
        /// (`origin_run_id`) — the field [`run_artifacts`] joins on (issue
        /// #1684). Everything else is the neutral shape the board's own tests
        /// use.
        pub(crate) fn run_card(
            id: &str,
            title: &str,
            origin_run_id: Option<&str>,
        ) -> crate::ports::TaskRecord {
            crate::ports::TaskRecord {
                id: id.into(),
                title: crate::ports::tasks::TaskTitle::authored(title),
                note: None,
                column: "in_review".into(),
                priority: "medium".into(),
                assignee: "ceo".into(),
                updated_at_millis: 1,
                origin: None,
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: origin_run_id.map(str::to_string),
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            }
        }


        /// A published artifact on `task_id`: one agent version, a `source`
        /// path, stamped `updated_at_millis` so the newest-first sort is
        /// testable.
        pub(crate) fn published(
            id: &str,
            task_id: &str,
            title: &str,
            source: &str,
            at_millis: u64,
        ) -> crate::ports::ArtifactRecord {
            let mut rec = crate::ports::ArtifactRecord::new(
                id,
                task_id,
                title,
                crate::ports::ArtifactKind::Markdown,
                "the agent's draft",
                "ceo",
                at_millis,
            )
            .with_source(source);
            rec.updated_at_millis = at_millis;
            rec
        }


        // ── Issue #371: the per-node progress fold ─────────────────────────

        /// Journals a `WorkflowRunStarted`, the way the runner does before the
        /// engine call.
        pub(crate) async fn journal_start(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            run_id: &str,
            scheduled: bool,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowRunStarted {
                        workflow_id: workflow_id.to_string(),
                        run_id: run_id.to_string(),
                        scheduled,
                        started_by: None,
                        resume_semantic: None,
                    },
                )
                .await
                .expect("append");
        }


        /// Journals one `WorkflowNodeFinished`, the way the run observer does.
        pub(crate) async fn journal_node(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            run_id: &str,
            node_id: &str,
            status: WorkflowNodeStatus,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowNodeFinished {
                        workflow_id: workflow_id.to_string(),
                        run_id: run_id.to_string(),
                        node_id: node_id.to_string(),
                        status,
                        elapsed_ms: 42,
                        diagnostics: Vec::new(),
                        agent_run_id: None,
                    },
                )
                .await
                .expect("append");
        }


        /// Journals one `WorkflowNodeStarted`, the way the run observer does
        /// immediately before a node's first attempt (issue #382).
        pub(crate) async fn journal_node_started(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            run_id: &str,
            node_id: &str,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowNodeStarted {
                        workflow_id: workflow_id.to_string(),
                        run_id: run_id.to_string(),
                        node_id: node_id.to_string(),
                    },
                )
                .await
                .expect("append");
        }


        /// Journals a finished outcome carrying a run id, the way every entry
        /// point does post-#371.
        pub(crate) async fn journal_finish(
            state: &AppState,
            id: &CompanyId,
            workflow_id: &str,
            run_id: &str,
            scheduled: bool,
            error: Option<&str>,
        ) {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .append(
                    id,
                    CompanyEvent::WorkflowRunFinished {
                        workflow_id: workflow_id.to_string(),
                        scheduled,
                        run_id: Some(run_id.to_string()),
                        deliveries: Vec::new(),
                        pending_approvals: Vec::new(),
                        error: error.map(str::to_string),
                        cancelled: false,
                        notices: Vec::new(),
                        board: Vec::new(),
                        blocked_nodes: Vec::new(),
                        approvals: Vec::new(),
                    },
                )
                .await
                .expect("append");
        }


        /// An event log that lets exactly one run settle **inside** `list_runs`'
        /// window.
        ///
        /// `read_from` delegates, and on its FIRST call appends `finish` to the
        /// inner log *after* taking the snapshot it returns. That is precisely
        /// the interleaving the race needs and the only way to get it
        /// deterministically: the run's real finish is missing from the snapshot
        /// the fold sees, and its supervisor entry is already gone by the time
        /// `live()` is consulted — so on those two facts alone it is
        /// indistinguishable from a run that died.
        ///
        /// Every later `read_from` — including the settle's own re-read of the
        /// tail — sees the finish, which is exactly what lets the read tell the
        /// two apart.
        pub(crate) struct FinishesDuringTheRead {
            pub(crate) inner: std::sync::Arc<dyn crate::ports::EventLog>,
            pub(crate) finish: std::sync::Mutex<Option<(CompanyId, CompanyEvent)>>,
        }


        #[async_trait::async_trait]
        impl crate::ports::EventLog for FinishesDuringTheRead {
            async fn append(
                &self,
                id: &CompanyId,
                event: CompanyEvent,
            ) -> crate::Result<crate::ports::types::EventSeq> {
                self.inner.append(id, event).await
            }

            async fn read_from(
                &self,
                id: &CompanyId,
                seq: crate::ports::types::EventSeq,
                limit: usize,
            ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
                let snapshot = self.inner.read_from(id, seq, limit).await?;
                // Taken out under the lock, so the append happens once however
                // many readers race here.
                let pending = self.finish.lock().expect("poisoned").take();
                if let Some((company, event)) = pending {
                    self.inner.append(&company, event).await?;
                }
                Ok(snapshot)
            }

            fn subscribe(
                &self,
                id: &CompanyId,
            ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem>
            {
                self.inner.subscribe(id)
            }
        }


        // -------------------------------------------------------------------
        // Page cut / cursor partition (issue #1012 follow-up)
        //
        // `select_run_page` is exercised directly because the anomaly these
        // tests are about — a run journaled with an `at_millis` OLDER than the
        // row before it, after the clock stepped backwards — cannot be staged
        // through the router at all: `FileStore::append` stamps
        // `at_millis: now_millis()` itself, and `journal_start`/`journal_finish`
        // hand it only a `CompanyEvent`. There is no seam to fake a clock
        // regression end-to-end, so the cut is tested where it lives and the
        // route is tested for the one thing the pure function cannot carry —
        // the serialized field.
        // -------------------------------------------------------------------

        /// A settled run at `(seq, at_millis)`, with every other field at its
        /// nothing-happened value. Only the two keys `select_run_page` reads
        /// matter here.
        pub(crate) fn page_run(seq: u64, at_millis: u64) -> WorkflowRunOutcome {
            WorkflowRunOutcome {
                seq,
                at_millis,
                workflow_id: "wf".to_string(),
                scheduled: false,
                run_id: Some(format!("run-{seq}")),
                resume_semantic: None,
                deliveries: Vec::new(),
                pending_approvals: Vec::new(),
                error: None,
                nodes: Vec::new(),
                started_nodes: Vec::new(),
                started_at_millis: Some(at_millis),
                running: false,
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
                degraded: false,
                stranded_approvals: 0,
                verdict: WorkflowRunVerdict::Ok,
            }
        }


        /// One request's worth of the read, as the route performs it: everything
        /// strictly older than the cursor is a candidate (that is exactly what
        /// `EventLog::read_before` bounds by), and the cut runs over it.
        ///
        /// Handing the whole candidate set in is a faithful superset of what the
        /// backward walk accumulates — it stops once it has settled `limit + 1`
        /// runs, and because it walks by descending `seq` those are the highest
        /// `seq`s among the candidates, which is precisely the set the cut keeps.
        pub(crate) fn page(
            journal: &[(u64, u64)],
            before_seq: Option<u64>,
            limit: usize,
        ) -> (Vec<WorkflowRunOutcome>, bool, Option<u64>) {
            let candidates: Vec<WorkflowRunOutcome> = journal
                .iter()
                .filter(|(seq, _)| before_seq.is_none_or(|bound| *seq < bound))
                .map(|(seq, at_millis)| page_run(*seq, *at_millis))
                .collect();
            select_run_page(candidates, limit)
        }


        /// A journal whose clock stepped backwards: `seq` 40 was appended after
        /// 30 but carries a wall-clock time older than both 30 and 20. Every
        /// other row is well-behaved.
        pub(crate) const REGRESSED: [(u64, u64); 5] = [
            (10, 1_000),
            (20, 2_000),
            (30, 3_000),
            // NTP correction / VM resume / an operator setting the date: the
            // append order is unchanged, the timestamp goes backwards.
            (40, 1_500),
            (50, 5_000),
        ];


        // -------------------------------------------------------------------
        // Cron preview (issue #262)
        // -------------------------------------------------------------------

        /// 2026-08-02 12:00 UTC, as epoch millis — the `after` pin every
        /// preview test searches forward from, so the answers are fixed rather
        /// than relative to whenever CI runs.
        pub(crate) const AFTER: u64 = 1_785_672_000_000;


        pub(crate) async fn preview(state: &AppState, expr: &str) -> serde_json::Value {
            json_body(
                router(state.clone())
                    .oneshot(request(
                        "POST",
                        "/api/v1/company/workflows/cron/preview",
                        Some(serde_json::json!({ "expr": expr, "after": AFTER })),
                    ))
                    .await
                    .unwrap(),
            )
            .await
        }


        // ── Issue #259: edit + delete at the HTTP boundary ──────────────────

        /// Creates `greeter` and returns its current version token.
        pub(crate) async fn create_greeter(state: &AppState) -> String {
            let response = router(state.clone())
                .oneshot(request(
                    "POST",
                    "/api/v1/company/workflows",
                    Some(create_body()),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let created = json_body(response).await;
            // A freshly created overlay graph is editable and carries a token.
            assert_eq!(created["editable"], true, "{created}");
            created["version"]
                .as_str()
                .unwrap_or_else(|| panic!("create must return a version token: {created}"))
                .to_string()
        }


        /// `create_body()` with a schedule on the trigger and a changed
        /// description — the exact "I typo'd my cron" edit the issue is about.
        pub(crate) fn edited_body(expected_version: Option<&str>) -> serde_json::Value {
            let mut body = serde_json::json!({
                "id": "greeter",
                "name": "Greeter",
                "description": "Say hi, every morning.",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * *" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
            });
            if let Some(v) = expected_version {
                body["expectedVersion"] = serde_json::json!(v);
            }
            body
        }


        // ── Issue #274: revision history + rollback at the HTTP boundary ────

        /// Edits `greeter` once (adding a schedule) so exactly one revision — the
        /// original, schedule-less body — is captured, and returns the token of
        /// the now-current (scheduled) graph.
        pub(crate) async fn create_then_edit_greeter(state: &AppState) -> String {
            let version = create_greeter(state).await;
            let response = router(state.clone())
                .oneshot(request(
                    "PUT",
                    "/api/v1/company/workflows/greeter",
                    Some(edited_body(Some(&version))),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            json_body(response).await["version"]
                .as_str()
                .expect("new token")
                .to_string()
        }


        // ── Issue #1009: settle eternal-`running` rows on the read ─────────

        /// Every `WorkflowRunFinished` the company journaled carrying `run_id`.
        pub(crate) async fn finishes_for(
            state: &AppState,
            id: &CompanyId,
            run_id: &str,
        ) -> Vec<(Option<String>, bool)> {
            let runtime = state.registry().get(id).expect("registered");
            runtime
                .events()
                .read_from(id, crate::ports::types::EventSeq::new(0), usize::MAX)
                .await
                .expect("read")
                .into_iter()
                .filter_map(|s| match s.event {
                    CompanyEvent::WorkflowRunFinished {
                        run_id: Some(rid),
                        error,
                        cancelled,
                        ..
                    } if rid == run_id => Some((error, cancelled)),
                    _ => None,
                })
                .collect()
        }

}

pub(crate) mod running {
        pub(crate) use std::sync::Arc;
        pub(crate) use std::sync::atomic::{AtomicBool, Ordering};

        pub(crate) use axum::body::{Body, to_bytes};
        pub(crate) use axum::http::{Request, StatusCode};
        pub(crate) use tower::ServiceExt;

        pub(crate) use crate::company::CompanyManifest;
        pub(crate) use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord, EventSeq};
        pub(crate) use crate::ports::{CompanyStore, WorkflowRun, WorkflowRunContext, WorkflowRunner};
        pub(crate) use crate::runtime::RuntimeBuilder;
        pub(crate) use crate::server::router;
        pub(crate) use crate::store::FsCompanyStore;
        pub(crate) use crate::{AppConfig, AppState};


        /// A runner that parks until released, and settles as cancelled if the
        /// run's stop signal fires first.
        ///
        /// It is the real `WorkflowRunner` port, so everything above it — the
        /// route, the supervisor registration, the spawned task, the journal
        /// write — is production code. Only the graph walk is stubbed, which is
        /// what lets these tests be about the *entry point* rather than about
        /// the engine (the engine's own cancel behaviour is pinned in
        /// `workflows::runner`).
        pub(crate) struct StalledRunner {
            pub(crate) entered: Arc<tokio::sync::Notify>,
            pub(crate) release: Arc<tokio::sync::Notify>,
            /// Set only if the run was allowed to finish on its own terms —
            /// which is how a test tells "the run completed" from "the run was
            /// dropped with the connection".
            pub(crate) completed: Arc<AtomicBool>,
        }


        #[async_trait::async_trait]
        impl WorkflowRunner for StalledRunner {
            async fn run(
                &self,
                _company: &CompanyId,
                _workflow: &crate::company::WorkflowFile,
                _input: serde_json::Value,
                ctx: &WorkflowRunContext,
            ) -> crate::Result<WorkflowRun> {
                // `notify_one` on both, not `notify_waiters`: a permit is
                // stored, so neither side has to be already parked. The detached
                // run answers before its task has even been polled, so a test
                // that waits on `entered` afterwards would otherwise race the
                // notification and hang.
                self.entered.notify_one();
                let released = self.release.notified();
                tokio::select! {
                    () = released => {}
                    () = ctx.cancel.cancelled() => {
                        return Ok(WorkflowRun {
                            output: serde_json::Value::Null,
                            pending_approvals: Vec::new(),
                            deliveries: Vec::new(),
                            cancelled: true,
                            nodes: Vec::new(),
                            notices: Vec::new(),
                            board: Vec::new(),
                            blocked_nodes: Vec::new(),
                            approvals: Vec::new(),
                        });
                    }
                }
                self.completed.store(true, Ordering::SeqCst);
                Ok(WorkflowRun {
                    output: serde_json::json!({ "run": {}, "nodes": {} }),
                    pending_approvals: Vec::new(),
                    deliveries: Vec::new(),
                    cancelled: false,
                    nodes: Vec::new(),
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                })
            }
        }


        pub(crate) struct Stalled {
            pub(crate) app: axum::Router,
            pub(crate) runtime: Arc<crate::company::runtime::CompanyRuntime>,
            pub(crate) entered: Arc<tokio::sync::Notify>,
            pub(crate) release: Arc<tokio::sync::Notify>,
            pub(crate) completed: Arc<AtomicBool>,
        }


        pub(crate) const GRAPH: &str = r#"
    id = "demo"
    name = "Demo"
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    [[node]]
    id = "done"
    kind = "output"
    name = "Report"
    [[edge]]
    from = "start"
    to = "done"
    label = "ok"
    "#;


        pub(crate) fn home() -> tempfile::TempDir {
            tempfile::Builder::new()
                .prefix("oc-workflows-running-")
                .tempdir()
                .expect("tempdir")
        }


        /// A hosted company with one overlay workflow and a runner that stalls.
        pub(crate) async fn stalled_company(home: &std::path::Path) -> Stalled {
            let manifest: CompanyManifest =
                toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
            let id = CompanyId::new("acme");
            FsCompanyStore::new(home.to_path_buf())
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    overlay_retired_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    id: id.clone(),
                    manifest: manifest.clone(),
                    ledger: Vec::new(),
                    overlay_agents: Vec::new(),
                    overlay_desk_members: Vec::new(),
                    overlay_desk_order: Vec::new(),
                    overlay_desks: Vec::new(),
                    overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                        id: "demo".to_string(),
                        toml: GRAPH.to_string(),
                    }],
                    overlay_budgets: Vec::new(),
                    overlay_policy: None,
                    overlay_tool_grants: None,
                    overlay_desk_tools: Default::default(),
                    disabled_workflows: Vec::new(),
                    lifecycle: "running".to_string(),
                    template_provenance: None,
                    setup: None,
                    name_confirmed: false,
                    activation_completed_at: None,
                    created_at_millis: None,
                })
                .await
                .unwrap();

            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let completed = Arc::new(AtomicBool::new(false));
            let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
                .with_id(id.clone())
                .build()
                .await
                .unwrap();
            runtime.set_workflow_runner(Arc::new(StalledRunner {
                entered: entered.clone(),
                release: release.clone(),
                completed: completed.clone(),
            }));
            let runtime = Arc::new(runtime);

            let state = AppState::new(AppConfig::default());
            state.registry().insert(id.clone(), runtime.clone());
            crate::server::test_support::seed_fixed_admin(&state, "acme").await;

            Stalled {
                app: router(state),
                runtime,
                entered,
                release,
                completed,
            }
        }


        pub(crate) fn run_request(body: serde_json::Value) -> Request<Body> {
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/workflows/demo/run")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap()
        }


        pub(crate) fn cancel_request(run_id: &str) -> Request<Body> {
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/workflows/runs/{run_id}/cancel"))
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap()
        }


        pub(crate) fn get_workflow_request() -> Request<Body> {
            Request::builder()
                .uri("/api/v1/company/workflows/demo")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap()
        }


        pub(crate) fn delete_workflow_request(version: &str) -> Request<Body> {
            Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/v1/company/workflows/demo?expectedVersion={version}"
                ))
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap()
        }


        pub(crate) async fn json_body(response: axum::response::Response) -> serde_json::Value {
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        }


        /// Every event the company journaled, oldest first.
        pub(crate) async fn journal(
            runtime: &Arc<crate::company::runtime::CompanyRuntime>,
        ) -> Vec<CompanyEvent> {
            runtime
                .events()
                .read_from(runtime.id(), EventSeq::new(0), usize::MAX)
                .await
                .expect("read")
                .into_iter()
                .map(|s| s.event)
                .collect()
        }


        /// Waits (bounded) for a `WorkflowRunFinished` to appear.
        pub(crate) async fn await_finished(
            runtime: &Arc<crate::company::runtime::CompanyRuntime>,
        ) -> Option<CompanyEvent> {
            for _ in 0..200 {
                if let Some(event) = journal(runtime)
                    .await
                    .into_iter()
                    .find(|e| matches!(e, CompanyEvent::WorkflowRunFinished { .. }))
                {
                    return Some(event);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            None
        }


        // ── Issue #542: dry run through the real route ──────────────────────

        /// A runner that completes immediately, returning one node row — enough
        /// to prove the route maps `WorkflowRun.nodes` onto the response and
        /// echoes the request's `dry_run` as the discriminator.
        pub(crate) struct EchoRunner;


        #[async_trait::async_trait]
        impl WorkflowRunner for EchoRunner {
            async fn run(
                &self,
                _company: &CompanyId,
                _workflow: &crate::company::WorkflowFile,
                _input: serde_json::Value,
                _ctx: &WorkflowRunContext,
            ) -> crate::Result<WorkflowRun> {
                Ok(WorkflowRun {
                    output: serde_json::json!({ "run": {}, "nodes": {} }),
                    pending_approvals: Vec::new(),
                    deliveries: Vec::new(),
                    cancelled: false,
                    nodes: vec![crate::ports::WorkflowRunNodeRow {
                        node_id: "done".to_string(),
                        status: crate::ports::types::WorkflowNodeStatus::Ok,
                        elapsed_ms: 3,
                        diagnostics: Vec::new(),
                    }],
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                })
            }
        }


        /// A runner that settles cleanly and hands back one delivery row —
        /// every node `ok`, no error, nothing cancelled, and a report that did
        /// not go out. The exact shape issue #981 caught reading green.
        pub(crate) struct DroppedReportRunner;


        /// A runner whose only delivery row is a **dry run**'s (issue #542): the
        /// report was routed as far as its destination and deliberately not
        /// dispatched. The row shape a real `deliver_outputs_dry` writes.
        pub(crate) struct DryRunRunner;


        #[async_trait::async_trait]
        impl WorkflowRunner for DryRunRunner {
            async fn run(
                &self,
                _company: &CompanyId,
                _workflow: &crate::company::WorkflowFile,
                _input: serde_json::Value,
                _ctx: &WorkflowRunContext,
            ) -> crate::Result<WorkflowRun> {
                Ok(WorkflowRun {
                    output: serde_json::json!({ "run": {}, "nodes": {} }),
                    pending_approvals: Vec::new(),
                    deliveries: vec![crate::ports::DeliveryReport {
                        node: "done".to_string(),
                        kind: "channel".to_string(),
                        target: Some("engineering".to_string()),
                        status: crate::ports::DeliveryStatus::Skipped,
                        detail: "this was a test run — nothing was sent".to_string(),
                        reason: crate::ports::DeliveryReason::DryRun,
                    }],
                    cancelled: false,
                    nodes: vec![crate::ports::WorkflowRunNodeRow {
                        node_id: "done".to_string(),
                        status: crate::ports::types::WorkflowNodeStatus::Ok,
                        elapsed_ms: 3,
                        diagnostics: Vec::new(),
                    }],
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                })
            }
        }


        #[async_trait::async_trait]
        impl WorkflowRunner for DroppedReportRunner {
            async fn run(
                &self,
                _company: &CompanyId,
                _workflow: &crate::company::WorkflowFile,
                _input: serde_json::Value,
                _ctx: &WorkflowRunContext,
            ) -> crate::Result<WorkflowRun> {
                Ok(WorkflowRun {
                    output: serde_json::json!({ "run": {}, "nodes": {} }),
                    pending_approvals: Vec::new(),
                    deliveries: vec![crate::ports::DeliveryReport {
                        node: "done".to_string(),
                        kind: "channel".to_string(),
                        target: Some("operator".to_string()),
                        status: crate::ports::DeliveryStatus::Failed,
                        detail: "`operator` is not an automation delivery channel".to_string(),
                        reason: crate::ports::DeliveryReason::ChannelNotWired,
                    }],
                    cancelled: false,
                    nodes: vec![crate::ports::WorkflowRunNodeRow {
                        node_id: "done".to_string(),
                        status: crate::ports::types::WorkflowNodeStatus::Ok,
                        elapsed_ms: 3,
                        diagnostics: Vec::new(),
                    }],
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                })
            }
        }


        /// A hosted company whose runner echoes immediately.
        pub(crate) async fn echo_company(home: &std::path::Path) -> axum::Router {
            company_with_runner(home, Arc::new(EchoRunner)).await
        }


        /// A hosted company with one overlay graph and the given runner behind
        /// the port, so the route, the supervisor and the journal write are all
        /// production code and only the graph walk is stubbed.
        pub(crate) async fn company_with_runner(
            home: &std::path::Path,
            runner: Arc<dyn WorkflowRunner>,
        ) -> axum::Router {
            let manifest: CompanyManifest =
                toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
            let id = CompanyId::new("acme");
            FsCompanyStore::new(home.to_path_buf())
                .save(&CompanyRecord {
                    overlay_desk_hive: Vec::new(),
                    overlay_retired_agents: Vec::new(),
                    overlay_agent_edits: Vec::new(),
                    id: id.clone(),
                    manifest: manifest.clone(),
                    ledger: Vec::new(),
                    overlay_agents: Vec::new(),
                    overlay_desk_members: Vec::new(),
                    overlay_desk_order: Vec::new(),
                    overlay_desks: Vec::new(),
                    overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                        id: "demo".to_string(),
                        toml: GRAPH.to_string(),
                    }],
                    overlay_budgets: Vec::new(),
                    overlay_policy: None,
                    overlay_tool_grants: None,
                    overlay_desk_tools: Default::default(),
                    disabled_workflows: Vec::new(),
                    lifecycle: "running".to_string(),
                    template_provenance: None,
                    setup: None,
                    name_confirmed: false,
                    activation_completed_at: None,
                    created_at_millis: None,
                })
                .await
                .unwrap();
            let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
                .with_id(id.clone())
                .build()
                .await
                .unwrap();
            runtime.set_workflow_runner(runner);
            let state = AppState::new(AppConfig::default());
            state.registry().insert(id.clone(), Arc::new(runtime));
            crate::server::test_support::seed_fixed_admin(&state, "acme").await;
            router(state)
        }

}

