/// The wire shape the console binds to.
///
/// `fold_asides` is worthless if the field reaches the browser under a
/// different name, and `tsc` cannot catch that: the DTO is Rust, the
/// interface is hand-written TypeScript, and nothing checks one against the
/// other. This is that check.

use super::operator_test_support_1::*;
use super::operator_test_support_2::*;
use super::operator_test_support_3::*;
use super::operator_test_support_4::*;

/// **T6 (issue #971).** A host-side expiry says so, without saying who.
///
/// The defect: an expiry appends `ApprovalResolved { Deny, System }`, this
/// frame dropped the actor, and the console toasted "Approval denied" — so
/// an operator was told they had declined a request they never saw. With a
/// 24-hour deadline that stops being rare.
///
/// The assertion above is **extended here, not replaced**: the new field is
/// a bit derived from `by.kind`, and the no-actor / no-user-id property it
/// is derived from has to keep holding, so it is re-asserted on this arm
/// with a `System` actor whose id is equally secret.
#[test]
fn projects_a_host_side_expiry_as_automatic_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalResolved {
        approval_id: ApprovalId::new("ap-2"),
        verdict: Verdict::Deny,
        by: Actor {
            kind: ActorKind::System,
            // Even the system actor's id stays off the feed: the console
            // needs the *fact* that no person decided this, not the name of
            // the internal path that did.
            id: "expiry".into(),
        },
    }))
    .expect("approval_resolved is an attention signal");
    assert_eq!(v["type"], "approval_resolved");
    assert_eq!(v["approvalId"], "ap-2");
    assert_eq!(v["verdict"], "deny");
    assert_eq!(
        v["automatic"], true,
        "the console must be able to say the deadline passed rather than \
         attributing the deny to whoever is looking at it"
    );
    // The extended property, restated on this arm.
    assert!(v.get("by").is_none(), "actor must not be projected");
    assert!(
        !v.to_string().contains("expiry"),
        "the actor id must not reach the wire on this arm either"
    );
}

#[test]
fn projects_task_steered_without_actor_or_instruction() {
    let v = super::project_event(&stored(CompanyEvent::TaskSteered {
        task_id: "t-9".into(),
        action: "redirect".into(),
        instruction: Some("focus on the API".into()),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        }),
    }))
    .expect("task_steered is an attention signal");
    assert_eq!(v["type"], "task_steered");
    assert_eq!(v["taskId"], "t-9");
    assert_eq!(v["action"], "redirect");
    let wire = v.to_string();
    assert!(!wire.contains("secret-user-id"));
    assert!(!wire.contains("focus on the API"));
}

#[test]
fn projects_workflow_created_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowCreated {
        workflow_id: "greeter".into(),
        name: "Greeter".into(),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        }),
    }))
    .expect("workflow_created is an attention signal");
    assert_eq!(v["type"], "workflow_created");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter");
    assert!(!v.to_string().contains("secret-user-id"));
}

/// Issue #259: the edit and delete signals project the same two fields and
/// drop the actor, exactly like `workflow_created` above.
#[test]
fn projects_workflow_updated_and_deleted_without_the_actor() {
    let actor = || {
        Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        })
    };

    let v = super::project_event(&stored(CompanyEvent::WorkflowUpdated {
        workflow_id: "greeter".into(),
        name: "Greeter v2".into(),
        by: actor(),
    }))
    .expect("workflow_updated is an attention signal");
    assert_eq!(v["type"], "workflow_updated");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter v2");
    assert!(!v.to_string().contains("secret-user-id"));

    let v = super::project_event(&stored(CompanyEvent::WorkflowDeleted {
        workflow_id: "greeter".into(),
        name: "Greeter".into(),
        by: actor(),
    }))
    .expect("workflow_deleted is an attention signal");
    assert_eq!(v["type"], "workflow_deleted");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter");
    assert!(!v.to_string().contains("secret-user-id"));
}

#[test]
fn projects_lifecycle_changed_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::LifecycleChanged {
        from: "running".into(),
        to: "paused".into(),
        by: Actor {
            kind: ActorKind::Operator,
            id: "operator".into(),
        },
    }))
    .expect("lifecycle_changed is an attention signal");
    assert_eq!(v["type"], "lifecycle_changed");
    assert_eq!(v["from"], "running");
    assert_eq!(v["to"], "paused");
    assert!(v.get("by").is_none(), "actor must not be projected");
}

