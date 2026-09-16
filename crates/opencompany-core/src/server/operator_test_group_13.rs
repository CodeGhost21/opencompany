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

/// The grants list is empty on a fresh company, and revoking something that
/// is not there is a 404 rather than a cheerful no-op.
#[tokio::test]
async fn the_grants_list_starts_empty_and_revoking_nothing_is_a_404() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/grants")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 0);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/nope")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A standing grant is listed with the authenticated user's id, is revocable,
/// and revoking is idempotent-by-404. Both scope forms answer.
#[tokio::test]
async fn a_standing_grant_is_listed_under_its_granter_and_can_be_revoked() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    runtime
        .grants
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("g1"),
            agent: "ops".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Approve,
            granted_by: Actor {
                kind: ActorKind::User,
                id: "user-7".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: 1_000,
            expires_at_millis: crate::ports::now_millis() + 60 * 60 * 1000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });

    // Both addressing forms list it.
    for uri in ["/api/v1/company/grants", "/api/v1/companies/acme/grants"] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value[0]["id"], "g1", "{uri}");
        assert_eq!(value[0]["tool"], "workspace_write");
        assert_eq!(value[0]["agent"], "ops");
        assert_eq!(
            value[0]["granted_by"]["id"], "user-7",
            "the list names who actually granted it"
        );
        assert!(
            value[0].get("payload").is_none() && value[0].get("args").is_none(),
            "a standing grant has no arguments, so the list opens no redaction surface"
        );
    }

    // Revoke, then it is gone and a second revoke is a 404.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g1")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(runtime.standing_grants().len(), 0);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g1")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// GRANT-012 (AUTH): `GET {scope}/grants` stays readable by any member —
/// the same consistency `GET {scope}/tools/grants` holds — but revoking one
/// is an admin action (issue #2169). A Member must see the list and be
/// refused the delete.
#[tokio::test]
async fn a_member_may_list_standing_grants_but_not_revoke_one() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    runtime
        .grants
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("g-member"),
            agent: "ops".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Approve,
            granted_by: Actor {
                kind: ActorKind::User,
                id: "user-7".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: 1_000,
            expires_at_millis: crate::ports::now_millis() + 60 * 60 * 1000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/grants")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a member may read the standing-grants list"
    );
    let body = body_json(response).await;
    assert_eq!(body[0]["id"], "g-member");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g-member")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "revoking a standing grant is an admin action, matching the tools/grants plane"
    );
}

/// GRANT-012 (FAIL): `revoke_standing` is a plain map removal with no
/// expiry check of its own — a grant past its deadline that nothing has
/// *swept* yet is still found and revoked normally (204), exactly as
/// `/extend` can still rescue a not-yet-swept approval. Only once
/// `sweep_standing` has actually removed it does revoke correctly answer
/// the "nothing to revoke" 404 the route's own doc promises — the same
/// distinction as an already-revoked id, never a 500.
#[tokio::test]
async fn revoking_a_grant_is_404_only_once_it_is_actually_swept() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let stale_grant = |id: &str| crate::runtime::grants::StandingGrant {
        id: crate::runtime::grants::GrantId::new(id),
        agent: "ops".into(),
        workflow: None,
        tool: "workspace_write".into(),
        verdict: Verdict::Approve,
        granted_by: Actor {
            kind: ActorKind::User,
            id: "user-7".into(),
        },
        approval_id: ApprovalId::new("appr-1"),
        at_millis: 1_000,
        // Already in the past either way; only sweeping tells the two apart.
        expires_at_millis: 1_001,
        origin_thread: None,
        origin_parent: None,
        origin_task: None,
        scope: None,
    };
    runtime.grants.grant_standing(stale_grant("g-unswept"));
    runtime.grants.grant_standing(stale_grant("g-swept"));

    let app = router(state.clone());
    let delete = |id: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/company/grants/{id}"))
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // Past-deadline but not yet swept: still a normal, successful revoke.
    let response = delete("g-unswept").await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "an expired-but-unswept grant is still physically present, so revoking it is an \
         ordinary success — exactly as extend can still rescue an unswept approval"
    );

    // Now actually sweep the other one out from under the route.
    let swept = runtime.grants.sweep_standing(crate::ports::now_millis());
    assert_eq!(swept.len(), 1, "premise: the grant was in fact swept");

    let response = delete("g-swept").await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "once actually swept, revoke must report the same 'nothing to revoke' answer an \
         already-revoked id does"
    );
}

