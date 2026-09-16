fn own_rows(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
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

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{
    CompanyEvent, DEFAULT_RUN_LIMIT, MAX_RUN_ARTIFACTS, WorkflowNodeStatus,
    WorkflowRunOutcome, WorkflowRunVerdict, select_run_page,
};
use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-workflows-hosted-")
        .tempdir()
        .expect("tempdir")
}

/// A manifest declaring one enabled workflow — mirrors what a
/// platform tenant provisions with, minus any `workflows/` directory
/// on disk (there isn't one: hosted tenants have no source dir).
fn manifest_with_enabled() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[workflows]\nenabled = [\"demo\"]\n",
    )
    .unwrap()
}

/// Builds a running company whose runtime has **no source directory**
/// (built without `with_seed_dir`, matching how the platform builds a
/// provisioned tenant) but whose persisted record declares an enabled
/// workflow — the exact hosted-mode gap #70 reports.
async fn state_with_hosted_company(home: &std::path::Path) -> AppState {
    state_with_hosted_company_lifecycle(home, "running").await
}

/// The same fixture at a chosen lifecycle, so a paused company is
/// reachable without a second copy of the record literal.
async fn state_with_hosted_company_lifecycle(
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

/// **Route-ordering pin.** `runs` is a syntactically valid `wid`, so
/// `GET /workflows/runs` overlaps `GET /workflows/{wid}`. Axum prefers
/// the static segment; if that ever changed, the history panel would
/// silently 404 (there is no workflow named `runs`) instead of failing
/// loudly. Same trade, and same pin, as `GET /tasks/inflight`.
#[tokio::test]
async fn run_history_is_not_shadowed_by_the_graph_read() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    journal_run(&state, &id, "digest", true, Vec::new(), None).await;

    let response = router(state)
        .oneshot(request("GET", "/api/v1/company/workflows/runs", None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the static /workflows/runs must win over /workflows/{{wid}}"
    );
    // A runs page — `{ runs: [...], hasMore }` — not a single graph object.
    let body = json_body(response).await;
    assert!(
        body["runs"].is_array(),
        "graph read shadowed the history: {body}"
    );
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
fn page_run(seq: u64, at_millis: u64) -> WorkflowRunOutcome {
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
fn page(
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
const REGRESSED: [(u64, u64); 5] = [
    (10, 1_000),
    (20, 2_000),
    (30, 3_000),
    // NTP correction / VM resume / an operator setting the date: the
    // append order is unchanged, the timestamp goes backwards.
    (40, 1_500),
    (50, 5_000),
];

/// **The issue #1012 follow-up pin.** Paging must reach every run, and
/// a run whose `at_millis` regressed below the page boundary's is the
/// one that used to be lost.
///
/// Pre-fix (cut on `(at_millis, seq)`, cursor derived from the last
/// displayed row) the first `limit=2` page is `[50, 30]` — run 40 sorts
/// below 30 on time and is truncated away — and the cursor is 30, which
/// excludes everything at or above `seq` 30. Run 40 is on neither page,
/// and since the boundary only descends, on no later page either.
#[test]
fn a_clock_regressed_run_is_reachable_on_a_later_page() {
    let mut seen: Vec<u64> = Vec::new();
    let mut cursor: Option<u64> = None;
    // Bounded so a cursor that fails to advance fails the test instead
    // of hanging it.
    for _ in 0..10 {
        let (rows, has_more, next) = page(&REGRESSED, cursor, 2);
        seen.extend(rows.iter().map(|run| run.seq));
        if !has_more {
            assert!(next.is_none(), "a last page must issue no cursor");
            break;
        }
        let next = next.expect("a page with more behind it must issue a cursor");
        assert!(
            cursor.is_none_or(|previous| next < previous),
            "the cursor must descend, or the walk cannot terminate"
        );
        cursor = Some(next);
    }

    seen.sort_unstable();
    assert_eq!(
        seen,
        vec![10, 20, 30, 40, 50],
        "every journaled run must be reachable by paging; \
         40 is the clock-regressed one"
    );
}

/// The property the design rests on, asserted directly rather than
/// inferred from a walk: the page and the cursor **partition** the
/// candidate set. Everything served is at or above the cursor,
/// everything withheld is strictly below it — so the next request,
/// which asks for `seq < cursor`, gets exactly the remainder with no
/// overlap and no gap.
#[test]
fn the_page_cursor_partitions_the_run_set() {
    let (rows, has_more, next) = page(&REGRESSED, None, 2);
    assert!(has_more);
    let cursor = next.expect("cursor");

    let served: Vec<u64> = rows.iter().map(|run| run.seq).collect();
    assert!(
        served.iter().all(|seq| *seq >= cursor),
        "a served run below the cursor would be served twice: {served:?} vs {cursor}"
    );
    let withheld: Vec<u64> = REGRESSED
        .iter()
        .map(|(seq, _)| *seq)
        .filter(|seq| !served.contains(seq))
        .collect();
    assert!(
        withheld.iter().all(|seq| *seq < cursor),
        "a withheld run at or above the cursor is unreachable forever: \
         {withheld:?} vs {cursor}"
    );
    assert_eq!(
        served.len() + withheld.len(),
        REGRESSED.len(),
        "the two halves must account for every run"
    );
}

/// Issue #228 / #1272's ordering is untouched: the cut is keyed on
/// `seq`, but what comes back is still sorted newest **finish** first,
/// on the very `(at_millis, seq)` pair each row displays.
#[test]
fn a_page_is_still_displayed_newest_finish_first() {
    // Within one page, `seq` order and finish order disagree: 40 was
    // appended last but finished (by the clock) before 30.
    let (rows, _, _) = page(&[(30, 3_000), (40, 1_500), (50, 5_000)], None, 3);
    let order: Vec<u64> = rows.iter().map(|run| run.seq).collect();
    assert_eq!(
        order,
        vec![50, 30, 40],
        "the page must be listed by finish time, not by the key it was cut on"
    );
}

/// No older page, no cursor. A cursor for a page that does not exist
/// invites a caller to ask for it, and an absent field is also what
/// tells an older console to fall back to its own derivation.
#[test]
fn next_before_seq_is_absent_when_there_is_no_older_page() {
    let (rows, has_more, next) = page(&REGRESSED, None, 50);
    assert_eq!(rows.len(), 5);
    assert!(!has_more);
    assert_eq!(next, None);

    // Exactly `limit` runs is still "no older page" — the read stops at
    // `limit + 1` precisely so this case is knowable.
    let (rows, has_more, next) = page(&REGRESSED, None, 5);
    assert_eq!(rows.len(), 5);
    assert!(!has_more);
    assert_eq!(next, None);
}

/// The one part the pure function cannot cover: the cursor reaches the
/// console, under the camelCase name the client reads, and feeding it
/// back gets the next page with no repeat and no gap.
#[tokio::test]
async fn run_history_issues_the_page_cursor_on_the_wire() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;

    for _ in 0..5 {
        journal_run(&state, &id, "digest", false, Vec::new(), None).await;
    }

    let fetch = |uri: String| {
        let state = state.clone();
        async move {
            json_body(
                router(state)
                    .oneshot(request("GET", &uri, None))
                    .await
                    .unwrap(),
            )
            .await
        }
    };
    let seqs = |body: &serde_json::Value| -> Vec<u64> {
        body["runs"]
            .as_array()
            .expect("array")
            .iter()
            .map(|row| row["seq"].as_u64().expect("seq"))
            .collect()
    };

    let first =
        fetch("/api/v1/company/workflows/runs?workflow=digest&limit=2".to_string()).await;
    assert_eq!(first["hasMore"], true, "{first}");
    let cursor = first["nextBeforeSeq"]
        .as_u64()
        .unwrap_or_else(|| panic!("nextBeforeSeq must be on the wire: {first}"));
    let page_one = seqs(&first);
    assert_eq!(page_one.len(), 2, "{first}");
    assert_eq!(
        cursor,
        *page_one.iter().min().expect("min"),
        "the cursor is the page's low-water seq: {first}"
    );

    let second = fetch(format!(
        "/api/v1/company/workflows/runs?workflow=digest&limit=2&before_seq={cursor}"
    ))
    .await;
    let page_two = seqs(&second);
    assert!(
        page_two.iter().all(|seq| *seq < cursor),
        "the next page must be strictly below the cursor: {second}"
    );
    assert!(
        page_two.iter().all(|seq| !page_one.contains(seq)),
        "no run may appear on two pages: {page_one:?} then {page_two:?}"
    );

    let last_cursor = second["nextBeforeSeq"]
        .as_u64()
        .expect("a third page exists");
    let third = fetch(format!(
        "/api/v1/company/workflows/runs?workflow=digest&limit=2&before_seq={last_cursor}"
    ))
    .await;
    let page_three = seqs(&third);
    assert_eq!(third["hasMore"], false, "{third}");
    assert!(
        third.get("nextBeforeSeq").is_none(),
        "the last page must omit the cursor entirely: {third}"
    );

    // No gap: the three pages are the whole history, once each.
    let mut walked: Vec<u64> = page_one;
    walked.extend(page_two);
    walked.extend(page_three);
    walked.sort_unstable();
    walked.dedup();
    assert_eq!(
        walked.len(),
        5,
        "paging must reach all five runs: {walked:?}"
    );
}

// -------------------------------------------------------------------
// Cron preview (issue #262)
// -------------------------------------------------------------------

/// 2026-08-02 12:00 UTC, as epoch millis — the `after` pin every
/// preview test searches forward from, so the answers are fixed rather
/// than relative to whenever CI runs.
const AFTER: u64 = 1_785_672_000_000;

async fn preview(state: &AppState, expr: &str) -> serde_json::Value {
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

/// **The issue #262 pin.** `0 9 * * *` and `9 0 * * *` are two
/// characters apart, both valid, and nine hours different. The preview
/// is the only thing that tells them apart before the report arrives at
/// the wrong time, so the distinction is pinned end-to-end over HTTP,
/// not just in the matcher's unit tests.
#[tokio::test]
async fn cron_preview_distinguishes_nine_am_from_nine_past_midnight() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let morning = preview(&state, "0 9 * * *").await;
    assert_eq!(morning["description"], "Every day at 09:00 UTC");
    // 2026-08-02 12:00 UTC → the next 09:00 is the following morning.
    assert_eq!(morning["next"][0], 1_785_747_600_000u64);
    assert_eq!(morning["next"].as_array().unwrap().len(), 3);

    let midnight = preview(&state, "9 0 * * *").await;
    assert_eq!(midnight["description"], "Every day at 00:09 UTC");
    assert_ne!(morning["next"][0], midnight["next"][0]);
}

/// A shape the humaniser declines to paraphrase still previews: the
/// description is `null` and the fire times carry the meaning. The
/// console shows "Next runs: …" rather than nothing.
#[tokio::test]
async fn cron_preview_returns_fires_without_a_description() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let body = preview(&state, "0 0 1 * *").await;
    assert!(body["description"].is_null(), "{body}");
    assert_eq!(body["next"].as_array().unwrap().len(), 3, "{body}");
}

/// **Malformed input answers 200, not 4xx.** The console previews while
/// the author is still typing, so a half-written expression is the
/// normal live state — and the console's client throws on any non-2xx,
/// which would turn every keystroke into a caught exception. The parser
/// message rides in the body instead.
#[tokio::test]
async fn cron_preview_reports_a_parse_error_as_a_200_body() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let response = router(state)
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/cron/preview",
            Some(serde_json::json!({ "expr": "every day" })),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let error = body["error"].as_str().expect("an error message: {body}");
    assert!(error.contains("5 fields"), "{body}");
    assert!(body["next"].is_null(), "no fire times on a parse error");
}

/// **Route-ordering pin**, the same trade `/workflows/runs` takes:
/// `cron` is a syntactically valid `wid`, so the static preview path is
/// registered before `/workflows/{wid}`. A regression would route the
/// preview into the graph read and 404.
#[tokio::test]
async fn cron_preview_is_not_shadowed_by_the_graph_read() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let response = router(state)
        .oneshot(request(
            "POST",
            "/api/v1/companies/acme/workflows/cron/preview",
            Some(serde_json::json!({ "expr": "0 9 * * MON" })),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the static /workflows/cron/preview must win over /workflows/{{wid}}"
    );
    let body = json_body(response).await;
    assert_eq!(body["description"], "Every Mon at 09:00 UTC", "{body}");
}

/// Both scope forms serve the history — the platform
/// `…/companies/{id}/…` address as well as the prosumer alias.
#[tokio::test]
async fn run_history_serves_both_scope_forms() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    journal_run(&state, &id, "digest", true, Vec::new(), None).await;

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/companies/acme/workflows/runs",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["runs"][0]["workflowId"], "digest");
}

