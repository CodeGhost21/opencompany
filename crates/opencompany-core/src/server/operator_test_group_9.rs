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

/// The link is followed only to a card the board still holds.
///
/// A stepless question's approval carries a task link because a card was in
/// hand when it was asked, not because the card is the thing to re-enter.
/// When that card is gone — deleted, or never on this board — reading the
/// link as a card resume answers the operator with *that card is no longer
/// on the board*, which is a report about a card in place of the answer to
/// the question they just gave. The answer goes back into the conversation
/// instead, exactly as it does for a question that was never linked.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_agent_question_linked_to_a_card_the_board_lost_still_answers_the_question() {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    let payload = BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::AgentQuestion,
        step: None,
        reason: "which of the two briefs is current?".to_string(),
        needed: "an answer from you".to_string(),
        group_key: None,
    };
    let approval = ApprovalId::new("question-2");
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: None,
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    // The link names a card that is not on the board, which is the whole
    // case: nothing is seeded for `t-gone`.
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::from_task_id(Some("t-gone")),
            ApprovalConversation {
                thread: Some("dm:eng".to_string()),
                parent: None,
            },
            None,
        )
        .await
        .unwrap();

    let (status, answer) = post_resolve(
        &app,
        &approval,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "retry" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(
        banked[0]["resolution"]["verdict"], "retry",
        "the operator's answer is banked whatever the resume finds: {}",
        banked[0]
    );
    let notes: Vec<String> = runtime
        .events
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("read events")
        .into_iter()
        .filter_map(|stored| match stored.event {
            crate::ports::types::CompanyEvent::AgentReply { chat_id, text, .. }
                if chat_id == "dm:eng" =>
            {
                Some(text)
            }
            _ => None,
        })
        .collect();
    assert!(
        !notes
            .iter()
            .any(|note| note.contains("no longer on the board")),
        "answering a question must not report on a card the asker never mentioned; \
         posted: {notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note == "Got it — picking that back up now."),
        "the answer must still reach the conversation it was asked in; posted: {notes:?}"
    );
}

/// A build with no blocker resume refuses the field outright. Accepting and
/// ignoring it would answer `200` to a skip that silently became a retry —
/// the exact defect, reintroduced by a feature flag.
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn a_build_without_the_resume_refuses_a_blocker_verdict() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(resolve_request(
            &ApprovalId::new("missing"),
            serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not supported by this build"),
        "{value}"
    );
}

/// The keystone (issue #1805): extending a parked approval pushes its
/// deadline out to a fresh full window, and the receipt names the new one —
/// the console can redraw the countdown without re-fetching the list.
#[tokio::test]
async fn extending_a_parked_approval_moves_its_deadline() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    // Parked long ago, so its original deadline is `1_000 + ttl`.
    let id = park_for_extend(&runtime, "appr-ext", 1_000).await;
    let before = runtime.pending_approvals()[0]
        .expires_at_millis
        .expect("a deadline is projected");

    let app = router(state);
    let response = app.oneshot(extend_request(&id)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    let after = runtime.pending_approvals()[0]
        .expires_at_millis
        .expect("a deadline is still projected");
    assert!(
        after > before,
        "the deadline moved out: before={before} after={after}"
    );
    assert!(body["extended"].as_bool().unwrap());
    assert_eq!(
        body["expiresAtMillis"].as_f64().unwrap() as u64,
        after,
        "the receipt's deadline is the one the card now projects"
    );
}

/// Extending something that is not parked — an unknown id, or one already
/// resolved or expired — is a 404, not a 200 over nothing.
#[tokio::test]
async fn extending_an_unknown_approval_is_404() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);
    let response = app
        .oneshot(extend_request(&ApprovalId::new("does-not-exist")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// POL-011 (INPUT). `{aid}` is an opaque path segment carried straight
/// into `ApprovalId::new` with no format check of its own — the id space
/// is "whatever a park was given", so the whole of input-safety here is
/// that an adversarial or malformed segment resolves to the same ordinary
/// 404 an unknown id does, never a panic or a 500.
#[tokio::test]
async fn extending_a_malformed_approval_id_is_404_not_a_crash() {
    fn percent_encode_path_segment(raw: &str) -> String {
        let mut out = String::new();
        for byte in raw.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char);
                }
                _ => out.push_str(&format!("%{byte:02X}")),
            }
        }
        out
    }

    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);

    let hostile_ids = [
        "../../../etc/passwd".to_string(),
        "🎉💥-not-an-approval".to_string(),
        "a".repeat(10_000),
        "'; DROP TABLE approvals; --".to_string(),
        "appr\u{0}-null-byte".to_string(),
    ];
    for raw in hostile_ids {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/company/approvals/{}/extend",
                        percent_encode_path_segment(&raw)
                    ))
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a malformed id ({raw:?}) must answer the same 404 an unknown id does, not crash"
        );
    }
}