#[test]
fn projects_payment_received() {
    let v = super::project_event(&stored(CompanyEvent::PaymentReceived {
        amount_usd: 25.0,
        memo: "invoice #1".into(),
    }))
    .expect("payment_received is an attention signal");
    assert_eq!(v["type"], "payment_received");
    assert_eq!(v["amountUsd"], 25.0);
    assert_eq!(v["memo"], "invoice #1");
}

/// The live half of #228: a finished run reaches the console as it happens,
/// carrying exactly the fields the run drawer already renders — so the
/// console can toast an undelivered report instead of waiting for a reload.
#[test]
fn projects_workflow_run_finished_with_the_fields_the_drawer_renders() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: vec![
            delivery_row("owner_summary", crate::ports::DeliveryStatus::Skipped),
            delivery_row("also_sent", crate::ports::DeliveryStatus::Sent),
        ],
        pending_approvals: vec!["review".into()],
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["type"], "workflow_run_finished");
    assert_eq!(v["seq"], 7);
    assert_eq!(v["workflowId"], "digest");
    assert_eq!(v["scheduled"], true);
    assert_eq!(v["pendingApprovals"][0], "review");

    // Per-row node/kind/target/status/detail — the same shape the manual
    // run's HTTP response already ships to this console.
    let rows = v["deliveries"].as_array().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["node"], "owner_summary");
    assert_eq!(rows[0]["kind"], "email");
    assert_eq!(rows[0]["status"], "skipped");
    assert_eq!(rows[0]["target"], "ada@example.com");
    assert!(
        rows[0]["detail"]
            .as_str()
            .unwrap()
            .contains("never written"),
        "the detail names the fix: {v}"
    );

    // A run that finished carries no `error` key, and `runId` — always
    // `None` today — is never a permanently-null key on the wire.
    assert!(v.get("error").is_none(), "{v}");
    assert!(v.get("runId").is_none(), "{v}");
}

/// The failure arm reaches the console too — it is the outcome that used to
/// produce nothing but a host-stdout warning.
#[test]
fn projects_workflow_run_finished_with_the_failure_reason() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: Some("no inference source for agent node `worker`".into()),
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["error"], "no inference source for agent node `worker`");
    assert_eq!(v["deliveries"].as_array().unwrap().len(), 0);
}

/// Issues #881 / #880: the blocked arm reaches the console live.
///
/// Without it a console watching a run settle would be told it finished
/// cleanly — no error, not cancelled, nothing delivered — and then the
/// history it reloads a moment later would say the run blocked. The two
/// surfaces read the same journal event, so they must project the same
/// facts.
#[test]
fn projects_workflow_run_finished_with_its_blocked_nodes_and_parked_approvals() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: Some("run-b".into()),
        deliveries: Vec::new(),
        pending_approvals: vec!["spec".into()],
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: vec![crate::ports::WorkflowBlockedNode {
            node_id: "spec".into(),
            tools: vec!["publish_artifact".into()],
            approval_ids: vec!["appr-1".into()],
            unparkable: 0,
            stranded: 0,
            blockers: 0,
        }],
        approvals: vec![crate::ports::WorkflowRunApprovalRow {
            node_id: Some("spec".into()),
            tool: Some("publish_artifact".into()),
            outcome: crate::ports::WorkflowApprovalOutcome::Parked,
            approval_id: Some("appr-1".into()),
        }],
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["blockedNodes"][0]["nodeId"], "spec");
    assert_eq!(v["blockedNodes"][0]["tools"][0], "publish_artifact");
    assert_eq!(v["approvals"][0]["outcome"], "parked");
    assert!(
        v.get("error").is_none(),
        "a run waiting on a person did not fail: {v}"
    );

    // The presence-check discipline: a run that blocked on nobody sends
    // neither key, so an existing frame is byte-unchanged.
    let clean = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projects");
    assert!(clean.get("blockedNodes").is_none(), "{clean}");
    assert!(clean.get("approvals").is_none(), "{clean}");
}