// ── Issue #259: edit + delete at the HTTP boundary ──────────────────

/// Creates `greeter` and returns its current version token.
async fn create_greeter(state: &AppState) -> String {
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
fn edited_body(expected_version: Option<&str>) -> serde_json::Value {
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

/// **Regression, issue #1882 review — the round-trip data-loss bug.**
/// An `ownerDesk` set on create must survive an edit that never
/// touches it. Before the fix, `ownerDesk` was accepted on the write
/// body but never appeared on a `GET`/`PUT` response, so a caller that
/// builds its edit request from what it just read (the only sane way
/// to satisfy `PUT`'s "send the whole graph back" contract) carried no
/// `ownerDesk` forward — the very next save silently cleared the desk
/// an operator had set. This constructs the edit body the same way a
/// conformant client does: by mutating the read response, not by
/// hand-copying the field.
#[tokio::test]
async fn owner_desk_survives_an_unrelated_edit() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = desk_state(&home).await;

    let mut body = create_body();
    body["ownerDesk"] = serde_json::json!("engineering");
    let response = router(state.clone())
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created = json_body(response).await;
    assert_eq!(
        created["ownerDesk"], "engineering",
        "the create response must echo the desk the caller set: {created}"
    );

    let response = router(state.clone())
        .oneshot(request("GET", "/api/v1/company/workflows/greeter", None))
        .await
        .unwrap();
    let mut graph = json_body(response).await;
    assert_eq!(
        graph["ownerDesk"], "engineering",
        "a read must project the desk a create just set: {graph}"
    );

    // Build the edit the way a conformant client does: from the graph
    // it just read, changing only the one field it means to change.
    let version = graph["version"].as_str().unwrap().to_string();
    graph["description"] = serde_json::json!("Say hi, differently.");
    graph["expectedVersion"] = serde_json::json!(version);

    let response = router(state.clone())
        .oneshot(request(
            "PUT",
            "/api/v1/company/workflows/greeter",
            Some(graph),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = router(state)
        .oneshot(request("GET", "/api/v1/company/workflows/greeter", None))
        .await
        .unwrap();
    let after = json_body(response).await;
    assert_eq!(
        after["ownerDesk"], "engineering",
        "an edit that never touched ownerDesk must not clear it: {after}"
    );
}

/// **The issue, at the HTTP boundary.** A saved workflow's cron was
/// permanent; now it can be corrected and the correction reads back.
#[tokio::test]
async fn edit_replaces_the_graph_and_reads_back() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, _id) = hosted_state(&home).await;
    let version = create_greeter(&state).await;

    let response = router(state.clone())
        .oneshot(request(
            "PUT",
            "/api/v1/company/workflows/greeter",
            Some(edited_body(Some(&version))),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = json_body(response).await;
    assert_eq!(updated["nodes"][0]["schedule"], "0 9 * * *");
    // The response carries the NEW token, so a second save needs no
    // intervening read.
    let next = updated["version"].as_str().expect("new token").to_string();
    assert_ne!(next, version, "the token must move with the body");

    // And a fresh read agrees — the edit is what the read path serves.
    let response = router(state.clone())
        .oneshot(request("GET", "/api/v1/company/workflows/greeter", None))
        .await
        .unwrap();
    let graph = json_body(response).await;
    assert_eq!(graph["nodes"][0]["schedule"], "0 9 * * *");
    assert_eq!(graph["description"], "Say hi, every morning.");
    assert_eq!(graph["version"], next.as_str());

    // Still exactly one workflow — an edit replaces, never forks.
    let response = router(state)
        .oneshot(request("GET", "/api/v1/company/workflows", None))
        .await
        .unwrap();
    let items = json_body(response).await;
    assert_eq!(own_rows(&items).len(), 1, "{items}");
}

// ── Issue #274: revision history + rollback at the HTTP boundary ────

/// Edits `greeter` once (adding a schedule) so exactly one revision — the
/// original, schedule-less body — is captured, and returns the token of
/// the now-current (scheduled) graph.
async fn create_then_edit_greeter(state: &AppState) -> String {
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