/// APPR-004: extend must be able to win a race the sweep has not yet run —
/// an approval whose deadline has already passed but that is still
/// physically parked (nothing has swept it out of the gate) must still be
/// extendable, and the extension must genuinely move the deadline rather
/// than just answer as if it had.
///
/// `resolve`'s own past-deadline check (`gate.rs`'s TTL math) and
/// `extend`'s (`ParkedApprovals::extend`, existence-only) are two
/// different tests over the same map — that gap is exactly the window
/// `/extend` exists to rescue something in, per issue #1805.
#[tokio::test]
async fn extending_beats_a_pending_sweep_on_an_already_past_deadline_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // Parked at the epoch: this host's TTL has long since passed, and
    // nothing has swept either entry out of the gate yet.
    let control = park_for_extend(&runtime, "appr-control", 1).await;
    let target = park_for_extend(&runtime, "appr-target", 1).await;

    let app = router(state.clone());

    // The control proves the premise: resolving an untouched twin of the
    // same stale park reports `expired`.
    let resolved = app
        .clone()
        .oneshot(resolve_request(
            &control,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let body = body_json(resolved).await;
    assert_eq!(
        body["outcome"], "expired",
        "premise: a park this old is already past this host's TTL, got {body}"
    );

    // Extending the other twin, before anything else touches it, must
    // still succeed — this is the whole reason `/extend` exists.
    let extended = app.clone().oneshot(extend_request(&target)).await.unwrap();
    assert_eq!(
        extended.status(),
        StatusCode::OK,
        "extend must be able to rescue a park the sweep has not yet reclaimed"
    );

    // And now resolving it must NOT report `expired` — the deadline
    // genuinely moved, not just the extend receipt's word for it.
    let resolved = app
        .oneshot(resolve_request(
            &target,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let body = body_json(resolved).await;
    assert_ne!(
        body["outcome"], "expired",
        "extend must genuinely push the deadline out, not just answer as if it did: {body}"
    );
}

/// PLAT-014 (Member ⇒ approve): the sharpest of the auth-matrix's four
/// rows. A Member sees a money-bearing approval exists (issue #468's
/// "waiting on approval" indicator has to survive for them) but not what
/// it is about (issue #618) — and cannot act on it at all: both
/// `POST {scope}/approvals/{aid}` and `/extend` are `AdminScopedCompany`.
/// All three properties are asserted against the same parked approval, so
/// the redaction and the auth gate cannot silently disagree about which
/// one is doing the protecting.
#[tokio::test]
async fn a_member_cannot_read_or_act_on_a_money_bearing_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-member", crate::ports::now_millis()).await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    // Sees it exists, but not what it costs.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/approvals")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let listed = body
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == approval.to_string())
        .expect("the approval is visible to a member");
    assert_eq!(
        listed["contents_hidden"], true,
        "a member must be told the contents were withheld: {listed}"
    );
    assert!(
        listed["amount_usd"].is_null(),
        "a member must not receive the dollar amount: {listed}"
    );
    assert!(
        listed["payload"].is_null(),
        "a member must not receive the payload either: {listed}"
    );

    // Cannot resolve it.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/approvals/{approval}"))
                .header("cookie", &member_cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "verdict": "approve" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a member must not be able to approve a parked effect"
    );

    // Cannot extend it either.
    let response = app
        .oneshot(extend_request_with_cookie(&approval, member_cookie))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a member must not be able to extend a parked effect's deadline"
    );
}

/// POL-011: `extend_approval` is one handler mounted under both scope
/// forms (`scoped("/approvals/{aid}/extend", ...)`), so the platform
/// `/companies/{id}/...` form must carry the exact same admin gate the
/// `/company/...` alias does — and must not become a side channel that
/// resolves against the wrong company merely because its id rode in the
/// path instead of the alias.
#[tokio::test]
async fn extend_on_the_scoped_route_form_enforces_admin_and_the_right_company() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-scoped", crate::ports::now_millis()).await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let admin_cookie = crate::server::test_support::fixed_cookie("acme");
    let app = router(state);

    // AUTH: a member is refused on the scoped form exactly as on the alias.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/acme/approvals/{approval}/extend"
                ))
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // FAIL: addressing a *different* company id on the scoped form must
    // 404 rather than reach into `acme`'s gate — the path segment is the
    // only thing naming the company here, unlike the alias.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/globex/approvals/{approval}/extend"
                ))
                .header("cookie", &admin_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a company id that does not exist must not extend acme's approval"
    );
    assert!(
        runtime.pending_approvals().iter().any(|a| a.id == approval),
        "the approval must still be sitting under its real company, untouched"
    );

    // And the scoped form works for the right admin and the right company.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/acme/approvals/{approval}/extend"
                ))
                .header("cookie", &admin_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// **Issue #383 / #380 defect 3 — the keystone.** A client that walks away
/// mid-turn must not take the agent's continuation with it.
///
/// The host is plain `axum::serve(listener, router(state))` and nothing on
/// the resolve path was spawned, so the follow-up agent turn lived *inside*
/// the request future. Hyper drops that future the moment the peer closes,
/// and nginx closes its upstream connection when it gives up on a slow
/// response. So on a hosted tenant the sequence was: verdict recorded,
/// journaled, single-use grant minted — and then the re-dispatch the grant
/// existed for cancelled mid-flight. The operator's approval was spent and
/// the conversation never resumed, which is precisely what #380 reported.
///
/// `Router::oneshot` reproduces that cancellation faithfully rather than by
/// analogy: the mechanism is the same one hyper uses — the handler future is
/// owned by the future the caller is polling, and dropping the latter drops
/// the former.
#[tokio::test]
async fn a_dropped_connection_does_not_cancel_the_follow_up_cycle() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    // Approve it, then let the connection die once the turn is under way.
    let mut resolving = Box::pin(c.app.clone().oneshot(resolve_request(
        &c.approval_id,
        serde_json::json!({"verdict":"approve"}),
    )));
    tokio::select! {
        _ = &mut resolving => panic!("the resolve answered before the follow-up turn began"),
        _ = c.entered.notified() => {}
    }
    drop(resolving);

    // The verdict is already durable and the grant already spent — this is
    // the state the operator is left in when the proxy gives up.
    assert!(
        !c.runtime
            .pending_approvals()
            .iter()
            .any(|a| a.id == c.approval_id),
        "the verdict was journaled before the connection dropped"
    );
    assert!(
        c.runtime.grants.peek(&c.approval_id).is_some(),
        "the single-use grant was minted before the connection dropped"
    );

    // So the continuation the grant exists for must still complete.
    c.release.notify_one();
    assert!(
        await_continuation(&c.runtime).await,
        "the follow-up cycle died with the dropped connection: the grant is spent \
         and the agent never continued"
    );
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "the continuation minted no second grant"
    );
}