/// Issue #371: the live per-node trail. Both arms project, both carry the
/// run id that ties them to the run's settle-frame, and — the point — the
/// node arm carries a status and a duration and nothing else.
#[test]
fn projects_the_per_node_progress_trail() {
    let started = super::project_event(&stored(CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        scheduled: true,
        started_by: None,
        resume_semantic: None,
    }))
    .expect("workflow_run_started reaches the console");
    assert_eq!(started["type"], "workflow_run_started");
    assert_eq!(started["workflowId"], "digest");
    assert_eq!(started["runId"], "run-1");
    assert_eq!(started["scheduled"], true);
    assert!(
        started.get("startedBy").is_none(),
        "no sender projects no key: {started}"
    );

    // Issue #1862 prerequisite: when the journal carries a sender, the SSE
    // frame forwards it under `startedBy`.
    let started_with_sender = super::project_event(&stored(CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        scheduled: false,
        started_by: Some(crate::ports::types::StartedBy::Agent("ceo".into())),
        resume_semantic: None,
    }))
    .expect("workflow_run_started reaches the console");
    assert_eq!(
        started_with_sender["startedBy"],
        serde_json::json!({"agent": "ceo"})
    );

    let node = super::project_event(&stored(CompanyEvent::WorkflowNodeFinished {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        node_id: "ceo".into(),
        status: crate::ports::types::WorkflowNodeStatus::Error,
        elapsed_ms: 1234,
        diagnostics: Vec::new(),
        agent_run_id: None,
    }))
    .expect("workflow_node_finished reaches the console");
    assert_eq!(node["type"], "workflow_node_finished");
    assert_eq!(node["runId"], "run-1");
    assert_eq!(node["nodeId"], "ceo");
    assert_eq!(node["status"], "error");
    assert_eq!(node["elapsedMs"], 1234);

    // The scrubbing claim, stated as a test: an errored node projects a
    // status word and NOTHING that could carry the node's own words. The
    // event type has no field to hold them, so this can only regress by
    // widening the event — which is the point of keeping it closed.
    let mut keys: Vec<&str> = node
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "atMillis",
            "elapsedMs",
            "nodeId",
            "runId",
            "seq",
            "status",
            "type",
            "workflowId",
        ],
        "the node frame carries only structural fields: {node}"
    );
}

/// Issue #382: the per-node START bracket reaches the console too. Without
/// its own arm it would fall to `project_event`'s `_ => return None` wildcard
/// and be silently dropped — the exact trap this file has been bitten by
/// three times — and the canvas would be back to guessing which node runs.
/// It carries the ids and NOTHING else: no status or duration (the node has
/// not run) and no input, so the frame is structural by construction.
#[test]
fn projects_the_per_node_started_bracket() {
    let node = super::project_event(&stored(CompanyEvent::WorkflowNodeStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        node_id: "ceo".into(),
    }))
    .expect("workflow_node_started reaches the console");
    assert_eq!(node["type"], "workflow_node_started");
    assert_eq!(node["workflowId"], "digest");
    assert_eq!(node["runId"], "run-1");
    assert_eq!(node["nodeId"], "ceo");

    // Structural-only: ids plus the envelope, and no status/duration/payload
    // slot the finish frame has. Regresses only by widening the event.
    let mut keys: Vec<&str> = node
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["atMillis", "nodeId", "runId", "seq", "type", "workflowId"],
        "the started frame carries only structural ids: {node}"
    );
}

/// Issue #371 also starts projecting the run id on the settle-frame — the
/// key that lets the console clear the right canvas when two runs overlap.
/// Still omitted for a pre-#371 row, so no permanently-null key appears.
#[test]
fn projects_the_run_id_on_a_finished_run_only_when_there_is_one() {
    let with_id = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: false,
        run_id: Some("run-9".into()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projected");
    assert_eq!(with_id["runId"], "run-9");

    let legacy = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: false,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projected");
    assert!(legacy.get("runId").is_none(), "{legacy}");
}

#[test]
fn drops_non_attention_and_raw_payload_events() {
    // The operator's own message, and every variant that carries a raw
    // third-party payload or is audit-only, is dropped so nothing unexpected
    // (or secret-bearing) ever reaches the console.
    //
    // This list is unchanged by #228: adding `workflow_run_finished` to the
    // projection widened the wire by exactly one listed variant, and this
    // test passing untouched is what proves the deny-by-default default
    // still drops everything it dropped before.
    let dropped = [
        CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            parent: None,
            text: "hi".into(),
            by: None,
            chat: None,
            deliverable: None,
            attachments: Vec::new(),
        },
        CompanyEvent::WebhookReceived {
            channel: "email".into(),
            body: serde_json::json!({"authorization": "Bearer sk-secret"}),
        },
        CompanyEvent::A2aTaskReceived {
            from: "@peer".into(),
            task: serde_json::json!({"token": "sk-secret"}),
        },
        CompanyEvent::ScheduleFired {
            cron: "0 9 * * *".into(),
            prompt: "daily standup".into(),
        },
        CompanyEvent::FeedbackFiled {
            note: "too slow".into(),
        },
        CompanyEvent::MemoryFactDeleted {
            fact_id: "f-1".into(),
        },
    ];
    for event in dropped {
        assert!(
            super::project_event(&stored(event.clone())).is_none(),
            "event should be dropped from the SSE feed: {event:?}"
        );
    }
}

