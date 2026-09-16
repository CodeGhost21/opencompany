/// The wire shape the console binds to.
///
/// `fold_asides` is worthless if the field reaches the browser under a
/// different name, and `tsc` cannot catch that: the DTO is Rust, the
/// interface is hand-written TypeScript, and nothing checks one against the
/// other. This is that check.

use super::operator_test_support_1::*;
use super::operator_test_support_3::*;
use super::operator_test_support_4::*;

pub(super) async fn put_desk_order(
    app: &axum::Router,
    cookie: &str,
    desk: &str,
    body: &str,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/company/desks/{desk}/order"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

pub(super) async fn get_operator_channel(app: &axum::Router, cookie: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/operator-channel")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Everything one agent said and heard, for a company with one reply in it.
///
/// Fetched through the router so the assertion is about the wire, not about
/// the struct it was built from.
pub(super) async fn session_rows(uri: &str) -> Vec<serde_json::Value> {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "General".to_string(),
                agent_id: "ceo".to_string(),
                text: "one turn, from one session".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = value.as_array().cloned().unwrap_or_default();
    assert!(!rows.is_empty(), "no session rows came back from {uri}");
    rows
}

/// Posts a chat message and returns the decoded `ChatResponse` body.
pub(super) async fn post_chat(app: &Router, cookie: &str, body: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "chat POST failed");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Reads a desk's history. `desk` empty reads the default General thread.
pub(super) async fn get_history(app: &Router, cookie: &str, desk: &str) -> Vec<serde_json::Value> {
    let uri = if desk.is_empty() {
        "/api/v1/company/chat/history".to_string()
    } else {
        format!("/api/v1/company/chat/history?desk={desk}")
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value.as_array().cloned().unwrap_or_default()
}

/// Sets or clears one reaction, returning the status.
pub(super) async fn post_reaction(
    app: &Router,
    cookie: &str,
    seq: &str,
    emoji: &str,
    on: bool,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/chat/messages/{seq}/reactions"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "emoji": emoji, "on": on }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// **Issue #2028 (finding 2, deadlock regression).** Answering a
/// task-backed blocker in a DM runs the whole path end to end: the route
/// reads and classifies the reply, settles the verdict, and waits on the
/// follow-up that re-dispatches the card — and that follow-up runs on a
/// spawned task which takes `task_writes` for its board edit.
///
/// So the route must not still hold `task_writes` when it waits. It did,
/// having mirrored the guard from the review branch above it, and the two
/// together are a deadlock: the handler waits for a task that is waiting for
/// the handler's lock. Explicitly bounded rather than left to hang, so a
/// regression fails in seconds instead of taking a runner down for an hour.
#[cfg(feature = "openhuman")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub(super) async fn a_dm_answer_to_a_task_backed_blocker_completes() {
    use crate::company::blocker_sender::BlockerSenderSignals;
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = build_state_with_brain_and_manifest(
        &home,
        "running",
        AppConfig::default(),
        None,
        roster_manifest(),
    )
    .await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    let mut card = crate::ports::tasks::TaskRecord {
        id: "t-9".to_string(),
        title: crate::ports::tasks::TaskTitle::authored("Draft the launch note"),
        note: None,
        column: crate::ports::tasks::COLUMN_PAUSED.to_string(),
        priority: "medium".to_string(),
        assignee: "backend_engineer".to_string(),
        updated_at_millis: 1,
        origin: None,
        origin_message_seq: None,
        parent_task_id: None,
        output: None,
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: crate::ports::tasks::TaskDeliverable::Once,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
        bounced: None,
    };
    card.origin =
        crate::ports::tasks::TaskOrigin::new(Some("dm:backend_engineer".to_string()), None);
    runtime.tasks().upsert(runtime.id(), &card).await.unwrap();

    runtime
        .park_blocker(
            &BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Task {
                    task_id: "t-9".to_string(),
                }),
                reason: "the model id was rejected".to_string(),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            },
            "t-9",
            BlockerSenderSignals {
                started_by: None,
                owner_desk: None,
                assignee: Some("backend_engineer".to_string()),
            },
        )
        .await
        .expect("parks the blocker into the teammate's DM");

    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/acme/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"chat":"dm:backend_engineer","text":"yes, go ahead and retry it"}"#,
                ))
                .unwrap(),
        ),
    )
    .await
    .expect(
        "answering a task-backed blocker in a DM deadlocked: the route held the board \
         lock while waiting on the follow-up that needs it",
    )
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        runtime.pending_approvals().is_empty(),
        "the answered blocker is retired"
    );
    let moved = runtime
        .tasks()
        .list(runtime.id())
        .await
        .expect("list")
        .into_iter()
        .find(|t| t.id == "t-9")
        .expect("the card is still on the board");
    assert_eq!(
        moved.column,
        crate::ports::tasks::COLUMN_IN_PROGRESS,
        "the DM answer re-dispatched the paused card"
    );
}