/// GRANT-012 (FAIL). `revoke_standing_grant` takes the grant out of the
/// live set **before** its durable journal append — the opposite order
/// from minting, and on purpose (see the function's own doc): a crash
/// here must fail toward no-permission, never toward a permission nobody
/// can see is still live. When the append then fails, the caller is told
/// the revoke failed, but the grant must already be gone from the live
/// set that actually governs future calls.
#[tokio::test]
async fn a_failed_revoke_append_still_removes_the_grant_from_the_live_set() {
    let home_dir = home();
    let store = std::sync::Arc::new(RefusingGrantRevokeStore {
        inner: crate::ports::journal::MemoryJournalStore::default(),
    });
    let m = manifest();
    let id = CompanyId::new("acme");
    let fs_store = FsCompanyStore::new(home_dir.path().to_path_buf());
    {
        use crate::ports::store::CompanyStore;
        fs_store
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
    }
    let runtime = RuntimeBuilder::new(home_dir.path().to_path_buf(), m)
        .with_id(id.clone())
        .with_journal_store(store)
        .build()
        .await
        .unwrap();
    let runtime = Arc::new(runtime);
    runtime
        .grants
        .grant_standing(racing_standing_grant("g-append-fail"));

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id.clone(), runtime.clone());
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g-append-fail")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "the forced append failure must surface"
    );
    assert_eq!(
        runtime.grants.standing().len(),
        0,
        "the live-set removal must land even though the durable record of it failed — \
         fail toward no permission, never toward one nobody can see is still granted"
    );
}

// ---------------------------------------------------------------------
// Issue #469 — a turn that parks several approvals.
//
// Every test above parks exactly one, which is the case that always
// worked. The failure the operator hit needs more than one: four
// `composio_execute` calls from a single turn, all approved, and then
// silence. These drive that shape end to end over the real router.
// ---------------------------------------------------------------------

/// A brain that parks `parks` gated tool calls on one operator message and
/// answers each `ApprovalResolved` it is told about.
///
/// Deliberately shaped like `HarnessBrain`'s approval arm rather than like a
/// convenient stub: it consults the live grant set and produces **no reply
/// at all** when there is no grant left to redeem, because that silent
/// no-op is exactly what the later of several follow-up cycles used to hit.
struct MultiParkBrain {
    parks: usize,
    /// One entry per `ApprovalResolved` the brain was handed, across all
    /// cycles.
    decisions: Arc<std::sync::Mutex<Vec<String>>>,
    /// How many cycles ran in total (the first is the chat turn).
    cycles: Arc<std::sync::atomic::AtomicUsize>,
    /// The runtime, so the brain can reach the grant set the way the
    /// harness's re-dispatch does. Filled by the test after the build.
    rt: Arc<std::sync::OnceLock<Arc<CompanyRuntime>>>,
    /// Fail the continuation cycle, to exercise defect 4.
    fail_continuation: bool,
    /// Stamp a workflow run id onto every parked effect (issue #1092), so
    /// the park records the shape a workflow node's gated tool call has:
    /// explicitly unlinked from any card, and carrying a run.
    run_id: Option<String>,
    /// An `@mention` to append to every continuation reply. Exercises the
    /// durable half of a reply's mention: the re-issue's reply journaling
    /// must badge the person it names, same as the `/chat` path.
    continuation_mention: Option<String>,
}

/// **The keystone (issue #469).** A turn that parks four sign-offs, all
/// approved, produces exactly ONE continuation — and an answer the operator
/// can actually see.
///
/// Before this, each resolve spawned its own follow-up cycle: four full
/// re-runs of one turn, each told about one decision. They did not race —
/// the per-company serial lock made them queue — but the later ones found
/// the grants the earlier ones had redeemed and produced nothing at all.
/// And none of it reached the operator either way, because the resolve
/// route never journaled a continuation's replies, so no `agent_reply`
/// frame was ever projected. Four approvals, four wasted turns, silence.
#[tokio::test]
async fn four_sign_offs_from_one_turn_produce_one_continuation() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 4, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    let mut handles = Vec::new();
    for id in &c.approvals {
        let app = c.app.clone();
        let request = approve_detached(id);
        handles.push(tokio::spawn(
            async move { app.oneshot(request).await.unwrap() },
        ));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap().status(), StatusCode::OK);
    }
    settle(&c.runtime, 4).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "one turn owes one continuation, not one per approval"
    );
    assert_eq!(
        c.decisions.lock().unwrap().len(),
        4,
        "the single continuation carries every decision, so the brain learns all four"
    );
    assert!(
        c.runtime.pending_approvals().is_empty(),
        "every sign-off was decided"
    );
    assert_eq!(
        agent_replies(&c.runtime).await.len(),
        4,
        "the continuation's answers must reach the event stream, or the operator \
         watches an approved action in silence"
    );
}

/// The two orders an operator can decide in must end in the same place.
///
/// Approving four at once and approving them one at a time are the same
/// request spread over a different span, and the gate is the last decision
/// rather than a time window — so neither can produce more continuations
/// than the other. A design that coalesced only what arrived together would
/// pass the test above and still re-run the turn four times here.
#[tokio::test]
async fn deciding_one_at_a_time_ends_where_deciding_all_at_once_does() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 4, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    for (i, id) in c.approvals.iter().enumerate() {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let ran = c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before;
        if i < 3 {
            assert_eq!(
                ran,
                0,
                "the turn is still blocked on {} more sign-off(s); continuing now \
                 would re-park them",
                3 - i
            );
        }
    }
    settle(&c.runtime, 4).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "the last decision unblocks the turn, and it runs once"
    );
    assert_eq!(c.decisions.lock().unwrap().len(), 4);
    assert_eq!(agent_replies(&c.runtime).await.len(), 4);
}

