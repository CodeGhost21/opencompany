use crate::company::blocker_sender::BlockerSenderSignals;
use crate::company::runtime::{BlockerReplyPlan, CompanyRuntime};
use crate::company::task_intent::BlockerReplyIntent;
use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};
use crate::ports::types::CompanyId;
use std::sync::Arc;
use tempfile::TempDir;

async fn runtime() -> (Arc<CompanyRuntime>, TempDir) {
    let home = tempfile::Builder::new()
        .prefix("opencompany-blocker-dms-")
        .tempdir()
        .expect("tempdir");
    let manifest: crate::company::CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
    )
    .expect("manifest");
    let runtime = Arc::new(
        crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(CompanyId::new("acme"))
            .build()
            .await
            .expect("runtime"),
    );
    (runtime, home)
}

fn blocker(task_id: &str, group_key: Option<&str>) -> BlockerPayload {
    BlockerPayload {
        kind: BlockerKind::Infrastructure,
        source: BlockerSource::Tool,
        step: Some(BlockerStep::Task {
            task_id: task_id.to_string(),
        }),
        reason: format!("could not connect to mcp server for {task_id}"),
        needed: "the integration reconnected from Apps".to_string(),
        group_key: group_key.map(str::to_string),
    }
}

fn assignee(id: &str) -> BlockerSenderSignals {
    BlockerSenderSignals {
        started_by: None,
        owner_desk: None,
        assignee: Some(id.to_string()),
    }
}

/// A blocker parks into its teammate's DM: the approval's thread is that
/// DM, and a `blocker_parked` notification is filed pointing at it — with
/// no payload beyond the one-line title.
#[tokio::test]
async fn a_blocker_surfaces_in_the_responsible_teammates_dm() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
        .await
        .expect("parks");

    let pending = runtime.pending_approvals();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].thread.as_deref(),
        Some("dm:eng"),
        "the card routes into the DM with the teammate it is attributed to"
    );

    let notes = runtime
        .notifications()
        .list(runtime.id(), "eng")
        .await
        .expect("notifications");
    let parked = notes
        .iter()
        .find(|n| n.notification.kind == "blocker_parked")
        .expect("a blocker-parked notification is filed");
    assert_eq!(parked.notification.context.as_deref(), Some("dm:eng"));
    assert!(
        parked.notification.title.contains("eng"),
        "the title names who is blocked: {}",
        parked.notification.title
    );
}

/// The projection names which kind of step a parked blocker stopped
/// (issue #2028) — the console needs this to word `skip`/`cancel`
/// honestly, since neither does the same thing to a board card that it
/// does to a workflow node.
#[tokio::test]
async fn pending_approvals_names_the_stopped_steps_kind() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
        .await
        .expect("parks a task-step blocker");
    let node_payload = BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::Tool,
        step: Some(BlockerStep::Node {
            run_id: "run-1".to_string(),
            node_id: "draft".to_string(),
        }),
        reason: "needs a model choice".to_string(),
        needed: "which model to use".to_string(),
        group_key: None,
    };
    runtime
        .park_blocker(&node_payload, "t-2", assignee("eng"))
        .await
        .expect("parks a node-step blocker");

    let pending = runtime.pending_approvals();
    assert_eq!(pending.len(), 2);
    let kinds: std::collections::HashSet<_> = pending
        .iter()
        .map(|a| a.blocker_step_kind.clone())
        .collect();
    assert_eq!(
        kinds,
        std::collections::HashSet::from([
            Some("task".to_string()),
            Some("node".to_string())
        ]),
        "a task-step and a node-step blocker must project distinct step kinds, not the \
         same value: {pending:?}"
    );
}

/// The sender is resolved, not passed through: a park with no attribution
/// still lands in a real DM — the orchestrator's.
#[tokio::test]
async fn an_unattributed_blocker_falls_to_the_orchestrator_dm() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(
            &blocker("t-1", None),
            "t-1",
            BlockerSenderSignals::default(),
        )
        .await
        .expect("parks");
    assert_eq!(
        runtime.pending_approvals()[0].thread.as_deref(),
        Some("dm:ceo"),
        "with nothing named, the first (orchestrator) agent answers"
    );
}