/// **Issue #882.** A chat turn whose caller walks away mid-flight must still
/// finish and still journal its answer.
///
/// This is the chat-lane twin of
/// `a_dropped_connection_does_not_cancel_the_follow_up_cycle`. Both the
/// cycle and the `AgentReply` append used to live inside the request future,
/// so a turn slower than nginx's read timeout was cancelled mid-flight and
/// the answer was never written. The operator's DM history then held their
/// question and nothing else — the turn could not be read back on reload and
/// could not be resumed, which is what #882 reported. Workflow runs survived
/// the identical 504 precisely because they are spawned.
///
/// `Router::oneshot` reproduces the cancellation by the same mechanism hyper
/// uses: the handler future is owned by the future the caller polls, so
/// dropping the latter drops the former.
#[tokio::test]
async fn a_dropped_connection_does_not_lose_the_chat_turns_work() {
    let home_dir = home();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(Arc::new(StalledChatBrain {
            entered: entered.clone(),
            release: release.clone(),
        })),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    // Send the turn, then let the connection die once it is under way —
    // exactly what the proxy does when it decides the upstream is too slow.
    let mut chatting = Box::pin(app.clone().oneshot(chat_request("run the seo audit")));
    tokio::select! {
        _ = &mut chatting => panic!("the chat answered before the turn began"),
        _ = entered.notified() => {}
    }
    drop(chatting);

    // Nothing is journaled yet: the turn is still stalled inside the brain.
    assert!(
        !reply_journaled(&runtime).await,
        "the reply was journaled before the turn was released"
    );

    // Issue #983: the turn was recorded the instant it was accepted, and
    // the record is what a re-read resolves — so at this point the operator
    // has walked away and the turn is still `Running` rather than absent.
    let row = turn_rows(&runtime)
        .await
        .pop()
        .expect("accepting the turn minted a row");
    assert_eq!(
        row.1, "running",
        "a turn whose caller is gone must still read as under way"
    );

    // The work must survive the caller giving up.
    release.notify_one();
    assert!(
        await_reply_journaled(&runtime).await,
        "the chat turn died with the dropped connection: the operator's \
         message is journaled, the answer is not, and the turn can neither \
         be read back nor resumed (issue #882)"
    );

    // Issue #983: and so must the settle. The row is written by the spawned
    // task, not by the handler, so a dropped connection leaving it
    // `Running` forever would be the #882 bug one layer down — the turn
    // finishes, the answer lands, and the status surface still claims work
    // is in flight until the next boot reaps it.
    until("the settle died with the dropped connection", async || {
        turn_rows(&runtime)
            .await
            .iter()
            .all(|(_, status)| status == "succeeded")
    })
    .await;
}

// ── Issue #983: an accepted turn exists and can be read back ────────────

/// A brain that blocks every operator turn on a semaphore the test holds.
///
/// Deliberately a `Semaphore` rather than a `Notify`: these tests run two
/// turns at once and release both, and `notify_one` wakes exactly one
/// waiter while `notify_waiters` wakes only those already parked. Permits
/// are held whether or not anybody is waiting yet, so the release cannot
/// race the turns into a hang.
struct BlockingChatBrain {
    /// One permit added per turn that has entered the brain.
    entered: Arc<tokio::sync::Semaphore>,
    /// The test's permission for a turn to finish — one permit each.
    release: Arc<tokio::sync::Semaphore>,
}