/// The continuation answers in the conversation the sign-off was raised in.
///
/// Not on the answering agent's own line: a desk channel's request and a
/// direct message to that channel's lead are answered by the same teammate,
/// so keying the reply on the agent delivers a channel's continuation into a
/// private thread nobody is watching (issue #379's lesson, which the reply
/// path had never learned — only the re-park had).
#[tokio::test]
async fn a_continuation_answers_in_the_thread_the_sign_off_was_raised_in() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 2, Some("sales"), false).await;

    for id in &c.approvals {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    settle(&c.runtime, 2).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(replies.len(), 2, "both re-issues answered");
    assert!(
        replies.iter().all(|r| r.starts_with("sales|")),
        "the continuation must land in the channel the approval was raised in, got {replies:?}"
    );
}

/// **Issue #1092.** A workflow node's parked call, once approved, answers
/// on its run — never as a direct message from the teammate that ran it.
///
/// This is the wiring test for `continuation_fallback_chat_id`: the unit
/// tests pin what the fallback *returns*, and this pins that
/// `publish_continuation` actually uses it, through a real park, a real
/// resolve and the journal the console reads back.
///
/// The assertion is written against the agent id rather than only for the
/// run id, because that is the regression: the leak put the re-issued
/// turn's narration into `chat/history?desk=<teammate>`, where it rendered
/// as an unprompted DM.
#[tokio::test]
async fn a_workflow_parks_continuation_answers_on_the_run_not_in_a_dm() {
    let home_dir = home();
    let c =
        multi_park_company_run(home_dir.path(), 1, None, false, Some("run-1092"), None).await;

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(replies.len(), 1, "the re-issue answered once");
    let (chat_id, _) = replies[0].split_once('|').expect("chat_id|text");
    assert_eq!(
        chat_id, "run-1092",
        "a workflow park's continuation belongs to its run, got {replies:?}"
    );
    // The regression, stated as itself: before this fix the fallback was
    // the answering teammate's own id, so this is what the leaked row held.
    assert_ne!(
        chat_id, "ceo",
        "the re-issue must not be journaled as a DM from the teammate that ran it"
    );
}

/// **Codex P1 (pass 2).** A continuation's reply is journaled through
/// `publish_continuation`, not the `/chat` turn — so a mention an agent
/// types back in an approval follow-up used to render as a chip and
/// nothing else: no badge, no durable row, exactly the person it is meant
/// to reach (offline when the reply lands) getting neither.
///
/// Both paths file through the same writer now; this pins that an `@user`
/// in a continuation reply lands as a mention notification whose audience
/// carries the person named, under the chat the continuation answered in.
#[tokio::test]
async fn a_continuation_reply_that_mentions_a_user_files_a_notification() {
    let home_dir = home();
    let c = multi_park_company_run(
        home_dir.path(),
        1,
        Some("sales"),
        false,
        None,
        Some("@harness-admin"),
    )
    .await;

    let users = c
        .runtime
        .users()
        .list_users(&CompanyId::new("acme"))
        .await
        .unwrap();
    let admin = users
        .iter()
        .find(|u| u.email == "harness-admin@example.test")
        .expect("the fixed admin is seeded");
    assert_eq!(
        admin.status,
        crate::ports::users::UserStatus::Active,
        "the admin must be an active, mentionable target"
    );

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;
    // The notification is filed inside `publish_continuation`, after the
    // reply is journaled — `settle` only waits for the reply. A loaded CI
    // runner can reach this point before the notification append finishes,
    // so poll for it (issue #1665, Codex P1 regression).
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = c
                .runtime
                .notifications()
                .list(&CompanyId::new("acme"), &admin.id)
                .await
                .unwrap();
            if notes.iter().any(|n| n.notification.kind == "mention") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the mention notification never appeared");

    let notes = c
        .runtime
        .notifications()
        .list(&CompanyId::new("acme"), &admin.id)
        .await
        .unwrap();
    let mentions: Vec<_> = notes
        .into_iter()
        .filter(|n| n.notification.kind == "mention")
        .collect();
    assert_eq!(
        mentions.len(),
        1,
        "the continuation's mention must badge the person it names"
    );
    let note = &mentions[0].notification;
    assert_eq!(note.context.as_deref(), Some("sales"));
    assert_eq!(
        note.title, "Someone mentioned you in sales",
        "a continuation has no author, so the generic label is the honest one"
    );
    assert!(
        note.audience
            .as_ref()
            .is_some_and(|a| a.contains(&admin.id)),
        "the named user must be in the notification's audience"
    );
}