/// Blockers sharing a root cause project as one group and are named by
/// the projection's `group_key`.
#[tokio::test]
async fn blockers_sharing_a_cause_group_together() {
    let (runtime, _home) = runtime().await;
    for task in ["t-1", "t-2", "t-3"] {
        runtime
            .park_blocker(
                &blocker(task, Some("connection:slack")),
                task,
                assignee("eng"),
            )
            .await
            .expect("parks");
    }
    let members = runtime.blocker_group_members("connection:slack", Some("task"));
    assert_eq!(
        members.len(),
        3,
        "every card on the broken connection is one group"
    );
    for summary in runtime.pending_approvals() {
        assert_eq!(summary.group_key.as_deref(), Some("connection:slack"));
    }
}

/// **P1 review finding on PR #2038.** A connection failure can stop
/// both a board card and a workflow node, and both park with the same
/// `connection:<name>` group key — but Skip means "produces nothing"
/// to a node and "redispatch, run it again" to a task. Fanning one
/// verdict across the two step kinds silently applies the wrong
/// consequence to whichever wasn't addressed, so the fan-out group
/// must split by step kind even when the root cause is shared.
#[tokio::test]
async fn a_shared_cause_never_fans_a_verdict_across_step_kinds() {
    let (runtime, _home) = runtime().await;
    let task_id = runtime
        .park_blocker(
            &blocker("t-1", Some("connection:slack")),
            "t-1",
            assignee("eng"),
        )
        .await
        .expect("parks a task-step blocker");
    let node_payload = BlockerPayload {
        kind: BlockerKind::Infrastructure,
        source: BlockerSource::Tool,
        step: Some(BlockerStep::Node {
            run_id: "run-1".to_string(),
            node_id: "draft".to_string(),
        }),
        reason: "could not connect to mcp server for run-1".to_string(),
        needed: "the integration reconnected from Apps".to_string(),
        group_key: Some("connection:slack".to_string()),
    };
    let node_id = runtime
        .park_blocker(&node_payload, "t-2", assignee("eng"))
        .await
        .expect("parks a node-step blocker on the same connection");

    let fanned = runtime
        .parked_blocker_group(&task_id)
        .expect("the task blocker is still parked");
    assert_eq!(
        fanned,
        vec![task_id.clone()],
        "the task blocker's fan-out group must not include the node-step sibling \
         just because they share a connection: {fanned:?}"
    );

    let (_, follow_up) = runtime
        .apply_blocker_reply_spawned(
            &fanned,
            &task_id,
            crate::ports::blockers::BlockerVerdict::Skip,
            "",
            None,
        )
        .await
        .expect("resolves the task blocker alone");
    crate::company::runtime::join_follow_up(follow_up)
        .await
        .expect("follow-up runs");

    assert!(
        runtime.pending_approvals().iter().any(|p| p.id == node_id),
        "skipping the task card must not have also skipped the workflow node — \
         it is still stalled on the same connection and still needs its own answer"
    );
}

/// A reply in a DM with a single pending blocker resolves it, and a
/// grouped reply fans the verdict to every card in the group.
#[tokio::test]
async fn a_reply_resolves_the_whole_group_and_fans_the_verdict() {
    let (runtime, _home) = runtime().await;
    for task in ["t-1", "t-2"] {
        runtime
            .park_blocker(
                &blocker(task, Some("connection:slack")),
                task,
                assignee("eng"),
            )
            .await
            .expect("parks");
    }
    let plan = runtime
        .plan_blocker_reply("dm:eng", None, "go ahead and retry")
        .await
        .expect("plan");
    let ids = match plan {
        BlockerReplyPlan::Resolve { ids, intent } => {
            assert_eq!(intent, BlockerReplyIntent::Retry);
            assert_eq!(ids.len(), 2, "one card, both parks");
            ids
        }
        _ => panic!("a single group in the DM resolves"),
    };
    runtime
        .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "go ahead and retry", None)
        .await
        .expect("applies");
    assert!(
        runtime.pending_approvals().is_empty(),
        "the verdict fanned to every card in the group"
    );
}

