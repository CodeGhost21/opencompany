use super::*;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::types::CompanyId;
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

const MANIFEST: &str = "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
     [policy]\nmode = \"supervised\"\n\
     always_approve = [\"payment.send\", \"filing.submit\"]\n\
     auto_approve_under_usd = 5.0\n\
     approval_ttl_hours = 24\n";

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-policy-")
        .tempdir()
        .expect("tempdir")
}

async fn state(home: &std::path::Path) -> AppState {
    let manifest: CompanyManifest = toml::from_str(MANIFEST).unwrap();
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// Same fixture as [`state`], but with an injected gate that has NOT been
/// built through the production path's `.with_policy_hitl_disabled()` —
/// the only way this suite can exercise `PolicyDto.policyHitlEnabled: true`
/// before any code path sets it live.
async fn state_with_policy_hitl_enabled(home: &std::path::Path) -> AppState {
    let manifest: CompanyManifest = toml::from_str(MANIFEST).unwrap();
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
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
            overlay_desk_hive: Vec::new(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let gate = std::sync::Arc::new(crate::policy::gate::ManifestApprovalGate::new(
        manifest.policy.clone(),
    ));
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .with_approvals(gate)
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

async fn call(state: &AppState, method: &str, body: Option<Value>) -> (StatusCode, Value) {
    call_as(
        state,
        method,
        body,
        Some(crate::server::test_support::fixed_cookie("acme")),
    )
    .await
}

/// [`call`], with the caller's cookie under the test's own control —
/// `None` for no session at all — instead of always the fixed admin.
async fn call_as(
    state: &AppState,
    method: &str,
    body: Option<Value>,
    cookie: Option<String>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri("/api/v1/company/policy")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let request = match body {
        Some(value) => request.body(Body::from(value.to_string())).unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// `GET` reports the manifest's policy when nothing is overridden, and says
/// so — `overridden` is what the console keys the reset control off.
#[tokio::test]
async fn get_reports_the_manifest_policy_when_nothing_is_overridden() {
    let dir = home();
    let state = state(dir.path()).await;
    let (status, body) = call(&state, "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "supervised");
    assert_eq!(body["manifestMode"], "supervised");
    assert_eq!(body["autoApproveUnderUsd"], 5.0);
    assert_eq!(body["manifestAutoApproveUnderUsd"], 5.0);
    assert_eq!(body["approvalTtlHours"], 24);
    assert_eq!(body["manifestApprovalTtlHours"], 24);
    assert_eq!(body["overridden"], false);
    assert!(body["setBy"].is_null());
    assert!(!body["tiers"].as_array().unwrap().is_empty());
}

/// `GET` also answers the console's "is this a real tool?" note: the
/// complete gateable registry, not the workflow-authorable subset. A wired
/// agent tool that cannot be a workflow node (`publish_artifact`,
/// `hosting_launch_site`) is still a fence the gate matches, so it must be
/// present — otherwise the console would call a working entry a mistake.
#[tokio::test]
async fn get_serves_the_complete_gateable_tool_registry() {
    let dir = home();
    let state = state(dir.path()).await;
    let (status, body) = call(&state, "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    let known_tools: &Vec<Value> = body["knownTools"]
        .as_array()
        .expect("knownTools is an array");
    assert!(
        known_tools.iter().any(|tool| tool == "publish_artifact"),
        "a wired agent tool the workflow catalog does not carry must be known"
    );
    assert!(
        known_tools.iter().any(|tool| tool == "shell"),
        "the registry still carries the workflow tools"
    );
}

/// `policyHitlEnabled` reports the live gate's own state, and every
/// production build reports it `false` — the console's honest "disabled"
/// copy must be reading a real fact, not a hardcoded one.
#[tokio::test]
async fn get_reports_policy_hitl_as_disabled_on_the_production_path() {
    let dir = home();
    let state = state(dir.path()).await;
    let (status, body) = call(&state, "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["policyHitlEnabled"], false);
}

/// The other half of the same fact: a gate that has NOT been built through
/// `.with_policy_hitl_disabled()` must report `true`, so the field really
/// does track the gate rather than always answering `false`.
#[tokio::test]
async fn get_reports_policy_hitl_as_enabled_when_the_gate_has_it_on() {
    let dir = home();
    let state = state_with_policy_hitl_enabled(dir.path()).await;
    let (status, body) = call(&state, "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["policyHitlEnabled"], true);
}

/// A tier `PUT` moves the tier and leaves the always-ask list on the
/// manifest's value — the independence the console's two controls rely on.
#[tokio::test]
async fn putting_a_tier_leaves_the_always_ask_list_alone() {
    let dir = home();
    let state = state(dir.path()).await;
    let (status, body) = call(&state, "PUT", Some(json!({ "mode": "full" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "full");
    assert_eq!(body["overridden"], true);
    assert_eq!(
        body["alwaysApprove"],
        json!(["payment.send", "filing.submit"]),
        "setting the tier must not discard the manifest's always-ask list"
    );
    // And it persisted, rather than only being reflected in the response.
    let (_, reread) = call(&state, "GET", None).await;
    assert_eq!(reread["mode"], "full");
}

/// An emptied always-ask list is stored as empty, not resolved back to the
/// manifest's three defaults.
#[tokio::test]
async fn an_emptied_always_ask_list_is_stored_as_empty() {
    let dir = home();
    let state = state(dir.path()).await;
    let (status, body) = call(&state, "PUT", Some(json!({ "alwaysApprove": [] }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["alwaysApprove"], json!([]));
    assert_eq!(body["mode"], "supervised", "the tier must not have moved");
}

/// The spend cap and deadline use the same field-wise write behaviour as
/// the tier: changing either leaves every other policy setting alone.
#[tokio::test]
async fn putting_a_cap_or_deadline_overrides_only_that_field() {
    let dir = home();
    let state = state(dir.path()).await;

    let (status, body) = call(
        &state,
        "PUT",
        Some(json!({ "autoApproveUnderUsd": null, "approvalTtlHours": 72 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["autoApproveUnderUsd"].is_null());
    assert_eq!(body["approvalTtlHours"], 72);
    assert_eq!(body["mode"], "supervised");
    assert_eq!(
        body["alwaysApprove"],
        json!(["payment.send", "filing.submit"])
    );

    let (_, reread) = call(&state, "GET", None).await;
    assert!(reread["autoApproveUnderUsd"].is_null());
    assert_eq!(reread["approvalTtlHours"], 72);
}

/// The saved override reaches the live gate on the documented schedule
/// (issue #1455): the deadline immediately — a parked card is re-checked
/// against the current TTL each time it is displayed or swept — and the
/// tier/cap/always-ask half at the next turn boundary, applied by
/// `run_locked` so an in-flight turn finishes under the snapshot it started
/// with.
#[tokio::test]
async fn a_policy_put_applies_the_deadline_immediately_and_the_cap_next_turn() {
    use crate::ports::types::CompanyEvent;

    let dir = home();
    let state = state(dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("registered").clone();
    assert_eq!(runtime.approval_gate.ttl_millis(), 24 * 60 * 60 * 1000);

    // Deadline-only PUT: the live gate's TTL moves without waiting for a turn.
    let (status, body) = call(&state, "PUT", Some(json!({ "approvalTtlHours": 72 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["approvalTtlHours"], 72);
    assert_eq!(runtime.approval_gate.ttl_millis(), 72 * 60 * 60 * 1000);

    // Cap PUT: the snapshot must NOT move mid-turn...
    let (status, _) = call(&state, "PUT", Some(json!({ "autoApproveUnderUsd": 50 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        runtime.approval_gate.policy().auto_approve_under_usd,
        Some(5.0),
        "the evaluation snapshot must not move mid-turn"
    );

    // ...and the next turn applies the effective policy snapshot. Policy
    // HITL is disabled, but the stored cap still moves on its documented
    // schedule for reporting and any future opt-in policy mode.
    runtime
        .run_cycle(vec![CompanyEvent::ScheduleFired {
            cron: "* * * * *".to_string(),
            prompt: "status".to_string(),
        }])
        .await
        .expect("the next turn runs");
    assert_eq!(
        runtime.approval_gate.policy().auto_approve_under_usd,
        Some(50.0)
    );
}

/// The test above proves the cap reaches the live policy snapshot "for
/// reporting", per its own comment. This proves the other half: on the
/// gate this route's `state()` fixture actually builds — through
/// `RuntimeBuilder`, exactly as production does, policy HITL disabled —
/// setting `autoApproveUnderUsd` and applying it does not make a spend
/// over that cap require approval. `PUT {scope}/policy` is admin-gated,
/// validates the value as non-negative and finite, persists it, and
/// carries it to the next turn's snapshot — every one of those steps
/// works — but nothing in the currently-shipped evaluation path ever
/// reads the snapshot's `auto_approve_under_usd` to decide anything,
/// because the disabled-HITL arm of `evaluate` returns before reaching the
/// mode dispatch that would consult it (`policy::gate`). A console
/// showing "capped at $50" is not currently describing an enforced limit.
#[tokio::test]
async fn the_persisted_cap_does_not_gate_a_spend_on_the_production_gate() {
    use crate::ports::ApprovalGate;
    use crate::ports::types::{CompanyEvent, Effect, EffectGroup};

    let dir = home();
    let state = state(dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("registered").clone();
    assert!(
        !runtime.approval_gate.policy_hitl_enabled(),
        "this fixture must build the gate the way production does"
    );

    let (status, _) = call(&state, "PUT", Some(json!({ "autoApproveUnderUsd": 1.0 }))).await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .run_cycle(vec![CompanyEvent::ScheduleFired {
            cron: "* * * * *".to_string(),
            prompt: "status".to_string(),
        }])
        .await
        .expect("the next turn applies the snapshot");
    assert_eq!(
        runtime.approval_gate.policy().auto_approve_under_usd,
        Some(1.0),
        "the cap did reach the live snapshot"
    );

    let over_cap = Effect {
        kind: "payment.send".to_string(),
        group: EffectGroup::Spend,
        amount_usd: Some(1_000_000.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::Value::Null,
        agent: None,
        run_id: None,
    };
    let decision = runtime
        .approval_gate
        .evaluate(&id, &over_cap)
        .await
        .unwrap();
    assert_eq!(
        decision,
        crate::ports::types::PolicyDecision::Allow,
        "a $1,000,000 spend against a $1 cap is allowed on the production gate today — \
         the persisted cap is not currently enforced"
    );
}

/// A deadline `null` releases that one override while preserving the cap,
/// just as `mode: null` releases only the tier override.
#[tokio::test]
async fn null_deadline_stops_overriding_the_deadline() {
    let dir = home();
    let state = state(dir.path()).await;
    call(
        &state,
        "PUT",
        Some(json!({ "autoApproveUnderUsd": 10, "approvalTtlHours": 72 })),
    )
    .await;

    let (status, body) = call(&state, "PUT", Some(json!({ "approvalTtlHours": null }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["autoApproveUnderUsd"], 10.0);
    assert_eq!(body["approvalTtlHours"], 24);
}

/// A body that sets nothing is refused rather than stored, and an unknown
/// tier is refused rather than silently downgraded to `supervised`.
#[tokio::test]
async fn an_empty_body_and_an_unknown_tier_are_both_refused() {
    let dir = home();
    let state = state(dir.path()).await;

    let (status, _) = call(&state, "PUT", Some(json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = call(&state, "PUT", Some(json!({ "mode": "supervized" }))).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown tier must be refused — accepting it would leave the console \
         showing a tier the gate was not running"
    );

    for hours in [0, 8_761, u64::MAX] {
        let (status, _) = call(&state, "PUT", Some(json!({ "approvalTtlHours": hours }))).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "an invalid approval deadline must be refused before persistence"
        );
    }

    // Neither refusal stored anything.
    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["overridden"], false);
}

/// `alwaysApprove` is admin-gated and attributed like every other field
/// here, but nothing bounded its size: an operator (or a script acting as
/// one) could grow the stored list without limit, a standing cost on every
/// `GET` from then on. A refusal here, not silent truncation, matching how
/// every other invalid field on this route is handled.
#[tokio::test]
async fn an_oversized_always_approve_list_is_refused() {
    let dir = home();
    let state = state(dir.path()).await;

    let too_many: Vec<String> = (0..=MAX_ALWAYS_APPROVE_ENTRIES)
        .map(|i| format!("tool.{i}"))
        .collect();
    let (status, _) = call(&state, "PUT", Some(json!({ "alwaysApprove": too_many }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Refused, not truncated and stored.
    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["overridden"], false);

    // Exactly at the cap is accepted.
    let at_cap: Vec<String> = (0..MAX_ALWAYS_APPROVE_ENTRIES)
        .map(|i| format!("tool.{i}"))
        .collect();
    let (status, body) = call(&state, "PUT", Some(json!({ "alwaysApprove": at_cap }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["alwaysApprove"].as_array().unwrap().len(),
        MAX_ALWAYS_APPROVE_ENTRIES
    );
}

/// Same bound, per entry: one absurdly long string is refused rather than
/// stored and re-served on every subsequent read.
#[tokio::test]
async fn an_oversized_always_approve_entry_is_refused() {
    let dir = home();
    let state = state(dir.path()).await;

    let huge = "x".repeat(MAX_ALWAYS_APPROVE_ENTRY_LEN + 1);
    let (status, _) = call(&state, "PUT", Some(json!({ "alwaysApprove": [huge] }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["overridden"], false);
}

/// `DELETE` restores the manifest's policy and is a no-op when nothing is
/// stored.
#[tokio::test]
async fn delete_restores_the_manifest_policy() {
    let dir = home();
    let state = state(dir.path()).await;

    call(&state, "PUT", Some(json!({ "mode": "full" }))).await;
    let (status, body) = call(&state, "DELETE", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "supervised");
    assert_eq!(body["overridden"], false);

    // Deleting again is a no-op, not a 404: the caller's intent is already
    // satisfied.
    let (status, _) = call(&state, "DELETE", None).await;
    assert_eq!(status, StatusCode::OK);
}

/// Every tier the runtime accepts has console text, so none is
/// unselectable (issue #562).
///
/// **This assertion can no longer fail on its own, and that is recorded
/// rather than hidden.** It compares `selectable_tiers()` against
/// `POLICY_MODES`, and since #560 landed `auto` the text table holds exactly
/// those four tiers — so deleting the `POLICY_MODES` filter entirely leaves
/// this green. A revert-and-check confirmed it.
///
/// It is kept because the invariant it states is real and will bite the next
/// time a tier is added to `POLICY_MODES` without text. The filter itself is
/// pinned by `a_tier_the_host_does_not_accept_is_not_offered`, which drives
/// `tiers_for` from synthetic lists and does fail against that revert.
///
/// Asserted in **one** direction on purpose. A `POLICY_MODES` entry with no
/// text here is a tier an operator cannot pick — the same class of gap as a
/// mode the manifest validator rejects, and equally invisible, since every
/// other test would pass. The converse is harmless and deliberate: text for
/// a tier the runtime has not gained yet (`auto`, issue #560, landing in its
/// own PR) is filtered out by `selectable_tiers` and simply does not appear.
/// Asserting that direction too would force the two PRs to be stacked.
#[test]
fn every_runtime_tier_has_console_text() {
    let offered: Vec<&str> = selectable_tiers().iter().map(|t| t.value).collect();
    assert_eq!(
        offered,
        POLICY_MODES.to_vec(),
        "a tier the runtime accepts has no console text, so an operator cannot select it"
    );
    for tier in selectable_tiers() {
        assert!(
            !tier.description.is_empty() && !tier.label.is_empty(),
            "tier `{}` has no operator-facing text, which is the whole point \
             of showing tiers rather than mode names",
            tier.value
        );
    }
}

/// The text table stays a superset of the runtime's tiers, and every entry
/// in it is a real mode name rather than a typo that would silently never
/// render.
#[test]
fn console_text_names_only_plausible_tiers() {
    for tier in TIER_TEXT {
        assert!(
            POLICY_MODES.contains(&tier.value),
            "`{}` is not an accepted mode — a typo here silently drops the \
             tier from the console",
            tier.value
        );
    }
}

/// The host's mode list decides what is offered, not the text table.
///
/// Driven off synthetic lists rather than `POLICY_MODES`, and that is the
/// whole point. `TIER_TEXT` and `POLICY_MODES` now hold the same four tiers,
/// so an assertion built on `POLICY_MODES` cannot distinguish a filtered
/// list from an unfiltered one, and deleting the filter would leave it
/// green. This one fails.
#[test]
fn a_tier_the_host_does_not_accept_is_not_offered() {
    // A host without `auto` (every release before #560) must not be offered
    // it, even though the text for it is compiled in.
    let older_host = tiers_for(&["readonly", "supervised", "full"]);
    assert_eq!(
        older_host.iter().map(|t| t.value).collect::<Vec<_>>(),
        vec!["readonly", "supervised", "full"],
        "text for a tier the host does not accept must not be offered — the \
         gate would silently downgrade it to `supervised`"
    );

    // Order follows the host's list, not the text table's.
    let reordered = tiers_for(&["full", "readonly"]);
    assert_eq!(
        reordered.iter().map(|t| t.value).collect::<Vec<_>>(),
        vec!["full", "readonly"]
    );

    // A mode with no text is skipped rather than panicking or rendering
    // blank; `every_runtime_tier_has_console_text` is what stops that state
    // reaching a release.
    assert!(tiers_for(&["not_a_tier"]).is_empty());
}

/// The privilege boundary this whole route family rests on: a signed-in
/// member cannot move the tier, cap, deadline or always-ask list, and an
/// unauthenticated caller cannot reach the route at all. Every write field
/// on `SetPolicy` is gated by the same `require_admin` call at the top of
/// `set_policy`/`clear_policy`, so one assertion against each method covers
/// every field this route accepts.
#[tokio::test]
async fn a_non_admin_cannot_change_or_clear_the_policy() {
    let dir = home();
    let state = state(dir.path()).await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member = crate::server::test_support::member_cookie("acme");

    for (method, body) in [("PUT", Some(json!({ "mode": "full" }))), ("DELETE", None)] {
        let (status, _) = call_as(&state, method, body.clone(), Some(member.clone())).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} as a member must be refused"
        );

        let (status, _) = call_as(&state, method, body, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} with no session must be refused"
        );
    }

    // Neither denied write moved anything — the admin-only fixture's
    // manifest tier is still what it was seeded with.
    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["mode"], "supervised");
    assert_eq!(body["overridden"], false);
}

/// `autoApproveUnderUsd` is validated non-negative (line 395-399 above),
/// but nothing exercised that check — every existing cap test sends a
/// small positive number. A negative cap must be refused before
/// persistence, the same way an unknown tier is.
///
/// The companion `!cap.is_finite()` half of the same `if` is not
/// exercised here: `1e400` is valid JSON syntax but this crate's
/// `serde_json` errors deserializing it to `f64` ("number out of range")
/// rather than saturating to `INFINITY`, so the request never reaches
/// [`set_policy`] at all — axum's `Json` extractor rejects it upstream
/// with a plain `400`. `NaN` has no JSON token to begin with. There is no
/// value a real HTTP client can put on the wire that reaches the
/// `is_finite()` branch as anything other than `true`.
#[tokio::test]
async fn a_negative_auto_approve_cap_is_refused() {
    let dir = home();
    let state = state(dir.path()).await;

    let (status, _) = call(&state, "PUT", Some(json!({ "autoApproveUnderUsd": -0.01 }))).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a negative cap must be refused"
    );

    // Not stored.
    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["overridden"], false);
}

/// The cap test above (`the_persisted_cap_does_not_gate_a_spend_on_the_
/// production_gate`) proves `autoApproveUnderUsd` is inert on the shipped
/// gate. `alwaysApprove` reaches a *different* arm of `evaluate`
/// (`always_approve::matches`, ahead of the mode dispatch) and is inert for
/// the same root cause but by a separate code path: both are read only
/// after the `policy_hitl_enabled` check, which every production fixture
/// fails. An operator who adds `payment.send` to the always-ask list today
/// sees it persist and echo back on every `GET`, and it still does not park
/// a matching effect.
#[tokio::test]
async fn always_approve_does_not_gate_on_the_production_gate() {
    use crate::ports::ApprovalGate;
    use crate::ports::types::{CompanyEvent, Effect, EffectGroup};

    let dir = home();
    let state = state(dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("registered").clone();
    assert!(
        !runtime.approval_gate.policy_hitl_enabled(),
        "this fixture must build the gate the way production does"
    );

    let (status, _) = call(
        &state,
        "PUT",
        Some(json!({ "alwaysApprove": ["payment.send"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    runtime
        .run_cycle(vec![CompanyEvent::ScheduleFired {
            cron: "* * * * *".to_string(),
            prompt: "status".to_string(),
        }])
        .await
        .expect("the next turn applies the snapshot");
    assert_eq!(
        runtime.approval_gate.policy().always_approve,
        vec!["payment.send".to_string()],
        "the always-ask list did reach the live snapshot"
    );

    let fenced = Effect {
        kind: "payment.send".to_string(),
        group: EffectGroup::Spend,
        amount_usd: Some(1.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::Value::Null,
        agent: None,
        run_id: None,
    };
    let decision = runtime.approval_gate.evaluate(&id, &fenced).await.unwrap();
    assert_eq!(
        decision,
        crate::ports::types::PolicyDecision::Allow,
        "an effect on the always-ask list is allowed on the production gate today — \
         the persisted always-ask list is not currently enforced"
    );

    // Control: the same effect against the same always-ask list DOES park
    // once HITL is enabled — proving the `Allow` above is caused
    // specifically by `policy_hitl_enabled`, not by a broken match on
    // `payment.send` that would read `Allow` for any input.
    // `ManifestApprovalGate::new` defaults to HITL enabled.
    let hitl_enabled_gate =
        crate::policy::gate::ManifestApprovalGate::new(runtime.approval_gate.policy());
    let control_decision = hitl_enabled_gate.evaluate(&id, &fenced).await.unwrap();
    assert_eq!(
        control_decision,
        crate::ports::types::PolicyDecision::RequireApproval,
        "the same always-ask list DOES park this effect once HITL is enabled — the \
         gate this route talks to is not enforcing it purely because production \
         ships with HITL disabled, not because the list or the match is broken"
    );
}

/// `mode: "auto"` is a real `POLICY_MODES` entry — the write route must
/// accept it, not just the three tiers every other test in this file uses.
/// On the live gate `evaluate_auto` is a direct call-through to
/// `evaluate_supervised_with_policy`, so a decision for `auto` and the same
/// decision for `supervised` must agree on every effect: proof that
/// selecting `auto` over `supervised` changes nothing about what gets
/// gated, even though the console renders them as two distinct choices.
///
/// Built off the persisted record rather than the running company's own
/// `approval_gate`: the production fixture's gate never dispatches on mode
/// at all (HITL disabled, see `the_persisted_cap_does_not_gate_a_spend_on_
/// the_production_gate`), and an injected HITL-enabled gate is pinned by
/// the test that built it rather than rebuilt from the record on
/// `run_cycle`. Two fresh gates built straight from the effective policy
/// isolate exactly the one thing this test is about — `evaluate_auto`
/// versus `evaluate_supervised` — from both of those unrelated seams.
#[tokio::test]
async fn mode_auto_is_accepted_and_decides_identically_to_supervised() {
    use crate::ports::ApprovalGate;
    use crate::ports::types::{Effect, EffectGroup};

    let dir = home();
    let state = state(dir.path()).await;

    let (status, body) = call(&state, "PUT", Some(json!({ "mode": "auto" }))).await;
    assert_eq!(status, StatusCode::OK, "`auto` is a valid tier");
    assert_eq!(body["mode"], "auto");

    let store = FsCompanyStore::new(dir.path().to_path_buf());
    let id = CompanyId::new("acme");
    let record = store.load(&id).await.unwrap().expect("company record");
    let auto_policy = record.effective_policy();
    assert_eq!(auto_policy.mode, "auto");

    let spend = Effect {
        kind: "vendor.pay".to_string(),
        group: EffectGroup::Spend,
        amount_usd: Some(1_000.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::Value::Null,
        agent: None,
        run_id: None,
    };

    let under_auto = crate::policy::gate::ManifestApprovalGate::new(auto_policy.clone());
    let under_auto_decision = under_auto.evaluate(&id, &spend).await.unwrap();

    let mut supervised_policy = auto_policy;
    supervised_policy.mode = "supervised".to_string();
    let under_supervised = crate::policy::gate::ManifestApprovalGate::new(supervised_policy);
    let under_supervised_decision = under_supervised.evaluate(&id, &spend).await.unwrap();

    assert_eq!(
        under_auto_decision, under_supervised_decision,
        "`auto` must decide identically to `supervised` — it has no arm of its own"
    );
}

/// `mode` has no `MAX_ALWAYS_APPROVE_ENTRY_LEN`-style length bound, unlike
/// `alwaysApprove` entries — the check is a linear membership test against
/// four short strings, so nothing stops a caller from sending an arbitrarily
/// large string. It must still come back a plain `422`, not a timeout, a
/// panic, or a body large enough to be a standing cost on this route.
#[tokio::test]
async fn an_oversized_mode_string_is_refused_promptly() {
    let dir = home();
    let state = state(dir.path()).await;

    let junk = "x".repeat(1_000_000);
    let started = std::time::Instant::now();
    let (status, _) = call(&state, "PUT", Some(json!({ "mode": junk }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "an oversized `mode` value must be refused promptly, not hang"
    );

    let (_, body) = call(&state, "GET", None).await;
    assert_eq!(body["overridden"], false);
}