/// The ask-which question lands in the thread that asked it.
///
/// When two blocked things share a DM and the reply names neither, the
/// runtime asks which one was meant. That question is an answer to the
/// operator's message, so it threads off it the way every other reply in
/// this handler does — otherwise the operator reads their own line in a
/// thread and the teammate's follow-up at the channel root, which is the
/// split this tier exists to close.
#[cfg(feature = "openhuman")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub(super) async fn the_ask_which_question_threads_off_the_reply_that_was_ambiguous() {
    use crate::company::blocker_sender::BlockerSenderSignals;
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = build_state_with_brain_and_manifest(
        &home,
        "running",
        AppConfig::default(),
        None,
        roster_manifest(),
    )
    .await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    for (task, connection) in [("t-1", "connection:slack"), ("t-2", "connection:notion")] {
        runtime
            .park_blocker(
                &BlockerPayload {
                    kind: BlockerKind::Infrastructure,
                    source: BlockerSource::Provider,
                    step: Some(BlockerStep::Task {
                        task_id: task.to_string(),
                    }),
                    reason: format!("{connection} refused the call"),
                    needed: "a working connection".to_string(),
                    group_key: Some(connection.to_string()),
                },
                task,
                BlockerSenderSignals {
                    started_by: None,
                    owner_desk: None,
                    assignee: Some("backend_engineer".to_string()),
                },
            )
            .await
            .expect("parks the blocker into the teammate's DM");
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/acme/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"chat":"dm:backend_engineer","text":"retry it"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = runtime
        .events
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("read events");
    let asked = stored
        .iter()
        .find_map(|s| match &s.event {
            crate::ports::types::CompanyEvent::OperatorMessage { chat, text, .. }
                if chat.as_deref() == Some("dm:backend_engineer") && text == "retry it" =>
            {
                Some(s.seq)
            }
            _ => None,
        })
        .expect("the operator's ambiguous reply is journalled");
    let prompt = stored
        .iter()
        .find_map(|s| match &s.event {
            crate::ports::types::CompanyEvent::AgentReply {
                chat_id,
                text,
                parent,
                ..
            } if chat_id == "dm:backend_engineer" && text.contains("Which") => {
                Some((text.clone(), *parent))
            }
            _ => None,
        })
        .expect("the runtime asks which of the two was meant");
    assert_eq!(
        prompt.1,
        Some(asked),
        "the ask-which question must hang off the reply that was ambiguous, not the \
         channel root; prompt was {:?}",
        prompt.0
    );
}

/// The tool call the operator is asked to sign off. `agent: Some(_)` is what
/// makes approving it mint a single-use grant rather than execute it
/// (issue #243) — which is the whole reason a lost continuation hurts: the
/// grant is spent on a turn that never happens.
///
/// The payload names an action the vendored catalogue tags `Write`, so
/// `consequence_of` classifies it as a send on its merits. Until issue #470
/// it named the slug under `tool_slug`, a key neither the tool nor the
/// classifier reads — so it was a call with no action at all, and it
/// reached the per-call verdict through the unknown-slug fallback instead.
pub(super) fn gated_tool_call() -> crate::ports::types::Effect {
    crate::ports::types::Effect {
        kind: "composio_execute".into(),
        group: crate::ports::types::EffectGroup::Sign,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: crate::policy::test_support::composio_send_args(),
        agent: Some("ceo".into()),
        run_id: None,
    }
}

/// A tool call an operator MAY grant a standing permission for (issue
/// #431), which `gated_tool_call` deliberately is not: its Composio payload
/// names an action the catalogue tags `Write`, so `consequence_of` reads it
/// as a send and it stays a per-call decision. `file_write` is declared
/// grantable in `src/policy/consequence.rs` and carries an agent, so it
/// satisfies both halves of `check_broadly_grantable` — it mutates, but
/// only the agent's own sandboxed workspace.
pub(super) fn grantable_tool_call() -> crate::ports::types::Effect {
    crate::ports::types::Effect {
        kind: "file_write".into(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "path": "notes/a.md", "body": "one" }),
        agent: Some("ceo".into()),
        run_id: None,
    }
}

/// The dotted kind the stalled brain parks once its follow-up turn gets
/// past the barrier. Parking journals durably (`record_parked`), so its
/// presence in `pending_approvals()` is proof the continuation reached the
/// end of the turn *and* wrote to disk — not merely that a task was alive.
pub(super) const CONTINUATION_MARKER: &str = "continuation.marker";