/// Parks a blocker the way a cycle that came from **no** conversation
/// does: `cycle_conversation` answers with a default
/// `ApprovalConversation`, so the journal row carries `thread: None`.
/// Every planning-pass park written before commit `26d558c92` has the
/// same shape, and those rows survive journal replay.
async fn park_thread_less_blocker(runtime: &Arc<CompanyRuntime>, task_id: &str) {
    use crate::ports::types::{Effect, EffectGroup};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let payload = blocker(task_id, None);
    let effect = Effect {
        kind: payload.effect_kind(),
        group: EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).expect("payload"),
        agent: None,
        run_id: None,
    };
    let id = runtime
        .approvals
        .park(runtime.id(), effect.clone())
        .await
        .expect("parks");
    runtime
        .journal
        .record_parked(
            &id,
            &effect,
            super::now_millis(),
            TaskLink::from_task_id(Some(task_id)),
            ApprovalConversation::default(),
            None,
        )
        .await
        .expect("journals");
}

/// A blocker that names no conversation is pending in **no**
/// conversation — `#general` least of all.
///
/// The bug (B-059): `pending_blocker_groups` matched through
/// `same_conversation`, which reads a missing chat id as "unaddressed,
/// therefore General". A thread-less park therefore read as pending in
/// the company-wide line, and the founder's next top-level message there
/// was consumed as its *answer* — accepted, settled in milliseconds with
/// no cycle and no reply, and indistinguishable in the console from a
/// message being worked on.
///
/// All four General spellings are asserted because the fold admits all
/// four (`is_general_chat`), so fixing only the console's `"main"` would
/// leave the same drop reachable from a host addressing `"General"`.
#[tokio::test]
async fn a_thread_less_blocker_is_pending_in_no_conversation() {
    let (runtime, _home) = runtime().await;
    park_thread_less_blocker(&runtime, "t-1").await;
    assert_eq!(
        runtime.pending_approvals()[0].thread,
        None,
        "the park under test is the thread-less shape"
    );

    for desk in ["main", "general", "General", ""] {
        let plan = runtime
            .plan_blocker_reply(desk, None, "please retry the nightly import")
            .await
            .expect("plan");
        assert!(
            matches!(plan, BlockerReplyPlan::NotBlocker),
            "a top-level message in {desk:?} must run as an ordinary turn, not settle a \
             blocker no conversation raised: {plan:?}"
        );
    }
    assert_eq!(
        runtime.pending_approvals().len(),
        1,
        "nothing was consumed, so the blocker still pends for whoever can actually answer it"
    );
}

/// The carve-out is not a blanket refusal: a blocker stamped with a real
/// thread still answers to it. Guards the fix from being "skip every
/// blocker", which would pass the test above and break #1862 outright.
#[tokio::test]
async fn a_threaded_blocker_still_answers_in_its_own_dm() {
    let (runtime, _home) = runtime().await;
    park_thread_less_blocker(&runtime, "t-1").await;
    runtime
        .park_blocker(&blocker("t-2", None), "t-2", assignee("eng"))
        .await
        .expect("parks");

    let plan = runtime
        .plan_blocker_reply("dm:eng", None, "retry it")
        .await
        .expect("plan");
    match plan {
        BlockerReplyPlan::Resolve { ids, .. } => assert_eq!(
            ids.len(),
            1,
            "only the blocker stamped with this DM is in scope; the thread-less one is in \
             no conversation and must not be fanned in"
        ),
        other => panic!("the DM's own blocker still resolves: {other:?}"),
    }
}

/// An unrelated reply is not a verdict — it falls through to an ordinary
/// turn rather than settling the blocker.
#[tokio::test]
async fn an_unrelated_reply_is_not_a_verdict() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
        .await
        .expect("parks");
    let plan = runtime
        .plan_blocker_reply("dm:eng", None, "hey, how's it going?")
        .await
        .expect("plan");
    assert!(
        matches!(plan, BlockerReplyPlan::NotBlocker),
        "a greeting runs as a normal turn and settles nothing"
    );
    assert_eq!(
        runtime.pending_approvals().len(),
        1,
        "the blocker still pends"
    );
}

