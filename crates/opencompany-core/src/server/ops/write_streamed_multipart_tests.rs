//! Integration tests for the `ops` write plane: tasks, memory, workspace,
//! skills, team, inbox-read, and desk chat — exercised end-to-end over the
//! router against a real fs-backed company.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::company::steer::{InflightEntry, InflightKind};
use crate::ports::facts::{FactKind, FactRecord};
use crate::ports::tasks::{TaskRecord, TaskTitle};
use crate::ports::types::{CompanyId, CompanyRecord, CompressedTrace, ContextChunk};
use crate::runtime::RuntimeBuilder;
use super::tests_an_uploaded_image_round::{OVERSIZE_BOUNDARY, post_upload};
use super::tests_applying_a_proposal_with::upload_file;
use crate::runtime::journal::{ApprovalConversation, TaskLink};
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};
use super::write_test_support::*;

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("opencompany-ops-")
        .tempdir()
        .expect("tempdir")
}

fn manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
    )
    .unwrap()
}

/// The sorted node names in a workspace tree body.
///
/// A freshly-built company is no longer an empty tree: boot scaffolds the
/// reserved `agents/` and `desks/` roots (issue #551), so the tests below name
/// what they expect rather than counting to zero. Nothing is provisioned
/// *inside* them — a member folder is minted when that agent or desk first
/// produces something.
fn provisioned_names(tree: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = tree
        .as_array()
        .expect("the tree read is an array")
        .iter()
        .map(|node| node["name"].as_str().unwrap_or_default().to_string())
        .collect();
    names.sort();
    names
}

async fn state_with_company(home: &std::path::Path) -> AppState {
    state_with_quota(home, crate::runtime::WorkspaceQuota::default()).await
}

/// [`state_with_company`], with the workspace held to `quota`.
///
/// Parameterised rather than duplicated so the one test that needs a non-default
/// `[workspace] max_blob_mb` (issue #647) exercises the same wiring every other
/// test here does, instead of a second harness that could drift from it.
async fn state_with_quota(
    home: &std::path::Path,
    quota: crate::runtime::WorkspaceQuota,
) -> AppState {
    state_with(home, quota, None).await
}

/// [`state_with_company`], with the workspace tree served by `workspace`
/// (issue #759).
///
/// The `fs` backend refuses to create two sibling nodes with one name
/// (`reject_path_collision`, issue #665), so the raced tree the repair route
/// exists to fix cannot be built through it. sqlite and mongodb — the backends
/// hosted tenants run, and the reason the state exists at all — accept it, and
/// this swaps in a double that behaves the same way. Everything else about the
/// harness is unchanged, so the route under test is the one the console calls.
async fn state_with_workspace(
    home: &std::path::Path,
    workspace: std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>,
) -> AppState {
    state_with(
        home,
        crate::runtime::WorkspaceQuota::default(),
        Some(workspace),
    )
    .await
}

async fn state_with(
    home: &std::path::Path,
    quota: crate::runtime::WorkspaceQuota,
    workspace: Option<std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>>,
) -> AppState {
    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest(),
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
    let mut builder = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(id.clone())
        .with_workspace_quota(quota);
    if let Some(workspace) = workspace {
        builder = builder.with_workspace(workspace);
    }
    let runtime = builder.build().await.unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    // Every route needs a principal now; the harness signs in as an admin so
    // tests keep asserting write behavior rather than auth.
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// The repo's `companies/` directory, whose bundles' `skills/` are the skill
/// registry — the same directory the serve path derives `skills_root` from.
fn repo_skills_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../companies")
}

/// Like [`state_with_company`], but with the repo's shipped bundles wired in as
/// the skill registry, so registry reads and server-authoritative installs
/// resolve against real documents instead of degrading to the empty-registry
/// fallback.
async fn state_with_registry(home: &std::path::Path) -> AppState {
    // `with_skills_root` consumes and returns the state, so the registered
    // company and seeded admin move along with it.
    state_with_company(home)
        .await
        .with_skills_root(repo_skills_root())
}