#[async_trait::async_trait]
impl crate::ports::brain::Brain for StalledContinuationBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        for event in &req.events {
            match event {
                CompanyEvent::OperatorMessage { .. } => {
                    host.park_effect(self.parked.clone()).await?;
                }
                CompanyEvent::ApprovalResolved { .. } => {
                    self.entered.notify_one();
                    self.release.notified().await;
                    host.park_effect(crate::ports::types::Effect {
                        kind: CONTINUATION_MARKER.into(),
                        group: crate::ports::types::EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: serde_json::json!({}),
                        agent: None,
                        run_id: None,
                    })
                    .await?;
                }
                _ => {}
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses: Vec::new(),
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "stalled continuation",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

pub(super) fn chat_request(text: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/company/chat")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({ "text": text }).to_string()))
        .unwrap()
}

/// A resolve against the single-company alias. `scope` lets the same body be
/// aimed at the `/companies/{id}` form, which must behave identically.
pub(super) fn resolve_request_scoped(
    scope: &str,
    approval_id: &ApprovalId,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}"))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub(super) fn resolve_request(approval_id: &ApprovalId, body: serde_json::Value) -> Request<Body> {
    resolve_request_scoped("/api/v1/company", approval_id, body)
}

/// Parks a workflow-node blocker: `TaskLink::Unlinked` with no
/// conversation, which is the shape a node blocker takes and the reason the
/// chat blocker path — which filters on the thread — can never reach one.
#[cfg(feature = "openhuman")]
pub(super) async fn park_node_blocker(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
    group_key: Option<&str>,
) -> ApprovalId {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let payload = BlockerPayload {
        kind: BlockerKind::Infrastructure,
        source: BlockerSource::Provider,
        step: Some(BlockerStep::Node {
            run_id: "run-1".to_string(),
            node_id: "draft".to_string(),
        }),
        reason: "the model id `gpt-nope` was rejected".to_string(),
        needed: "a model id this provider serves".to_string(),
        group_key: group_key.map(str::to_string),
    };
    let approval = ApprovalId::new(id);
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: Some("run-1".to_string()),
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();
    approval
}

/// Every `BlockerResolved` line the durable journal holds, in append order —
/// what the operator's answer actually banked, read off disk rather than off
/// the in-memory map the resume consumes.
#[cfg(feature = "openhuman")]
pub(super) async fn banked_resolutions(
    home: &std::path::Path,
    company: &CompanyId,
) -> Vec<serde_json::Value> {
    let path = crate::store::paths::Bundle::new(home, company).journal_jsonl();
    let raw = tokio::fs::read_to_string(path).await.unwrap_or_default();
    raw.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|line| line["record"] == "BlockerResolved")
        .collect()
}

/// A company with one parked workflow-node blocker, and the pieces a resolve
/// test needs to read back what its click banked.
#[cfg(feature = "openhuman")]
struct BlockedCompany {
    app: axum::Router,
    runtime: Arc<CompanyRuntime>,
    home: std::path::PathBuf,
    company: CompanyId,
    approval_id: ApprovalId,
}

#[cfg(feature = "openhuman")]
pub(super) async fn blocked_company(home: &std::path::Path) -> BlockedCompany {
    let home = home.to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let approval_id = park_node_blocker(&runtime, "blocker-1", None).await;
    BlockedCompany {
        app,
        runtime,
        home,
        company,
        approval_id,
    }
}

/// Posts a resolve and returns its status and parsed body.
#[cfg(feature = "openhuman")]
pub(super) async fn post_resolve(
    app: &axum::Router,
    id: &ApprovalId,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(resolve_request(id, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Every refusal owes the same two things beyond its 400: the blocker is
/// still parked, and nothing was banked. A validation that answered 400
/// after journaling a verdict would have spent the operator's question.
#[cfg(feature = "openhuman")]
pub(super) async fn assert_refused(body: serde_json::Value, expect_in_error: &str) {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(&c.app, &c.approval_id, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let message = answer["error"].as_str().unwrap_or_default();
    assert!(
        message.contains(expect_in_error),
        "the refusal must say why; got {message:?}"
    );
    assert!(
        c.runtime
            .pending_approvals()
            .iter()
            .any(|p| p.id == c.approval_id),
        "a refused request must leave the blocker parked"
    );
    assert!(
        banked_resolutions(&c.home, &c.company).await.is_empty(),
        "a refused request must journal no verdict"
    );
}

/// Parks one effect in BOTH the gate and the journal under a fixed id, at a
/// controllable instant — the gate is what `extend_approval` asks whether an
/// id is live, and the journal is what projects the deadline, so an extend
/// test needs both seeded exactly as a real park leaves them.
pub(super) async fn park_for_extend(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
    at_millis: u64,
) -> ApprovalId {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};
    let approval = ApprovalId::new(id);
    let effect = crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(1_200.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "vendor@example.test" }),
        agent: Some("ceo".into()),
        run_id: None,
    };
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at_millis);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at_millis,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();
    approval
}

pub(super) fn extend_request_with_cookie(approval_id: &ApprovalId, cookie: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/company/approvals/{approval_id}/extend"))
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

pub(super) fn extend_request(approval_id: &ApprovalId) -> Request<Body> {
    extend_request_with_cookie(
        approval_id,
        crate::server::test_support::fixed_cookie("acme"),
    )
}