/// Two distinct blocked things in one DM: a bare verdict asks which; a
/// verdict naming one resolves only that one.
#[tokio::test]
async fn several_blockers_disambiguate_by_name() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(
            &blocker("t-1", Some("connection:slack")),
            "t-1",
            assignee("eng"),
        )
        .await
        .expect("parks");
    runtime
        .park_blocker(
            &blocker("t-2", Some("connection:notion")),
            "t-2",
            assignee("eng"),
        )
        .await
        .expect("parks");

    let ambiguous = runtime
        .plan_blocker_reply("dm:eng", None, "retry it")
        .await
        .expect("plan");
    assert!(
        matches!(ambiguous, BlockerReplyPlan::AskWhich { .. }),
        "a bare verdict over two blocked things asks which"
    );

    let named = runtime
        .plan_blocker_reply("dm:eng", None, "retry slack")
        .await
        .expect("plan");
    match named {
        BlockerReplyPlan::Resolve { ids, .. } => {
            assert_eq!(
                ids,
                runtime.blocker_group_members("connection:slack", Some("task"))
            );
        }
        _ => panic!("naming the connection resolves only its group"),
    }
}

/// An explicit reply settles only a blocker parked in the same
/// conversation: a verdict threaded to another DM's blocker card, sent
/// from a desk with no blocker of its own, runs as an ordinary turn.
#[tokio::test]
async fn an_explicit_reply_stays_within_its_conversation() {
    let (runtime, _home) = runtime().await;
    runtime
        .park_blocker(
            &blocker("t-1", Some("connection:slack")),
            "t-1",
            assignee("eng"),
        )
        .await
        .expect("parks");
    let parent = runtime
        .events
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("read")
        .into_iter()
        .find(|stored| {
            matches!(
                stored.event,
                crate::ports::types::CompanyEvent::ApprovalParked { .. }
            )
        })
        .expect("the park is on the log")
        .seq;

    let same = runtime
        .plan_blocker_reply("dm:eng", Some(parent), "retry")
        .await
        .expect("plan");
    assert!(
        matches!(same, BlockerReplyPlan::Resolve { .. }),
        "a reply in the blocker's own DM resolves it"
    );

    let cross = runtime
        .plan_blocker_reply("dm:ops", Some(parent), "retry")
        .await
        .expect("plan");
    assert!(
        matches!(cross, BlockerReplyPlan::NotBlocker),
        "the same verdict from another conversation settles nothing"
    );
}

/// Manually parks a blocker with an arbitrary `at_millis` (and
/// therefore an arbitrary deadline), bypassing `park_blocker`'s
/// always-now stamp — the same technique `seed_parked` uses elsewhere
/// in this file, adapted to a real blocker payload so the group it
/// joins is genuine.
async fn park_blocker_at(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
    payload: &BlockerPayload,
    at_millis: u64,
) -> crate::ports::types::ApprovalId {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};
    let approval = crate::ports::types::ApprovalId::new(id);
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
        agent: None,
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
        .expect("seed parked blocker");
    approval
}