/// The operator deltas persisted for `acme` — the durable rows behind the API.
async fn persisted_skills(state: &AppState) -> Vec<crate::ports::skills_state::SkillState> {
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("company");
    runtime.skills().list(runtime.id()).await.expect("deltas")
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_auth(state, method, uri, body, None).await
}

async fn send_auth(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    } else {
        // No explicit credential: sign in as the harness admin. Every route
        // needs a principal now, so an unauthenticated request would only ever
        // assert 401 rather than the behavior under test.
        request = request.header("cookie", crate::server::test_support::fixed_cookie("acme"));
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A body of `prefix` + `payload` zero bytes + `suffix`, streamed in 1 MiB
/// frames with a yield before each.
///
/// The framing is load-bearing, not tidiness. A contiguous body would make the
/// *test process* hold the whole payload before the server saw a byte of it,
/// and the multipart reader drains every frame that is ready in one poll — so
/// an always-ready stream is buffered whole however finely it was cut up. The
/// yield keeps the reader one frame ahead of the parser rather than a whole
/// body ahead, which is what lets a 257 MiB request that the handler *skips*
/// cost about a megabyte instead of 257 of them.
fn streamed_multipart(prefix: Vec<u8>, payload: usize, suffix: Vec<u8>) -> Body {
    const FRAME: usize = 1024 * 1024;
    let prefix = std::sync::Arc::new(prefix);
    let suffix = std::sync::Arc::new(suffix);
    let filler = bytes::Bytes::from(vec![0u8; FRAME]);
    let frames = payload.div_ceil(FRAME);

    let stream = futures::stream::unfold(0usize, move |step| {
        let prefix = prefix.clone();
        let suffix = suffix.clone();
        let filler = filler.clone();
        async move {
            tokio::task::yield_now().await;
            let frame = if step == 0 {
                bytes::Bytes::from(prefix.as_ref().clone())
            } else if step <= frames {
                filler.slice(..(payload - (step - 1) * FRAME).min(FRAME))
            } else if step == frames + 1 {
                bytes::Bytes::from(suffix.as_ref().clone())
            } else {
                return None;
            };
            Some((Ok::<_, std::io::Error>(frame), step + 1))
        }
    });
    Body::from_stream(stream)
}

/// The opening of a `file` part, up to (not including) its bytes.
fn file_part_prefix(filename: &str) -> Vec<u8> {
    format!(
        "--{OVERSIZE_BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes()
}

/// The node names currently in the workspace tree.
async fn tree_names(state: &AppState) -> Vec<String> {
    let (status, tree) = send(state, "GET", "/api/v1/company/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    provisioned_names(&tree)
}

/// The headline of #647: a file over the store's per-file cap is refused as
/// **too large**, with the sentence an operator can act on.
///
/// This failed before the fix, and not subtly — the route's `DefaultBodyLimit`
/// was the same 64 MiB as the cap, so it truncated the body first and the
/// truncation surfaced as a parse failure: `400 invalid request: unreadable
/// file part: Error parsing multipart/form-data request`. A correctly-formed
/// request, described as broken, for a reason the operator could not guess.
/// The store's refusal below existed the whole time and could never be reached
/// through this route.
#[tokio::test]
async fn a_file_over_the_per_file_cap_is_refused_as_too_large_not_as_malformed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let before = tree_names(&state).await;

    // One megabyte over the 64 MiB default: enough to break the cap, nowhere
    // near the 256 MiB the route will now read.
    let oversize = 65 * 1024 * 1024;
    let (status, body) = post_upload(
        &state,
        streamed_multipart(
            file_part_prefix("hero.mov"),
            oversize,
            format!("\r\n--{OVERSIZE_BOUNDARY}--\r\n").into_bytes(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "workspace_quota_exceeded", "{body}");
    let message = body["error"].as_str().expect("an error message");
    assert!(message.contains("hero.mov"), "names the file: {message}");
    assert!(message.contains("65.0 MiB"), "names its size: {message}");
    assert!(message.contains("64.0 MiB"), "names the limit: {message}");
    assert!(message.contains("Nothing was stored"), "{message}");

    // The two words the bug used to answer with. Asserting on their absence is
    // the regression guard: a future change that lets the body limit preempt
    // the store again would put them straight back.
    assert!(
        !message.contains("unreadable file part"),
        "the request was not unreadable: {message}"
    );
    assert!(
        !message.contains("Error parsing"),
        "nor was it malformed: {message}"
    );

    assert_eq!(tree_names(&state).await, before, "and nothing was stored");
}

/// A file part declared as a texty type, so the upload takes the **text**
/// branch rather than the binary one.
fn text_file_part_prefix(filename: &str) -> Vec<u8> {
    format!(
        "--{OVERSIZE_BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n"
    )
    .into_bytes()
}

/// Issue #665: an over-cap upload is refused even when its bytes are valid
/// UTF-8.
///
/// The store's quota decorator meters **binary payloads only**, and that is a
/// deliberate narrowing — `src/runtime/workspace_quota.rs` says so, on the
/// grounds that "a note is bounded by what a model will emit into a tool call".
/// That premise holds for every writer the decorator covers and is false for
/// this route, which is where arbitrary operator-supplied bytes enter the tree.
///
/// So a 65 MiB `.csv` — valid UTF-8, therefore classified as prose — used to be
/// stored with **no size check at all**, while the byte-identical payload under
/// a binary content type was refused. Same request, same size, opposite answer,
/// decided by whether the bytes happened to decode.
///
/// The narrowing itself is untouched: an agent's note is still unmetered, and
/// `tree_quota_gb` still counts binary payloads alone.
#[tokio::test]
async fn an_over_cap_upload_is_refused_even_when_its_bytes_are_valid_utf8() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let before = tree_names(&state).await;

    // NUL bytes: valid UTF-8, so `text_body` decodes them and the upload takes
    // the text branch. One megabyte over the 64 MiB default cap.
    let oversize = 65 * 1024 * 1024;
    let (status, body) = post_upload(
        &state,
        streamed_multipart(
            text_file_part_prefix("export.csv"),
            oversize,
            format!("\r\n--{OVERSIZE_BOUNDARY}--\r\n").into_bytes(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "workspace_quota_exceeded", "{body}");
    let message = body["error"].as_str().expect("an error message");
    assert!(message.contains("export.csv"), "names the file: {message}");
    assert!(message.contains("65.0 MiB"), "names its size: {message}");
    assert!(message.contains("64.0 MiB"), "names the limit: {message}");
    assert!(message.contains("Nothing was stored"), "{message}");

    assert_eq!(tree_names(&state).await, before, "and nothing was stored");
}

/// The other half of #665, and the reason the fix is a cap rather than a
/// reclassification: an *under*-cap text upload is still stored as prose.
///
/// Refusing large text must not turn ordinary text uploads into opaque blobs — a
/// `.csv` an operator uploads is meant to stay searchable, backlinkable and
/// editable in the console. If this ever fails, the fix has started deciding
/// storage representation instead of bounding size.
#[tokio::test]
async fn an_under_cap_text_upload_is_still_stored_as_prose() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, node) = upload_file(
        &state,
        "notes.csv",
        Some("text/csv"),
        b"a,b,c\n1,2,3\n",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{node}");
    assert_eq!(
        node["content"], "a,b,c\n1,2,3\n",
        "a text upload keeps its body: {node}"
    );
    assert!(
        node.get("mime").is_none() || node["mime"].is_null(),
        "and is a prose note, not a binary payload: {node}"
    );
}

/// The route's own backstop is classified too, and it fires while *skipping* a
/// part — the reader can notice the limit anywhere it reads, not only where the
/// handler wants bytes.
///
/// Without this the classifier arm ships untested and a drift in axum's status
/// mapping would silently regress the answer to the old lying 400.
#[tokio::test]
async fn a_body_over_the_route_limit_is_classified_while_a_part_is_skipped() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let before = tree_names(&state).await;

    // A field the handler ignores by name, so it is drained rather than
    // buffered — and the drain runs past the 256 MiB the route will read.
    let prefix = format!(
        "--{OVERSIZE_BOUNDARY}\r\nContent-Disposition: form-data; name=\"ignored\"\r\n\r\n"
    )
    .into_bytes();
    let mut suffix = format!("\r\n--{OVERSIZE_BOUNDARY}\r\n").into_bytes();
    suffix.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n\r\nxx\r\n",
    );
    suffix.extend_from_slice(format!("--{OVERSIZE_BOUNDARY}--\r\n").as_bytes());

    let (status, body) = post_upload(
        &state,
        streamed_multipart(prefix, 257 * 1024 * 1024, suffix),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "workspace_quota_exceeded", "{body}");
    let message = body["error"].as_str().expect("an error message");
    assert!(
        message.contains("256.0 MiB"),
        "names the ceiling: {message}"
    );
    assert!(message.contains("Nothing was stored"), "{message}");
    // The size is deliberately absent: the body was cut off, so the true total
    // is not knowable here and a guess would be worse than silence.
    assert!(
        !message.contains("Error parsing"),
        "still not a parse failure: {message}"
    );

    assert_eq!(tree_names(&state).await, before, "and nothing was stored");
}

/// The same backstop, noticed at the other read site — while the handler is
/// pulling the `file` part's bytes rather than skipping past someone else's.
#[tokio::test]
async fn a_file_part_over_the_route_limit_is_classified_too() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let before = tree_names(&state).await;

    let (status, body) = post_upload(
        &state,
        streamed_multipart(
            file_part_prefix("enormous.bin"),
            257 * 1024 * 1024,
            format!("\r\n--{OVERSIZE_BOUNDARY}--\r\n").into_bytes(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "workspace_quota_exceeded", "{body}");
    let message = body["error"].as_str().expect("an error message");
    assert!(
        message.contains("256.0 MiB"),
        "names the ceiling: {message}"
    );
    assert!(
        !message.contains("unreadable file part"),
        "the part was readable, just too long: {message}"
    );

    assert_eq!(tree_names(&state).await, before, "and nothing was stored");
}

/// The counter-test, and the half of the issue that is easiest to lose: a
/// genuinely malformed body still answers 400.
///
/// Classifying by size must not swallow the case the old message was right
/// about. These two shapes stay `invalid_request` — and stay distinguishable
/// from the 413s above, which is the whole point of the change.
#[tokio::test]
async fn a_malformed_multipart_body_is_still_a_400() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // The declared boundary never appears.
    let (status, body) =
        post_upload(&state, Body::from(b"this is not a multipart body".to_vec())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "invalid_request", "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("malformed multipart upload")),
        "{body}"
    );

    // A part that opens and never closes: headers, some bytes, no terminating
    // boundary. Truncated — the shape the body limit used to be mistaken for.
    let mut unterminated = file_part_prefix("half.bin");
    unterminated.extend_from_slice(b"partial bytes and then nothing");
    let (status, body) = post_upload(&state, Body::from(unterminated)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "invalid_request", "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|m| m.contains("unreadable file part")),
        "{body}"
    );
}

// ---------------------------------------------------------------------------
// First-run company setup (docs/spec/runtime/company-setup.md)
// ---------------------------------------------------------------------------

/// The e-commerce worked example, end to end over the router.
///
/// The default test build has no harness, so this is the unpolished path — and
/// that is exactly the contract worth pinning: a company with no inference
/// credential still gets a real industry roster rather than an empty page or an
/// error. Decision D3's floor, asserted at the surface an operator meets.
#[tokio::test]
async fn setup_proposes_a_real_roster_with_no_model_wired() {
    let home = home();
    let state = state_with_company(home.path()).await;

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/setup/roster",
        Some(json!({
            "industry": "E-commerce — I sell homeware online",
            "teamHint": "",
            "automate": "Social media posts, Meta ads, generating my reports, order dispatch",
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["template"], "ecommerce", "{body}");
    assert_eq!(
        body["source"], "fallback",
        "no harness is wired, so the curated team ships: {body}"
    );
    let agents = body["agents"].as_array().expect("agents array");
    assert!(
        (4..=6).contains(&agents.len()),
        "a proposal must be a workable team, got {}: {body}",
        agents.len()
    );
    let roles: Vec<&str> = agents
        .iter()
        .map(|a| a["role"].as_str().unwrap_or_default())
        .collect();
    assert!(roles.contains(&"Logistics Coordinator"), "{roles:?}");
    // Every row must be directly usable as a `POST …/team` body — the console
    // passes them straight through, so a missing field would surface as a
    // half-created teammate rather than as a validation error here.
    for agent in agents {
        for field in ["name", "role", "description"] {
            assert!(
                agent[field].as_str().is_some_and(|v| !v.trim().is_empty()),
                "agent is missing `{field}`: {agent}"
            );
        }
    }
}

/// Setup proposes; it does not create. The roster must be untouched afterwards,
/// because the console is what creates each teammate — and because the empty
/// roster is also the "has setup run?" signal (decision D4), a route that
/// created them itself would answer that question before the operator had seen
/// a single name.
#[tokio::test]
async fn setup_creates_no_teammates_of_its_own() {
    let home = home();
    let state = state_with_company(home.path()).await;

    let (before_status, before) = send(&state, "GET", "/api/v1/company/team", None).await;
    assert_eq!(before_status, StatusCode::OK);
    let before_len = before.as_array().expect("roster").len();

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/setup/roster",
        Some(json!({ "industry": "content creator", "automate": "daily posts" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, after) = send(&state, "GET", "/api/v1/company/team", None).await;
    assert_eq!(
        after.as_array().expect("roster").len(),
        before_len,
        "setup created teammates itself: {after}"
    );
}

/// The answers are persisted, because Phase 2 builds this company's workflows
/// from them and must not have to ask a second time.
#[tokio::test]
async fn setup_remembers_the_answers() {
    let home = home();
    let state = state_with_company(home.path()).await;

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/setup/roster",
        Some(json!({
            "industry": "E-commerce",
            "teamHint": "plus customer support",
            "automate": "Meta ads, order dispatch",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.path().to_path_buf());
    let record = store
        .load(&CompanyId::new("acme"))
        .await
        .expect("load")
        .expect("record");
    let answers = record.setup.expect("the answers were stored");
    assert_eq!(answers.industry, "E-commerce");
    assert_eq!(answers.team_hint, "plus customer support");
    assert_eq!(answers.automate, "Meta ads, order dispatch");
}

/// An operator who types nothing still gets a team. The three questions are
/// free text and the last two are skippable, so an empty body is a real request
/// rather than a client bug — and stranding someone on the setup screen is the
/// one outcome worse than a generic roster.
#[tokio::test]
async fn setup_answers_an_empty_body_with_the_generic_team() {
    let home = home();
    let state = state_with_company(home.path()).await;

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/setup/roster",
        Some(json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["template"], "generic", "{body}");
    assert!(
        body["agents"].as_array().expect("agents").len() >= 4,
        "{body}"
    );
}

/// `[workspace] max_blob_mb` above the default is a real knob again.
///
/// It never was one: the route stopped reading at 64 MiB whatever a company had
/// configured, so raising the cap bought nothing but a different way to fail.
/// A company at 128 MiB can now actually store a 65 MiB file.
#[tokio::test]
async fn a_company_that_raised_its_blob_cap_can_use_it() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_quota(
        &home,
        crate::runtime::WorkspaceQuota {
            max_blob_bytes: 128 * 1024 * 1024,
            tree_quota_bytes: None,
        },
    )
    .await;

    let size = 65 * 1024 * 1024;
    let (status, node) = post_upload(
        &state,
        streamed_multipart(
            file_part_prefix("raised.bin"),
            size,
            format!("\r\n--{OVERSIZE_BOUNDARY}--\r\n").into_bytes(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{node}");
    assert_eq!(node["name"], "raised.bin");
    assert_eq!(node["size"], size as u64);
    assert!(tree_names(&state).await.contains(&"raised.bin".to_string()));
}

// ---------------------------------------------------------------------------
// Issue #705 — an irreversible effect's amount is admin-only
// ---------------------------------------------------------------------------