#[tokio::test]
async fn events_route_streams_text_event_stream() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/events")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The SSE head is returned immediately; the body streams indefinitely, so
    // we assert the status + content-type without draining it.
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
}

#[tokio::test]
async fn events_route_requires_a_session() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The composer's own typing pings must not echo back to it — the bus has
/// no per-listener addressing, so this filter is the only thing standing
/// between "you typed" and a fresh "Alice is typing…" line under your own
/// cursor.
#[test]
fn a_typing_frame_from_the_viewer_is_dropped_and_from_anybody_else_is_kept() {
    let mine = crate::turn_stream::LiveFrame::Typing(crate::turn_stream::TypingFrame {
        kind: "typing",
        user_id: "u1".into(),
        chat_id: "engineering".into(),
        parent_id: None,
        at_millis: 0,
    });
    assert!(super::is_own_typing_frame(&mine, Some("u1")));
    assert!(!super::is_own_typing_frame(&mine, Some("u2")));
    assert!(
        !super::is_own_typing_frame(&mine, None),
        "a machine credential with nobody behind it authors nothing to echo"
    );

    let presence = crate::turn_stream::LiveFrame::Presence(crate::turn_stream::PresenceFrame {
        kind: "presence",
        user_id: "u1".into(),
        status: "online",
        at_millis: 0,
    });
    assert!(
        !super::is_own_typing_frame(&presence, Some("u1")),
        "presence is left alone — only typing echoes"
    );
}

/// Every contradictory or unbounded scope request is a 400, and none of them
/// reaches the runtime.
///
/// The approval id is deliberately one that does not exist: each of these
/// must be refused at the edge, so the fact that resolving a missing
/// approval would otherwise be a harmless no-op never gets a chance to mask
/// a body that should not have been accepted.
///
/// A deny may now ride the tool scope (issue #1458 — a standing refusal),
/// so that pairing is asserted as *accepted* at the bottom rather than
/// listed among the refusals.
#[tokio::test]
async fn a_contradictory_or_unbounded_scope_is_refused() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let day: u64 = 24 * 60 * 60 * 1000;
    for (label, body) in [
        (
            "an argument edit and a standing grant contradict",
            format!(
                r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{day},"amended_payload":{{"to":"x"}}}}"#
            ),
        ),
        (
            "the deadline is mandatory",
            r#"{"verdict":"approve","scope":"tool"}"#.to_string(),
        ),
        (
            "zero is not a duration",
            r#"{"verdict":"approve","scope":"tool","expires_in_millis":0}"#.to_string(),
        ),
        (
            "past the seven-day cap is refused, never clamped",
            format!(
                r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{}}}"#,
                MAX_STANDING_GRANT_MILLIS + 1
            ),
        ),
        (
            "a duration is meaningless on the once scope",
            format!(r#"{{"verdict":"approve","scope":"once","expires_in_millis":{day}}}"#),
        ),
    ] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/approvals/appr-missing")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label}: must be refused at the edge"
        );
    }

    // An unrecognised scope is refused too, one layer earlier: `ResolveScope`
    // is a closed enum, so axum's JSON extractor rejects it as 422 before
    // any handler runs. The status differs from the checks above; what
    // matters is that it is never silently downgraded to `once`, which would
    // hand an operator a single call when they asked for a standing one.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"verdict":"approve","scope":"forever"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Exactly at the cap is fine — the boundary is inclusive.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{MAX_STANDING_GRANT_MILLIS}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::BAD_REQUEST);

    // A deny riding the tool scope is no longer a contradiction: it mints a
    // standing refusal (issue #1458). Same edge validation as an approve —
    // duration mandatory, bounded, and the missing approval resolves as a
    // no-op — so it is accepted exactly where a matching approve would be.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"verdict":"deny","scope":"tool","expires_in_millis":{day}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::BAD_REQUEST);
}

/// The default body — no `scope` key at all — is accepted exactly as before.
#[tokio::test]
async fn an_omitted_scope_is_the_pre_374_request() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"verdict":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