/// **Issue #2028 (P2 review finding).** `blocker_group_members` is
/// oldest-first, so the group's first receipt need not belong to the
/// id the request addressed. An older sibling can expire mid-loop
/// while the addressed blocker settles the requested verdict just
/// fine; the returned receipt must describe the ADDRESSED blocker,
/// not whichever member happens to be oldest.
#[tokio::test]
async fn the_addressed_members_own_outcome_is_reported_not_the_oldest_siblings() {
    let (runtime, _home) = runtime().await;
    let group = Some("connection:slack");
    // Ancient: already past its deadline against real wall-clock time.
    let old = park_blocker_at(&runtime, "old", &blocker("t-old", group), 1).await;
    // Fresh: parked now, nowhere near its deadline.
    let addressed = runtime
        .park_blocker(&blocker("t-new", group), "t-new", assignee("eng"))
        .await
        .expect("parks the addressed blocker");

    let (receipt, follow_up) = runtime
        .apply_blocker_reply_spawned(
            &[old.clone(), addressed.clone()],
            &addressed,
            crate::ports::blockers::BlockerVerdict::Retry,
            "",
            None,
        )
        .await
        .expect("resolves the group");
    crate::company::runtime::join_follow_up(follow_up)
        .await
        .expect("follow-ups run");

    assert_eq!(
        receipt.outcome(),
        "settled",
        "the addressed blocker settled the requested verdict just fine — reporting \
         anything else (e.g. the oldest sibling's \"expired\") tells the operator \
         their own decision failed when it did not: {receipt:?}"
    );

    // Sanity on the test's own premise: the older sibling really did
    // expire in this same call, so a naive "receipts[0]" implementation
    // would have reported exactly that outcome instead.
    assert!(
        runtime.pending_approvals().is_empty(),
        "both members left the pending queue — one settled, one expired"
    );
}

/// **Issue #2028 (P2 review finding).** `parked_blocker_group` returns
/// `None` for BOTH "never a blocker" and "was a blocker, already
/// resolved" — `resolve_blocker` used to 400 either way. A blocker
/// that just settled (another tab, a double-click, a sibling's fan-out
/// beating this request) must answer the same idempotent
/// `AlreadyResolved` an ordinary approval's double-submit gets, not a
/// refusal that tells the operator their successful decision failed.
#[tokio::test]
async fn a_settled_blockers_late_request_is_already_resolved_not_refused() {
    let (runtime, _home) = runtime().await;
    let id = runtime
        .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
        .await
        .expect("parks");
    runtime
        .apply_blocker_reply(
            std::slice::from_ref(&id),
            BlockerReplyIntent::Retry,
            "go ahead",
            None,
        )
        .await
        .expect("resolves");
    assert!(
        runtime.parked_blocker_group(&id).is_none(),
        "test setup: the blocker is no longer parked"
    );

    let (receipt, follow_up) = runtime.already_resolved_blocker_receipt(&id).expect(
        "an id that WAS a blocker must get an idempotent answer once it has \
             resolved, not None (which the caller reads as \"never a blocker\" and \
             refuses)",
    );
    assert!(
        matches!(
            receipt,
            crate::runtime::cycle::ResolveReceipt::AlreadyResolved
        ),
        "a settled blocker's late request is AlreadyResolved, not an error: {receipt:?}"
    );
    crate::company::runtime::join_follow_up(follow_up)
        .await
        .expect("the synthetic already-resolved follow-up completes cleanly");
}

/// The other half of the same guard: an id that was never a blocker at
/// all — an unknown id, or an ordinary (non-blocker) approval — must
/// still be refused. Only "was a blocker, now resolved" gets the
/// idempotent answer.
#[tokio::test]
async fn an_id_that_was_never_a_blocker_gets_no_idempotent_answer() {
    let (runtime, _home) = runtime().await;
    assert!(
        runtime
            .already_resolved_blocker_receipt(&crate::ports::types::ApprovalId::new(
                "never-existed"
            ))
            .is_none(),
        "an unknown id must not be answered as a settled blocker"
    );
}

/// The paused card a parked blocker's approval links to.
async fn seed_paused_card(runtime: &Arc<CompanyRuntime>, id: &str) {
    use crate::ports::tasks::{COLUMN_PAUSED, TaskDeliverable, TaskRecord, TaskTitle};

    runtime
        .ops
        .tasks
        .upsert(
            &runtime.id,
            &TaskRecord {
                id: id.to_string(),
                title: TaskTitle::authored("Draft the launch note"),
                note: None,
                column: COLUMN_PAUSED.to_string(),
                priority: "medium".to_string(),
                assignee: "eng".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("dm:eng".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .expect("seed card");
}

/// A bare agent question: `step: None`, so the resume has only the
/// approval's task link to work from.
fn question() -> BlockerPayload {
    BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::AgentQuestion,
        step: None,
        reason: "which cluster should this deploy to?".to_string(),
        needed: "the cluster name".to_string(),
        group_key: None,
    }
}

/// Every verdict the durable journal banked for `id`, in append order —
/// read off disk, not off the in-memory map a resume consumes and
/// clears. What an operator's answer actually recorded.
async fn banked_verdicts(
    home: &std::path::Path,
    company: &CompanyId,
    id: &crate::ports::types::ApprovalId,
) -> Vec<String> {
    let path = crate::store::paths::Bundle::new(home, company).journal_jsonl();
    let raw = tokio::fs::read_to_string(path).await.unwrap_or_default();
    raw.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|line| line["record"] == "BlockerResolved" && line["id"] == id.to_string())
        .filter_map(|line| line["resolution"]["verdict"].as_str().map(str::to_string))
        .collect()
}

/// **Issue #2028 — a late second verdict must not overwrite the answer
/// that already won, deterministically.** No threads: the first request
/// is resolved and resumed to completion, and only then does a second
/// arrive carrying a group list captured before any of it ran — exactly
/// what a second browser tab holds, and what every caller passes
/// (`parked_blocker_group` snapshots outside the lock).
///
/// The loser must write **nothing**. Before the fix it banked its own
/// `record_blocker_resolution` and armed its own answer before
/// `settle_approval` told it that it had lost, so the durable journal
/// gained a Cancel line for an approval the host had settled as Retry,
/// and the armed Cancel was left in the side-channel with no resume left
/// to consume it — for the next boot to re-arm and act on.
#[tokio::test]
async fn a_late_second_verdict_banks_nothing_over_the_answer_that_won() {
    use crate::ports::blockers::BlockerVerdict;

    let (runtime, home) = runtime().await;
    let id = runtime
        .park_blocker(&question(), "t-1", assignee("eng"))
        .await
        .expect("parks");
    // Captured BEFORE the first request runs, and reused afterwards —
    // the stale snapshot every caller holds.
    let group = runtime
        .parked_blocker_group(&id)
        .expect("the blocker is parked");

    let (winner, follow_up) = runtime
        .apply_blocker_reply_spawned(&group, &id, BlockerVerdict::Retry, "", None)
        .await
        .expect("the first request resolves");
    assert_eq!(winner.outcome(), "settled", "the first request wins");
    crate::company::runtime::join_follow_up(follow_up)
        .await
        .expect("its resume runs to completion");

    let (loser, follow_up) = runtime
        .apply_blocker_reply_spawned(&group, &id, BlockerVerdict::Cancel, "", None)
        .await
        .expect("the late request is answered, not refused");
    crate::company::runtime::join_follow_up(follow_up)
        .await
        .expect("it owes no resume");
    assert_eq!(
        loser.outcome(),
        "already_resolved",
        "the late request settled nothing: {loser:?}"
    );

    let banked = banked_verdicts(home.path(), runtime.id(), &id).await;
    assert_eq!(
        banked,
        vec!["retry".to_string()],
        "the durable journal must hold only the verdict that actually settled; a \
         losing request that banks its own is the record disagreeing with the \
         approval event about what the operator decided: {banked:?}"
    );
    assert!(
        runtime.grants.peek_blocker_resolution(&id).is_none(),
        "a losing request must leave nothing armed — an answer banked with no resume \
         left to consume it is what the next boot re-arms and carries out"
    );
}

/// **Issue #2028 (P1 review finding) — the same race, run as a race.**
/// Two operators resolve one blocker with different verdicts
/// concurrently, on a multi-thread runtime so the two really interleave.
/// Whichever verdict the durable approval event names must be the one
/// the resume acts on, the only one banked, and the only one left armed.
///
/// Repeated over fresh runtimes because the losing order is what varies:
/// a single round can have the loser arrive after the winner's resume
/// has already consumed the entry, which is the benign interleaving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_resolves_cannot_desync_the_armed_verdict_from_the_settled_one() {
    for round in 0..15 {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            one_concurrent_round(round),
        )
        .await
        .expect("a resolve round must not hang");
    }
}

async fn one_concurrent_round(round: usize) {
    use crate::ports::blockers::BlockerVerdict;

    let (runtime, home) = runtime().await;
    let payload = question();
    seed_paused_card(&runtime, "t-1").await;
    let id = runtime
        .park_blocker(&payload, "t-1", assignee("eng"))
        .await
        .expect("parks");

    // Two concurrent requests naming different verdicts for the SAME
    // id. `apply_blocker_reply_spawned` serializes internally, so this
    // is a real race on the lock, not a hand-arranged interleaving.
    let a = {
        let rt = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move {
            rt.apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                BlockerVerdict::Retry,
                "",
                None,
            )
            .await
        })
    };
    let b = {
        let rt = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move {
            rt.apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                BlockerVerdict::Cancel,
                "",
                None,
            )
            .await
        })
    };
    let (a, b) = tokio::join!(a, b);
    let a = a.expect("task a joins");
    let b = b.expect("task b joins");

    // Exactly one of the two racing requests actually claims the
    // approval (`settle_approval`'s atomic `resolve_outcome`); the
    // loser reads `AlreadyResolved`. Whichever wins, its verdict is
    // what both the durable event AND the armed resume must agree on.
    #[allow(clippy::type_complexity)]
    let settled = |r: &crate::Result<(
        crate::runtime::cycle::ResolveReceipt,
        tokio::task::JoinHandle<crate::Result<crate::runtime::types::CycleReport>>,
    )>| {
        matches!(
            r,
            Ok((crate::runtime::cycle::ResolveReceipt::Settled(_), _))
        )
    };
    let winner_verdict = match (settled(&a), settled(&b)) {
        (true, false) => BlockerVerdict::Retry,
        (false, true) => BlockerVerdict::Cancel,
        (won_a, won_b) => panic!(
            "exactly one request must settle the approval: a settled={won_a} \
             b settled={won_b}"
        ),
    };

    for outcome in [a, b] {
        let (_, follow_up) = outcome.expect("resolves or is already-resolved");
        crate::company::runtime::join_follow_up(follow_up)
            .await
            .expect("follow-up runs");
    }

    // Retry and cancel post different notes into the DM, and exactly
    // one resume runs, so the note that landed must match the winner.
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

    let (expected, contradicting) = match winner_verdict {
        BlockerVerdict::Retry => (
            "Got it — picking that back up now.",
            "Okay — I've cancelled that. It's back in To-do if you want to pick it up \
             later.",
        ),
        BlockerVerdict::Cancel => (
            "Okay — I've cancelled that. It's back in To-do if you want to pick it up \
             later.",
            "Got it — picking that back up now.",
        ),
        _ => unreachable!(),
    };
    assert!(
        notes.iter().any(|n| n.as_str() == expected),
        "round {round}: the resume must post the WINNING verdict's note \
         ({expected:?}); posted: {notes:?}"
    );
    assert!(
        !notes.iter().any(|n| n.as_str() == contradicting),
        "round {round}: the resume must never carry out the LOSING request's verdict \
         — found its note ({contradicting:?}) even though the durable event named \
         {winner_verdict:?}: {notes:?}"
    );

    // The note only catches the loser when it overwrote the arming
    // *before* the winner's resume consumed it, which is the narrow
    // window. The journal catches it every time: a losing request that
    // banks at all leaves a second verdict on the record for an
    // approval only one verdict ever settled.
    let banked = banked_verdicts(home.path(), runtime.id(), &id).await;
    assert_eq!(
        banked,
        vec![winner_verdict.as_str().to_string()],
        "round {round}: only the verdict that settled may be banked; the durable \
         record must not disagree with the approval event: {banked:?}"
    );
    assert!(
        runtime.grants.peek_blocker_resolution(&id).is_none(),
        "round {round}: nothing may stay armed once the one resume this approval \
         owed has run — a leftover answer is what the next boot re-arms and acts on"
    );
}
