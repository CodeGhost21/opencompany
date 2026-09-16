use super::*;
use crate::ports::tasks::TaskTitle;
use std::sync::Mutex as StdMutex;

use crate::ports::runs::RunStatus;
use crate::ports::types::{CompanyRecord, CompanySummary, LedgerEntry};

fn agent(id: &str, tier: Option<&str>) -> ManifestAgent {
    ManifestAgent {
        provider: None,
        global: false,
        id: id.to_string(),
        role: "Role".to_string(),
        name: None,
        description: None,
        tier: tier.map(str::to_string),
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    }
}

/// Issue #267: the brief's **shape** is the thing under test, because the
/// shape is what the model followed. Answering has to lead, the
/// never-a-card rule has to be stated rather than implied, authoring a
/// workflow has to read as something done in this turn, and the whole thing
/// has to be no longer than the version it replaced — a "rebalance" that
/// grew the brief would just be more prose competing with the lead.
#[test]
fn the_brief_leads_with_answering_and_did_not_grow() {
    let brief = orchestrator_brief();

    // The default leads. Measured by position, not by presence: the old
    // brief contained the same rule as its closing clause and behaviour
    // followed the enumeration instead.
    let answer_first = brief
        .find("MOST MESSAGES ARE QUESTIONS OR QUICK READS")
        .expect("the answering default is stated");
    for later in [
        "delegate_to_desk",
        "spawn_task",
        "create_workflow",
        "add_agent",
        "assign_task",
        "review_task",
    ] {
        let at = brief.find(later).unwrap_or_else(|| panic!("names {later}"));
        assert!(
            answer_first < at,
            "`{later}` is introduced before the answering default"
        );
    }

    assert!(
        brief.contains("is NEVER a card"),
        "the never-a-card rule must be stated, not implied: {brief}"
    );
    // The #442 two-decisions block survives the restructure.
    assert!(brief.contains("they are INDEPENDENT"), "{brief}");
    assert!(brief.contains("the hand-off IS the card"), "{brief}");
    // A "create a workflow" ask is authored now, not parked.
    assert!(
        brief.contains("author it NOW with `create_workflow`"),
        "the automate path must read as this-turn work: {brief}"
    );

    // The length of the brief this replaced. A ceiling, not a target.
    const PREVIOUS_LEN: usize = 2784;
    assert!(
        brief.len() <= PREVIOUS_LEN,
        "the brief grew to {} (was {PREVIOUS_LEN})",
        brief.len()
    );
}

/// Issue #276: both directions of the arming summary, including the name
/// and id, and neither the actor nor the reason.
#[test]
fn an_arming_change_summarizes_in_both_directions_without_the_actor_or_the_reason() {
    let event = |enabled, reason| CompanyEvent::WorkflowEnabledChanged {
        workflow_id: "digest".to_string(),
        name: "Daily digest".to_string(),
        enabled,
        reason,
        by: Some(crate::ports::types::Actor {
            kind: crate::ports::types::ActorKind::User,
            id: "u_secret".to_string(),
        }),
    };

    let off = summarize_event(&event(
        false,
        crate::ports::types::WorkflowEnabledReason::Disarmed,
    ));
    assert!(off.contains("switched off"), "{off}");
    assert!(off.contains("Daily digest"), "{off}");
    assert!(off.contains("digest"), "{off}");

    let on = summarize_event(&event(
        true,
        crate::ports::types::WorkflowEnabledReason::Operator,
    ));
    assert!(on.contains("switched on"), "{on}");
    assert!(on.contains("Daily digest"), "{on}");

    // The actor id never reaches the insight surface, and neither does the
    // rule-vs-person distinction — see the arm's comment.
    for summary in [&off, &on] {
        assert!(!summary.contains("u_secret"), "{summary}");
        assert!(!summary.contains("disarm"), "{summary}");
        assert!(!summary.contains("operator"), "{summary}");
    }
}

/// **Issue #248, the insight-surface twin of the sidecar guard.** This
/// one-liner is folded into the orchestrator's recent-activity context, so
/// it is read by a model rather than by the tenant. A delivery row's
/// `target` is a recipient's address and its `detail` quotes one when the
/// transport refuses, so neither may appear here. The exclusion was written
/// this way by #228; this pins it.
#[test]
fn a_finished_run_summarizes_to_counts_without_the_recipient_or_transport_text() {
    // `.invalid` is reserved by RFC 2606, so this fixture names nobody.
    const RECIPIENT: &str = "recipient@example.invalid";

    let summary = summarize_event(&CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: None,
        deliveries: vec![crate::ports::DeliveryReport {
            node: "owner_summary".to_string(),
            kind: "email".to_string(),
            target: Some(RECIPIENT.to_string()),
            status: crate::ports::DeliveryStatus::Failed,
            detail: format!(
                "the mail transport refused the message: 550 5.1.1 <{RECIPIENT}>: Recipient \
                 address rejected"
            ),
            reason: crate::ports::DeliveryReason::MailTransportRefused,
        }],
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    });

    assert!(!summary.contains(RECIPIENT), "{summary}");
    assert!(!summary.contains("recipient@"), "{summary}");
    assert!(!summary.contains("Recipient address rejected"), "{summary}");
    assert!(!summary.contains("550"), "{summary}");
    // Still useful: which workflow, and that something did not go out.
    assert!(summary.contains("digest"), "{summary}");
    assert!(summary.contains("1 not delivered"), "{summary}");
}

/// **Issue #383, the twin of the sidecar's pin.** The insight tail is the
/// other non-tenant reader of a finished run, and it had the same hole: a
/// cancelled run carries no error, so it summarized as a clean finish and
/// invited the orchestrator to reason about — or redo — work an operator had
/// just stopped.
#[test]
fn a_cancelled_run_summarizes_as_stopped_rather_than_finished() {
    let summary = summarize_event(&CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".to_string(),
        scheduled: true,
        run_id: Some("run-1".to_string()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: true,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    });

    assert!(summary.contains("stopped"), "{summary}");
    assert!(
        !summary.contains("finished"),
        "a stopped run must not read as a finished one: {summary}"
    );
}

/// **Issue #327.** A workspace write summarizes structurally — the change
/// word and the node id, nothing else.
///
/// The node's *name* is the exclusion with teeth. It is operator-authored
/// free text that routinely carries the substance of the note ("Q3 layoffs
/// shortlist"), and this string is a non-sensitive one-liner for the
/// insight surface, which is precisely where free text does not belong.
/// Same reasoning as the recipient exclusion two tests up; the arm was
/// written this way, and this is what pins it.
#[test]
fn a_workspace_write_summarizes_to_the_change_and_node_without_the_notes_name() {
    let summary = summarize_event(&CompanyEvent::WorkspaceChanged {
        node_id: "n-42".to_string(),
        change: "updated".to_string(),
    });

    // Exact, not `contains`: the whole claim is that nothing *else* is in
    // here. A future arm that looked the node up to add its name would keep
    // passing every `contains` assertion and fail this one.
    assert_eq!(summary, "workspace updated: n-42");
}

#[test]
fn orchestrator_id_prefers_the_tagged_agent() {
    let roster = vec![
        agent("ceo", None),
        agent("chief", Some("orchestrator")),
        agent("eng", Some("reasoning")),
    ];
    assert_eq!(orchestrator_id(&roster).as_deref(), Some("chief"));
}

#[test]
fn orchestrator_id_falls_back_to_first_agent() {
    let roster = vec![agent("ceo", None), agent("eng", None)];
    assert_eq!(orchestrator_id(&roster).as_deref(), Some("ceo"));
}

#[test]
fn orchestrator_id_is_none_for_an_empty_roster() {
    assert_eq!(orchestrator_id(&[]), None);
}

#[test]
fn delegation_tool_names_are_classified_internal() {
    assert!(is_delegation_tool(SPAWN_TASK_TOOL));
    assert!(is_delegation_tool(DELEGATE_TO_DESK_TOOL));
    assert!(is_delegation_tool(ADD_AGENT_TOOL));
    assert!(is_delegation_tool(CREATE_WORKFLOW_TOOL));
    // The read tool is NOT a delegation tool.
    assert!(!is_delegation_tool(QUERY_COMPANY_TOOL));
    assert!(!is_delegation_tool("send_email"));
}

#[test]
fn queue_drains_fifo_up_to_cap_and_discards_the_rest() {
    let queue = DelegationQueue::default();
    for i in 0..5 {
        queue.push(Delegation::SpawnTask {
            title: format!("t{i}"),
            note: None,
            assignee: None,
        });
    }
    assert_eq!(queue.queued(), 5);
    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(drained.len(), 3);
    // The first three (FIFO) survive; the queue is emptied.
    assert_eq!(
        drained[0],
        Delegation::SpawnTask {
            title: "t0".to_string(),
            note: None,
            assignee: None,
        }
    );
    assert_eq!(queue.queued(), 0);
}

#[test]
fn clear_empties_the_queue() {
    let queue = DelegationQueue::default();
    queue.push(Delegation::DelegateToDesk {
        desk: "strategy".to_string(),
        instruction: "plan".to_string(),
    });
    queue.clear();
    assert_eq!(queue.queued(), 0);
}

// ── Issue #419: the cap is announced, not silently applied ─────────────

/// The queue itself refuses past the cap rather than accepting work the
/// drain will destroy.
#[test]
fn push_within_cap_refuses_once_the_turn_is_full() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    for i in 0..MAX_DELEGATIONS_PER_TURN {
        assert_eq!(
            queue.push_within_cap(
                Delegation::SpawnTask {
                    title: format!("t{i}"),
                    note: None,
                    assignee: None,
                },
                MAX_DELEGATIONS_PER_TURN,
                NO_DEPTH_BOUND,
            ),
            Staged::Queued
        );
    }
    assert_eq!(
        queue.push_within_cap(
            Delegation::SpawnTask {
                title: "one too many".to_string(),
                note: None,
                assignee: None,
            },
            MAX_DELEGATIONS_PER_TURN,
            NO_DEPTH_BOUND,
        ),
        Staged::OverCap
    );
    assert_eq!(queue.queued(), MAX_DELEGATIONS_PER_TURN);
}

// ── Issue #453: no claim, no delegation ────────────────────────────────

/// The commitment is checked before the cap, and it is the answer an
/// unclaimed queue gives however empty it is. A model told "this turn is
/// full" would try again next turn; on an unclaimed path the next turn fails
/// identically, so the two refusals must stay distinguishable.
#[test]
fn an_unclaimed_queue_refuses_before_the_cap_is_even_consulted() {
    let queue = DelegationQueue::default();
    assert!(!queue.drain_committed(), "uncommitted is the default");
    assert_eq!(
        queue.push_within_cap(
            Delegation::SpawnTask {
                title: "first and only".to_string(),
                note: None,
                assignee: None,
            },
            MAX_DELEGATIONS_PER_TURN,
            NO_DEPTH_BOUND,
        ),
        Staged::NoDrain(NoDrainReason::Unwired),
        "an EMPTY unclaimed queue is still a queue nothing drains"
    );
    assert_eq!(queue.queued(), 0);
}

/// The RAII half, which is the one that was missing everywhere before #453:
/// an early exit — a `?`, a panic, a `return` from the middle of a turn —
/// must leave the queue empty and uncommitted, so the *next* caller inherits
/// a refusal rather than this one's abandoned work.
#[test]
fn a_claim_that_exits_early_un_commits_and_clears() {
    let queue = DelegationQueue::default();

    // A turn that queues work and then bails before draining.
    fn bail(queue: &DelegationQueue) -> Result<(), &'static str> {
        let _claim = queue.claim();
        assert_eq!(
            queue.push_within_cap(
                Delegation::ReviewTask {
                    task_id: "t1".to_string(),
                    decision: ReviewDecision::Approve,
                    note: None,
                },
                MAX_DELEGATIONS_PER_TURN,
                NO_DEPTH_BOUND,
            ),
            Staged::Queued
        );
        assert_eq!(queue.queued(), 1, "staged while the claim is live");
        Err("the turn failed after queuing")
    }

    assert!(bail(&queue).is_err());
    assert_eq!(
        queue.queued(),
        0,
        "the abandoned delegation must not survive the claim that staged it"
    );
    assert!(
        !queue.drain_committed(),
        "and the next caller must inherit a refusal, not this one's promise"
    );

    // Acquiring also clears, so a prior turn's leftovers can never be
    // executed for the caller that comes next.
    queue.push(Delegation::SpawnTask {
        title: "left behind".to_string(),
        note: None,
        assignee: None,
    });
    let _claim = queue.claim();
    assert_eq!(queue.queued(), 0);
}

/// The headline refusal, per tool, with the sentence each one owes the
/// model. The effect clause is the tool's own — a generic "refused" would
/// leave the model guessing which of its calls did not happen.
#[tokio::test]
async fn every_delegation_tool_refuses_when_nothing_will_drain() {
    let queue = DelegationQueue::default();
    let store = Arc::new(MemStore::seeded(desks_record(&CompanyId::new("acme"))))
        as Arc<dyn CompanyStore>;

    let cases: Vec<(Box<dyn Tool>, Value, &str)> = vec![
        (
            Box::new(SpawnTaskTool::new(
                queue.clone(),
                CompanyId::new("acme"),
                store.clone(),
            )),
            json!({ "title": "Ship it" }),
            "the card \"Ship it\" was NOT opened",
        ),
        (
            Box::new(DelegateToDeskTool::new(
                queue.clone(),
                CompanyId::new("acme"),
                store,
            )),
            json!({ "desk": "strategy", "instruction": "draft a plan" }),
            "nothing was handed to the strategy desk",
        ),
        (
            Box::new(AssignTaskTool::new(queue.clone())),
            json!({ "task_id": "t1", "assignee": "writer" }),
            "card t1 was NOT assigned",
        ),
        (
            Box::new(ReviewTaskTool::new(queue.clone())),
            json!({ "task_id": "t1", "decision": "approve" }),
            "card t1 was NOT reviewed",
        ),
    ];

    for (tool, args, effect) in cases {
        let name = tool.name().to_string();
        let result = tool.execute(args).await.expect("execute");
        assert!(result.is_error, "{name} must refuse: {}", result.text());
        let text = result.text();
        assert!(text.contains(effect), "{name}: {text}");
        // Not retryable — the next turn on this path drains no better.
        assert!(text.contains("Do not retry"), "{name}: {text}");
        // And the model must not narrate it as done, which is the whole
        // failure this replaces.
        assert!(text.contains("report the action as done"), "{name}: {text}");
        // Deliberately NOT the cap sentence: this is a different problem
        // with a different remedy.
        assert!(!text.contains("delegations"), "{name}: {text}");
    }
    assert_eq!(queue.queued(), 0, "nothing may be staged by a refusal");
}

/// **Issue #267 review, finding 3.** The two no-drain causes stop sharing a
/// sentence.
///
/// Written for a genuinely inert context, the refusal was then inherited by
/// a fully capable company whose triage read the message as a question —
/// where "board actions are unavailable in this context" is simply false as
/// the operator will hear it. Paired with a triage miss the experience was
/// *ask for a landing page → "I could not do it; board actions are
/// unavailable"*, with nothing to suggest that rephrasing would work.
///
/// The halves both causes need stay on both; what differs is what the model
/// is told happened, and what it can offer next.
#[tokio::test]
async fn the_triage_refusal_says_it_read_a_question_and_offers_a_way_forward() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim_answering();
    let refused = SpawnTaskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()),
    )
    .execute(json!({ "title": "Build the landing page" }))
    .await
    .expect("execute");
    assert!(refused.is_error, "{}", refused.text());
    let text = refused.text();

    assert!(
        text.contains("read as a question"),
        "it must name what actually happened: {text}"
    );
    assert!(
        text.contains("this message only"),
        "…and scope it to this message, not to the whole context: {text}"
    );
    assert!(
        text.contains("restate it"),
        "…and leave the model something recoverable to offer: {text}"
    );
    // The two claims that are false here, and were the whole complaint.
    assert!(
        !text.contains("nothing here can carry out board work"),
        "a capable company must not claim it cannot do board work: {text}"
    );
    assert!(
        !text.contains("unavailable in this context"),
        "the context is fine; the message was a question: {text}"
    );
    // …while everything both causes owe the model survives.
    assert!(text.contains("Do not retry"), "{text}");
    assert!(text.contains("report the action as done"), "{text}");
    assert!(
        text.contains("the card \"Build the landing page\" was NOT opened"),
        "the tool's own effect clause is untouched: {text}"
    );
    assert_eq!(queue.queued(), 0);
}

/// …and the inert-context refusal keeps saying the thing that is true only
/// of it, so the split is a split rather than a rename.
#[tokio::test]
async fn the_unwired_refusal_still_says_the_context_cannot_do_board_work() {
    let queue = DelegationQueue::default();
    let refused = SpawnTaskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()),
    )
    .execute(json!({ "title": "Ship it" }))
    .await
    .expect("execute");
    let text = refused.text();
    assert!(refused.is_error, "{text}");
    assert!(
        text.contains("nothing here can carry out board work"),
        "{text}"
    );
    assert!(!text.contains("read as a question"), "{text}");
}

/// The measurement finding 3 asks for: the two causes are distinguishable
/// as data, not only as prose. Without this the rate at which the triage
/// gate fires — the residual miss rate of a keyword classifier with teeth —
/// could not be counted apart from a genuinely unwired context.
#[test]
fn the_two_no_drain_causes_are_countable_apart() {
    let queue = DelegationQueue::default();
    let spawn = || Delegation::SpawnTask {
        title: "Build the landing page".to_string(),
        note: None,
        assignee: None,
    };
    assert_eq!(
        queue.push_within_cap(spawn(), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND),
        Staged::NoDrain(NoDrainReason::Unwired)
    );
    let claim = queue.claim_answering();
    assert_eq!(
        queue.push_within_cap(spawn(), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND),
        Staged::NoDrain(NoDrainReason::Triage)
    );
    // …and a hand-off is not refused at all under the same claim, because it
    // is how the question gets answered (finding 2).
    assert_eq!(
        queue.push_within_cap(
            Delegation::DelegateToDesk {
                desk: "eng".to_string(),
                instruction: "what did you ship?".to_string(),
            },
            MAX_DELEGATIONS_PER_TURN,
            NO_DEPTH_BOUND,
        ),
        Staged::Queued
    );
    drop(claim);
    assert_ne!(
        NoDrainReason::Unwired.as_str(),
        NoDrainReason::Triage.as_str(),
        "the log field must separate them"
    );
}

/// The defect #419 names: the tool told the model "it will be opened on the
/// board this turn" for a card the drain then threw away, so a turn asked
/// for five cards, reported five, and left two. The call past the cap is now
/// an **error** naming the bound, and nothing is queued.
#[tokio::test]
async fn spawn_task_refuses_past_the_cap_instead_of_promising_a_discarded_card() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = SpawnTaskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()),
    );
    for i in 0..MAX_DELEGATIONS_PER_TURN {
        let ok = tool
            .execute(json!({ "title": format!("item {i}") }))
            .await
            .expect("execute");
        assert!(!ok.is_error, "within the cap: {}", ok.text());
    }
    let refused = tool
        .execute(json!({ "title": "the fourth item" }))
        .await
        .expect("execute");
    assert!(refused.is_error, "{}", refused.text());
    let text = refused.text();
    assert!(text.contains("the fourth item"), "{text}");
    assert!(text.contains("NOT opened"), "{text}");
    assert!(
        text.contains(&MAX_DELEGATIONS_PER_TURN.to_string()),
        "the refusal names the bound: {text}"
    );
    // The queue is exactly full — the refusal queued nothing, so the drain
    // has nothing left over to destroy.
    assert_eq!(queue.queued(), MAX_DELEGATIONS_PER_TURN);
    assert_eq!(queue.drain(MAX_DELEGATIONS_PER_TURN).len(), 3);
}

/// Same for the hand-off tool, whose success line ("Its lead will answer
/// this turn") was the more misleading of the two: it claimed a teammate had
/// been given work nobody would ever run.
#[tokio::test]
async fn delegate_to_desk_refuses_past_the_cap() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    // An empty store loads no record, so desk grounding fails open and the
    // hand-off is queued exactly as it was before #272 — which isolates this
    // test to the cap.
    let store = Arc::new(MemStore::default()) as Arc<dyn CompanyStore>;
    let tool = DelegateToDeskTool::new(queue.clone(), CompanyId::new("acme"), store);
    for i in 0..MAX_DELEGATIONS_PER_TURN {
        let ok = tool
            .execute(json!({ "desk": "eng", "instruction": format!("item {i}") }))
            .await
            .expect("execute");
        assert!(!ok.is_error, "within the cap: {}", ok.text());
    }
    let refused = tool
        .execute(json!({ "desk": "eng", "instruction": "one more" }))
        .await
        .expect("execute");
    assert!(refused.is_error, "{}", refused.text());
    assert!(
        refused.text().contains("nothing was handed"),
        "{}",
        refused.text()
    );
    assert_eq!(queue.queued(), MAX_DELEGATIONS_PER_TURN);
}

/// The two board-lifecycle tools share the queue and therefore the cap, so
/// they share the refusal — an `assign_task` that silently did not assign is
/// the same defect wearing a different hat.
#[tokio::test]
async fn the_lifecycle_tools_refuse_past_the_cap_too() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let assign = AssignTaskTool::new(queue.clone());
    let review = ReviewTaskTool::new(queue.clone());
    for i in 0..MAX_DELEGATIONS_PER_TURN {
        assign
            .execute(json!({ "task_id": format!("t{i}"), "assignee": "eng" }))
            .await
            .expect("execute");
    }
    let refused_assign = assign
        .execute(json!({ "task_id": "t9", "assignee": "eng" }))
        .await
        .expect("execute");
    assert!(refused_assign.is_error, "{}", refused_assign.text());
    assert!(
        refused_assign.text().contains("NOT assigned"),
        "{}",
        refused_assign.text()
    );
    let refused_review = review
        .execute(json!({ "task_id": "t9", "decision": "approve" }))
        .await
        .expect("execute");
    assert!(refused_review.is_error, "{}", refused_review.text());
    assert!(
        refused_review.text().contains("NOT reviewed"),
        "{}",
        refused_review.text()
    );
    assert_eq!(queue.queued(), MAX_DELEGATIONS_PER_TURN);
}

#[tokio::test]
async fn spawn_task_tool_enqueues_a_task() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    // An empty store loads no record, so assignee grounding fails open and
    // the string is queued exactly as typed — isolating this test to the
    // plain enqueue path. Grounding itself is covered separately below.
    let tool = SpawnTaskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()),
    );
    tool.execute(json!({ "title": "Ship it", "note": "soon", "assignee": "eng" }))
        .await
        .expect("execute");
    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(
        drained,
        vec![Delegation::SpawnTask {
            title: "Ship it".to_string(),
            note: Some("soon".to_string()),
            assignee: Some("eng".to_string()),
        }]
    );
}

#[tokio::test]
async fn spawn_task_tool_requires_a_title() {
    let queue = DelegationQueue::default();
    let tool = SpawnTaskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()),
    );
    assert!(tool.execute(json!({ "note": "no title" })).await.is_err());
    assert_eq!(queue.queued(), 0);
}

// ── Issue #186 part b: the lifecycle tools ─────────────────────────────

#[tokio::test]
async fn assign_task_tool_enqueues_an_assignment() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = AssignTaskTool::new(queue.clone());
    tool.execute(json!({ "task_id": "t1", "assignee": "eng", "note": "closer to it" }))
        .await
        .expect("execute");
    assert_eq!(
        queue.drain(MAX_DELEGATIONS_PER_TURN),
        vec![Delegation::AssignTask {
            task_id: "t1".to_string(),
            assignee: "eng".to_string(),
            note: Some("closer to it".to_string()),
        }]
    );
}

#[tokio::test]
async fn assign_task_tool_requires_a_card_and_an_assignee() {
    let queue = DelegationQueue::default();
    let tool = AssignTaskTool::new(queue.clone());
    assert!(tool.execute(json!({ "assignee": "eng" })).await.is_err());
    assert!(tool.execute(json!({ "task_id": "t1" })).await.is_err());
    // A blank string is not an assignee.
    assert!(
        tool.execute(json!({ "task_id": "t1", "assignee": "  " }))
            .await
            .is_err()
    );
    assert_eq!(queue.queued(), 0);
}

#[tokio::test]
async fn review_task_tool_enqueues_both_verdicts() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = ReviewTaskTool::new(queue.clone());
    let approved = tool
        .execute(json!({ "task_id": "t1", "decision": "approve", "note": "good" }))
        .await
        .expect("approve");
    let revised = tool
        .execute(json!({ "task_id": "t2", "decision": "revise" }))
        .await
        .expect("revise");

    // Issue #453: staged truth, not the past tense. The card has not moved
    // when this sentence is written — the drain the claim promises is what
    // moves it — and saying otherwise is what made an undrained turn a lie
    // told through the agent.
    assert!(!approved.is_error);
    let text = approved.text();
    assert!(text.contains("Recorded your approval of card t1"), "{text}");
    assert!(text.contains("as this turn completes"), "{text}");
    assert!(!text.contains("has moved"), "nothing has moved yet: {text}");
    let text = revised.text();
    assert!(text.contains("card t2 returns to To-do"), "{text}");
    assert!(text.contains("as this turn completes"), "{text}");

    assert_eq!(
        queue.drain(MAX_DELEGATIONS_PER_TURN),
        vec![
            Delegation::ReviewTask {
                task_id: "t1".to_string(),
                decision: ReviewDecision::Approve,
                note: Some("good".to_string()),
            },
            Delegation::ReviewTask {
                task_id: "t2".to_string(),
                decision: ReviewDecision::Revise,
                note: None,
            },
        ]
    );
}

/// An unrecognised verdict is an error, never a silent approval — a card
/// must not pass review because the model typed something unexpected.
#[tokio::test]
async fn review_task_tool_rejects_an_unknown_verdict_rather_than_approving() {
    let queue = DelegationQueue::default();
    let tool = ReviewTaskTool::new(queue.clone());
    assert!(
        tool.execute(json!({ "task_id": "t1", "decision": "maybe" }))
            .await
            .is_err()
    );
    assert!(tool.execute(json!({ "task_id": "t1" })).await.is_err());
    assert_eq!(queue.queued(), 0, "nothing may be queued on a bad verdict");
}

/// Both lifecycle tools are internal delegation work, so the approval
/// policy must classify them as such — never as an external effect to park.
#[test]
fn the_lifecycle_tools_are_internal_delegation_tools() {
    assert!(is_delegation_tool(ASSIGN_TASK_TOOL));
    assert!(is_delegation_tool(REVIEW_TASK_TOOL));
}

/// Issue #884: the teammate hand-off is internal work too. Left out, the
/// approval policy would read it as an external effect and park every
/// hand-off behind an operator approval — and the new edge would sit outside
/// the loop checks every other delegation passes through.
#[test]
fn the_teammate_hand_off_is_an_internal_delegation_tool() {
    assert!(is_delegation_tool(DELEGATE_TO_TEAMMATE_TOOL));
}

/// The orchestrator is actually handed the new tools.
#[test]
fn delegation_tools_include_the_lifecycle_tools() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let names: Vec<String> = delegation_tools(&DelegationQueue::default(), company, store)
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    assert!(names.contains(&ASSIGN_TASK_TOOL.to_string()), "{names:?}");
    assert!(names.contains(&REVIEW_TASK_TOOL.to_string()), "{names:?}");
    // …without dropping the ones that were already there.
    assert!(names.contains(&SPAWN_TASK_TOOL.to_string()), "{names:?}");
    assert!(
        names.contains(&DELEGATE_TO_DESK_TOOL.to_string()),
        "{names:?}"
    );
    // Issue #884: and the teammate hand-off, exactly once — a duplicate name
    // on one belt is what the `else if` in `build` exists to prevent.
    assert_eq!(
        names
            .iter()
            .filter(|n| *n == DELEGATE_TO_TEAMMATE_TOOL)
            .count(),
        1,
        "{names:?}"
    );
}

/// A company with a `strategy` desk led by a roster teammate, an
/// `archive` desk nobody on the roster sits on, and a `writer` teammate who
/// is *not* a desk — the exact shape issue #272 was observed on.
fn desks_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer"]

[[group_chat]]
id = "archive"
name = "Archive desk"
members = ["nobody"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

fn desk_tool(record: CompanyRecord, queue: &DelegationQueue) -> DelegateToDeskTool {
    let company = record.id.clone();
    DelegateToDeskTool::new(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
    )
}

#[tokio::test]
async fn delegate_to_desk_tool_enqueues_a_hand_off() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = desk_tool(desks_record(&CompanyId::new("acme")), &queue);
    let result = tool
        .execute(json!({ "desk": "strategy", "instruction": "draft a plan" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "a real desk with a lead is delegatable");
    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(
        drained,
        vec![Delegation::DelegateToDesk {
            desk: "strategy".to_string(),
            instruction: "draft a plan".to_string(),
        }]
    );
}

// --- Recursive desk delegation (issue #176) -----------------------------

/// A three-desk record where two desks have roster leads, so a member of one
/// can be given an allowlist that admits one desk and not another.
fn nested_desks_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"
delegates_to = ["research"]

[[agent]]
id = "analyst"
role = "Analyst"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer"]

[[group_chat]]
id = "research"
name = "Research desk"
members = ["analyst"]

[[group_chat]]
id = "legal"
name = "Legal desk"
members = ["ceo"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

/// The `writer`'s copy of `delegate_to_desk`: allowed `research` only.
fn member_desk_tool(record: CompanyRecord, queue: &DelegationQueue) -> DelegateToDeskTool {
    let company = record.id.clone();
    DelegateToDeskTool::for_member(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["research".to_string()],
        },
    )
}

/// Depth is the length of the scope chain, and it gates **hand-offs only**.
///
/// At the bound a `delegate_to_desk` is refused with the new
/// [`NoDrainReason::Depth`], while a `spawn_task` still stages — refusing
/// that too would push a member that has hit the bound into working silently
/// rather than leaving the work tracked.
#[test]
fn push_within_cap_refuses_a_hand_off_past_the_depth_bound() {
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let hand_off = || Delegation::DelegateToDesk {
        desk: "research".to_string(),
        instruction: "dig into it".to_string(),
    };
    let card = || Delegation::SpawnTask {
        title: "follow up".to_string(),
        note: None,
        assignee: None,
    };

    // Depth 0 (the orchestrator's own turn) under a bound of 1: allowed.
    assert_eq!(queue.scope_depth(), 0);
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 1),
        Staged::Queued
    );
    queue.clear();

    // One level in, under a bound of 1: refused as depth-capped.
    let scope = queue.enter_scope("strategy".to_string());
    assert_eq!(queue.scope_depth(), 1);
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 1),
        Staged::NoDrain(NoDrainReason::Depth)
    );
    // …while the board write at the same depth is untouched.
    assert_eq!(
        queue.push_within_cap(card(), MAX_DELEGATIONS_PER_TURN, 1),
        Staged::Queued
    );
    queue.clear();
    // …and the same hand-off under the default bound of 2 stages.
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 2),
        Staged::Queued
    );
    queue.clear();

    // Two levels in, under a bound of 2: refused.
    let deeper = queue.enter_scope("research".to_string());
    assert_eq!(queue.scope_depth(), 2);
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 2),
        Staged::NoDrain(NoDrainReason::Depth)
    );

    // The guards pop on drop, outermost last.
    drop(deeper);
    assert_eq!(queue.scope_depth(), 1);
    drop(scope);
    assert_eq!(queue.scope_depth(), 0);
}

/// The refusal has to be countable and distinguishable from the two that
/// preceded it, and its text must not claim either of their causes — the
/// same message would tell a fully capable company that its context cannot
/// do board work.
#[test]
fn the_depth_refusal_is_its_own_reason_and_its_own_sentence() {
    assert_eq!(NoDrainReason::Depth.as_str(), "depth_capped");
    for other in [NoDrainReason::Unwired, NoDrainReason::Triage] {
        assert_ne!(NoDrainReason::Depth.as_str(), other.as_str());
    }
    let text = no_drain(
        DELEGATE_TO_DESK_TOOL,
        "nothing was handed to the research desk",
        NoDrainReason::Depth,
    );
    assert!(text.contains("as far as this company allows"), "{text}");
    assert!(
        text.contains("`spawn_task`"),
        "the model must be told what still works: {text}"
    );
    assert!(
        !text.contains("question"),
        "a depth refusal must not borrow the triage cause: {text}"
    );
    assert!(
        !text.contains("unavailable in this context"),
        "a depth refusal must not borrow the unwired cause: {text}"
    );
}

/// The chain ends with the claim, on **both** boundaries.
///
/// The exit half is the load-bearing one: a `ScopeGuard` pops on every
/// ordinary exit, but a panic inside a nested turn unwinds past it, and a
/// chain left standing would make the next operator message start at depth 2
/// and refuse its first hand-off. An ordinary `clear()` must NOT reset it —
/// clearing happens between delegations inside a live chain.
#[test]
fn the_scope_chain_resets_with_the_claim_and_survives_a_clear() {
    let queue = DelegationQueue::default();
    {
        let _claim = queue.claim();
        std::mem::forget(queue.enter_scope("strategy".to_string()));
        std::mem::forget(queue.enter_scope("research".to_string()));
        assert_eq!(queue.scope_chain(), ["strategy", "research"]);
        queue.clear();
        assert_eq!(
            queue.scope_chain(),
            ["strategy", "research"],
            "clear() runs between delegations inside a live chain and must not reset depth"
        );
    }
    assert_eq!(
        queue.scope_depth(),
        0,
        "the claim's Drop must reset a chain leaked past its guards"
    );
    // …and the acquire resets too, for a claim taken after a leak.
    std::mem::forget(queue.enter_scope("strategy".to_string()));
    let _claim = queue.claim();
    assert_eq!(queue.scope_depth(), 0);
}

/// A member may not hand work back up its own chain (A→B→A), and the
/// refusal is recorded for the card as well as returned to the model.
#[tokio::test]
async fn a_member_may_not_hand_work_back_to_a_desk_on_the_chain() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    // The chain the orchestrator's hand-off to `strategy` opened, with the
    // writer's own turn running inside it.
    let _scope = queue.enter_scope("strategy".to_string());
    // `writer` leads `strategy`, so it is BOTH on the chain and self-led;
    // give it a wildcard allowlist so the allowlist check cannot be what
    // refuses.
    let tool = DelegateToDeskTool::for_member(
        queue.clone(),
        company.clone(),
        Arc::new(MemStore::seeded(nested_desks_record(&company))) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["*".to_string()],
        },
    );
    let result = tool
        .execute(json!({ "desk": "strategy", "instruction": "start over" }))
        .await
        .expect("execute");
    assert!(result.is_error, "a cycle must be refused");
    let text = result.output_for_llm(true);
    assert!(text.contains("strategy"), "{text}");
    assert_eq!(queue.queued(), 0, "nothing may be staged for a cycle");
    assert_eq!(
        queue.drain_refusals(MAX_DELEGATIONS_PER_TURN),
        vec!["strategy".to_string()],
        "the drain must be able to record the attempt on the card"
    );

    // A desk that is neither on the chain nor led by the caller goes
    // through.
    let ok = tool
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(!ok.is_error, "{}", ok.output_for_llm(true));
}

/// A member may only reach the desks its manifest entry names, and the
/// refusal lists them — the model has no other way to learn its allowlist.
#[tokio::test]
async fn a_member_may_only_reach_the_desks_its_manifest_allows() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = member_desk_tool(nested_desks_record(&company), &queue);

    let refused = tool
        .execute(json!({ "desk": "legal", "instruction": "review it" }))
        .await
        .expect("execute");
    assert!(refused.is_error, "an off-allowlist desk must be refused");
    let text = refused.output_for_llm(true);
    assert!(text.contains("legal"), "{text}");
    assert!(
        text.contains("research"),
        "the permitted set must be named so the model can retry in-turn: {text}"
    );
    assert_eq!(queue.queued(), 0);

    let allowed = tool
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(!allowed.is_error, "{}", allowed.output_for_llm(true));
    assert_eq!(queue.queued(), 1);
}

/// The **orchestrator's** copy is unrestricted: no allowlist, no cycle
/// guard, and it reaches every desk exactly as it did before #176.
#[tokio::test]
async fn the_orchestrators_copy_is_unrestricted() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    // Even from inside a chain — which the orchestrator never is, but the
    // contrast is the point.
    let _scope = queue.enter_scope("legal".to_string());
    let tool = desk_tool(nested_desks_record(&company), &queue);
    for desk in ["strategy", "research", "legal"] {
        let result = tool
            .execute(json!({ "desk": desk, "instruction": "go" }))
            .await
            .expect("execute");
        assert!(
            !result.is_error,
            "the orchestrator must reach {desk}: {}",
            result.output_for_llm(true)
        );
        queue.clear();
    }
}

/// A store that cannot answer, so the grounding read has nothing to check
/// the target against.
struct BrokenStore;

#[async_trait::async_trait]
impl CompanyStore for BrokenStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Err(crate::OpenCompanyError::Store("store is down".to_string()))
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

/// A member's hand-off fails **closed** when the record cannot be read, and
/// the orchestrator's still fails open.
///
/// The asymmetry is the whole point. The allowlist and the cycle guard are
/// enforced at this tool boundary and nowhere else — `run_delegation`
/// executes whatever the queue holds without re-deriving either — so a
/// member queued ungrounded reaches every desk in the company for as long
/// as the store is unhappy. The orchestrator has no allowlist to lose, so
/// an unreadable record leaves it exactly where #272 left it.
#[tokio::test]
async fn a_members_hand_off_is_refused_when_the_record_cannot_be_read() {
    let company = CompanyId::new("acme");
    let scope = || MemberScope {
        member: "writer".to_string(),
        delegates_to: vec!["research".to_string()],
    };

    // Ok(None) — no record under that id.
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let missing = DelegateToDeskTool::for_member(
        queue.clone(),
        company.clone(),
        Arc::new(MemStore::default()) as Arc<dyn CompanyStore>,
        scope(),
    );
    let refused = missing
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(
        refused.is_error,
        "a member may not be queued against a record nobody could read: {}",
        refused.output_for_llm(true)
    );
    let text = refused.output_for_llm(true);
    assert!(text.contains("research"), "{text}");
    assert!(
        text.contains("writer"),
        "the refusal must name whose allowlist went unchecked: {text}"
    );
    assert_eq!(queue.queued(), 0, "nothing may be staged ungrounded");
    assert_eq!(
        queue.drain_refusals(MAX_DELEGATIONS_PER_TURN),
        vec!["research".to_string()],
        "the drain must be able to record the attempt on the card"
    );

    // Err(..) — the store is there and unhappy. Same answer.
    let broken = DelegateToDeskTool::for_member(
        queue.clone(),
        company.clone(),
        Arc::new(BrokenStore) as Arc<dyn CompanyStore>,
        scope(),
    );
    let refused = broken
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(
        refused.is_error,
        "a store error must refuse too: {}",
        refused.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 0);
    queue.clear();
    let _ = queue.drain_refusals(MAX_DELEGATIONS_PER_TURN);

    // …and the orchestrator's copy over the same broken store still queues.
    let orchestrator = DelegateToDeskTool::new(
        queue.clone(),
        company,
        Arc::new(BrokenStore) as Arc<dyn CompanyStore>,
    );
    let queued = orchestrator
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(
        !queued.is_error,
        "a store hiccup must not take the orchestrator's delegation offline: {}",
        queued.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 1);
}

/// The depth bound comes off the **live company record**, not a build-time
/// snapshot — an operator can edit `[tools].max_delegation_depth` without
/// the cached belt being rebuilt.
#[tokio::test]
async fn the_depth_bound_is_read_from_the_manifest_at_call_time() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let mut record = nested_desks_record(&company);
    record.manifest.tools.max_delegation_depth = Some(1);
    let tool = member_desk_tool(record, &queue);
    // One level in, under the manifest's bound of 1.
    let _scope = queue.enter_scope("strategy".to_string());
    let result = tool
        .execute(json!({ "desk": "research", "instruction": "dig into it" }))
        .await
        .expect("execute");
    assert!(result.is_error, "depth 1 must stop a member re-delegating");
    assert!(
        result
            .output_for_llm(true)
            .contains("as far as this company allows"),
        "{}",
        result.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 0);
}

/// The member's belt is exactly `spawn_task` + the two hand-off tools —
/// never the orchestrator's authority tools.
#[test]
fn a_members_delegation_belt_is_the_two_hand_off_tools() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(nested_desks_record(&company)));
    let tools = member_delegation_tools(
        &queue,
        company,
        store,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["research".to_string()],
        },
    );
    let mut names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            DELEGATE_TO_DESK_TOOL,
            DELEGATE_TO_TEAMMATE_TOOL,
            SPAWN_TASK_TOOL
        ]
    );
}

// ── delegate_to_teammate at the tool boundary (issue #884) ──────────────

/// A company whose `strategy` desk has THREE members, so its lead has peers
/// to reach — the shape D1 was observed on — plus an `analyst` on a desk the
/// lead's `delegates_to` permits and a `legal_counsel` on one it does not.
fn peers_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"
delegates_to = ["research"]

[[agent]]
id = "editor"
role = "Editor"

[[agent]]
id = "analyst"
role = "Analyst"

[[agent]]
id = "legal_counsel"
role = "Counsel"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer", "editor"]

[[group_chat]]
id = "research"
name = "Research desk"
members = ["analyst"]

[[group_chat]]
id = "legal"
name = "Legal desk"
members = ["legal_counsel"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

/// `writer`'s copy of the teammate tool: a desk lead with one peer on its
/// own desk and a `research` allowlist.
fn member_teammate_tool(
    record: CompanyRecord,
    queue: &DelegationQueue,
) -> DelegateToTeammateTool {
    let company = record.id.clone();
    DelegateToTeammateTool::for_member(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["research".to_string()],
        },
    )
}

/// D1 at the boundary: the lead's hand-off to the peer beside it is
/// **accepted**, and queues the delegation the drain runs that teammate's
/// turn from.
#[tokio::test]
async fn a_lead_may_hand_work_to_a_peer_on_its_own_desk() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = member_teammate_tool(peers_record(&company), &queue);
    let result = tool
        .execute(json!({ "teammate": "editor", "instruction": "tighten the copy" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.output_for_llm(true));
    assert_eq!(
        queue.drain(MAX_DELEGATIONS_PER_TURN),
        vec![Delegation::DelegateToTeammate {
            teammate: "editor".to_string(),
            instruction: "tighten the copy".to_string(),
        }]
    );
}

/// A key that is nobody is refused before anything is queued, and the
/// attempt is recorded for the drain to report on the card — the same
/// independence #272 gave the desk refusals.
#[tokio::test]
async fn a_teammate_that_is_not_on_the_roster_is_refused_and_recorded() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = member_teammate_tool(peers_record(&company), &queue);
    let result = tool
        .execute(json!({ "teammate": "ghost", "instruction": "do it" }))
        .await
        .expect("execute");
    assert!(result.is_error, "{}", result.output_for_llm(true));
    assert!(
        result.output_for_llm(true).contains("editor"),
        "the refusal must name who CAN be reached: {}",
        result.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 0);
    assert_eq!(
        queue.drain_refusals(MAX_DELEGATIONS_PER_TURN),
        vec!["ghost".to_string()]
    );
}

/// A real teammate on neither the caller's desk nor an allowlisted one is
/// refused; one on an allowlisted desk is not. The allowlist is #176's, read
/// at teammate granularity rather than duplicated.
#[tokio::test]
async fn the_allowlist_bounds_which_teammates_a_member_may_reach() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = member_teammate_tool(peers_record(&company), &queue);

    let refused = tool
        .execute(json!({ "teammate": "legal_counsel", "instruction": "review it" }))
        .await
        .expect("execute");
    assert!(refused.is_error, "{}", refused.output_for_llm(true));
    assert_eq!(queue.queued(), 0);

    // `analyst` sits on `research`, which `writer`'s `delegates_to` names.
    let allowed = tool
        .execute(json!({ "teammate": "analyst", "instruction": "pull the numbers" }))
        .await
        .expect("execute");
    assert!(!allowed.is_error, "{}", allowed.output_for_llm(true));
    assert_eq!(queue.queued(), 1);
}

/// A hand-off back to somebody already on the chain is refused as a cycle —
/// the A→B→A guard, at the boundary, in the model's own turn.
#[tokio::test]
async fn a_hand_off_back_up_the_teammate_chain_is_refused() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let record = peers_record(&company);
    let tool = DelegateToTeammateTool::for_member(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "editor".to_string(),
            delegates_to: Vec::new(),
        },
    );
    // `editor` is running inside a hand-off `writer` made.
    let _scope = queue.enter_scope(crate::runtime::delegation_tools::teammate_scope_key(
        "writer",
    ));
    let result = tool
        .execute(json!({ "teammate": "writer", "instruction": "you take it back" }))
        .await
        .expect("execute");
    assert!(result.is_error, "{}", result.output_for_llm(true));
    assert!(
        result.output_for_llm(true).contains("loop"),
        "{}",
        result.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 0);
}

/// The depth bound applies to the teammate hand-off exactly as it does to
/// the desk one — the guard a ring of three the cycle check cannot see still
/// runs into.
#[tokio::test]
async fn the_depth_bound_stops_a_teammate_hand_off_too() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let mut record = peers_record(&company);
    record.manifest.tools.max_delegation_depth = Some(1);
    let tool = member_teammate_tool(record, &queue);
    let _scope = queue.enter_scope("strategy".to_string());
    let result = tool
        .execute(json!({ "teammate": "editor", "instruction": "tighten the copy" }))
        .await
        .expect("execute");
    assert!(result.is_error, "depth 1 must stop a further hand-off");
    assert!(
        result
            .output_for_llm(true)
            .contains("as far as this company allows"),
        "{}",
        result.output_for_llm(true)
    );
    assert_eq!(queue.queued(), 0);
}

/// The orchestrator's copy is unrestricted: it reaches a teammate that is
/// not a desk lead, with no allowlist in the way. Grounding still applies.
#[tokio::test]
async fn the_orchestrators_teammate_tool_is_unrestricted_but_grounded() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let record = peers_record(&company);
    let store = Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>;
    let tool = DelegateToTeammateTool::new(queue.clone(), company, store);

    let ok = tool
        .execute(json!({ "teammate": "editor", "instruction": "tighten the copy" }))
        .await
        .expect("execute");
    assert!(!ok.is_error, "{}", ok.output_for_llm(true));

    let refused = tool
        .execute(json!({ "teammate": "ghost", "instruction": "do it" }))
        .await
        .expect("execute");
    assert!(refused.is_error, "{}", refused.output_for_llm(true));
}

/// Issue #1162, the other half of the fix: a hand-off written with a
/// teammate's **display name** is accepted, and what reaches the queue is
/// the **canonical id**.
///
/// Queueing the key as typed is what would make a name-accepting refusal
/// worse than the refusal it replaced — the tool would answer "Handed to
/// …" and the drain, which resolves independently, would find nothing to
/// deliver to. The reply names both strings so the model learns the id.
#[tokio::test]
async fn a_teammate_named_by_display_name_is_queued_under_its_id() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let mut record = peers_record(&company);
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "dana_designer".to_string(),
        name: "Dana Designer".to_string(),
        role: "Designer".to_string(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    let store = Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>;
    let tool = DelegateToTeammateTool::new(queue.clone(), company, store);

    let result = tool
        .execute(json!({ "teammate": "Dana Designer", "instruction": "draw it" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.output_for_llm(true));
    let reply = result.output_for_llm(true);
    assert!(
        reply.contains("Dana Designer") && reply.contains("dana_designer"),
        "the reply must name the person and teach the id: {reply}"
    );
    assert_eq!(
        queue.drain(MAX_DELEGATIONS_PER_TURN),
        vec![Delegation::DelegateToTeammate {
            teammate: "dana_designer".to_string(),
            instruction: "draw it".to_string(),
        }],
        "the queue must carry the canonical id, not the key as typed"
    );
}

/// A display name two teammates answer to is refused rather than routed to
/// whichever was added first, and the refusal carries the ids to retry
/// with — the collision is the operator's to resolve, and the model cannot
/// do it without being told the alternatives (issue #1162).
#[tokio::test]
async fn a_display_name_two_teammates_share_is_refused_with_their_ids() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let mut record = peers_record(&company);
    for id in ["dana_designer", "dana_designer_2"] {
        record.overlay_agents.push(OverlayAgent {
            provider: None,
            id: id.to_string(),
            name: "Dana Designer".to_string(),
            role: "Designer".to_string(),
            description: None,
            tools: None,
            model: None,
            harness: None,
        });
    }
    let store = Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>;
    let tool = DelegateToTeammateTool::new(queue.clone(), company, store);

    let result = tool
        .execute(json!({ "teammate": "Dana Designer", "instruction": "draw it" }))
        .await
        .expect("execute");
    assert!(result.is_error, "{}", result.output_for_llm(true));
    let refusal = result.output_for_llm(true);
    assert!(
        refusal.contains("dana_designer") && refusal.contains("dana_designer_2"),
        "the refusal must name both ids: {refusal}"
    );
    assert_eq!(queue.queued(), 0);
}

/// Both arguments are required, and neither may be blank — a hand-off with
/// no instruction is a turn run on nothing.
#[tokio::test]
async fn the_teammate_tool_requires_both_arguments() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = member_teammate_tool(peers_record(&company), &queue);
    assert!(tool.execute(json!({ "teammate": "editor" })).await.is_err());
    assert!(
        tool.execute(json!({ "instruction": "do it" }))
            .await
            .is_err()
    );
    assert!(
        tool.execute(json!({ "teammate": "  ", "instruction": "do it" }))
            .await
            .is_err()
    );
    assert_eq!(queue.queued(), 0);
}

/// Issue #272: the observed failure — the orchestrator handed work to
/// `writer`, which is a teammate rather than a desk. Nothing may be queued,
/// and the refusal must carry the real desk ids so the model can correct
/// itself in the same turn.
#[tokio::test]
async fn delegate_to_desk_tool_refuses_a_desk_that_does_not_exist() {
    let queue = DelegationQueue::default();
    let tool = desk_tool(desks_record(&CompanyId::new("acme")), &queue);
    let result = tool
        .execute(json!({ "desk": "writer", "instruction": "draft the release note" }))
        .await
        .expect("execute");
    assert!(result.is_error, "an invented desk must be refused");
    let text = result.output_for_llm(true);
    assert!(text.contains("strategy"), "valid ids must be named: {text}");
    assert!(
        text.contains("teammate"),
        "a teammate-as-desk target must be named as such: {text}"
    );
    assert_eq!(
        queue.queued(),
        0,
        "a refused target must not survive as a queued hand-off"
    );
    assert_eq!(
        queue.drain_refusals(MAX_DELEGATIONS_PER_TURN),
        vec!["writer".to_string()],
        "the drain must be able to report the attempt on the card"
    );
}

/// A desk that exists but has nobody on the roster can never run a turn, so
/// the hand-off is refused rather than queued into a drain that cannot
/// deliver it.
#[tokio::test]
async fn delegate_to_desk_tool_refuses_a_desk_with_no_roster_lead() {
    let queue = DelegationQueue::default();
    let tool = desk_tool(desks_record(&CompanyId::new("acme")), &queue);
    let result = tool
        .execute(json!({ "desk": "archive", "instruction": "file it" }))
        .await
        .expect("execute");
    assert!(result.is_error, "a leadless desk must be refused");
    let text = result.output_for_llm(true);
    assert!(
        text.contains("no member on the roster"),
        "the refusal must name the cause: {text}"
    );
    assert!(
        text.contains("strategy"),
        "a desk that CAN take work must be offered: {text}"
    );
    assert_eq!(queue.queued(), 0);
}

/// Fail-open: with no record to read, delegation behaves exactly as it did
/// before grounding existed. A store gap must not take delegation offline.
#[tokio::test]
async fn delegate_to_desk_tool_queues_ungrounded_when_no_record_is_readable() {
    let queue = DelegationQueue::default();
    // Claimed (issue #453): "fail open" is about the *desk grounding*, and
    // this pins that an unreadable record still queues. Whether anything
    // drains is a separate question with its own refusal.
    let _claim = queue.claim();
    let tool = DelegateToDeskTool::new(
        queue.clone(),
        CompanyId::new("acme"),
        Arc::new(MemStore::default()) as Arc<dyn CompanyStore>,
    );
    let result = tool
        .execute(json!({ "desk": "whatever", "instruction": "do it" }))
        .await
        .expect("execute");
    assert!(!result.is_error);
    assert_eq!(queue.queued(), 1);
}

/// **Issue #348 review.** The recent-activity tail is ten slots wide, and a
/// discussion (#335) is an operator-driven writer into the same journal the
/// tail reads. A row per post would let one afternoon's thread on one card
/// push every dispatch, reply and approval out of the orchestrator's only
/// view of what the company has been doing — and replace them with rows it
/// cannot act on, since no agent participates in a discussion.
///
/// So: posts never hold a slot, the run events survive a thread that
/// outnumbers them, and the fact that people are talking is still reported —
/// as one folded count, with no message text (the same no-quoting rule
/// `summarize_event`'s arm carries).
#[tokio::test]
async fn discussion_posts_fold_to_one_line_instead_of_evicting_the_activity_tail() {
    use crate::ports::types::StoredEvent;
    use futures::stream::{self, BoxStream};

    /// A log that replays a fixed history.
    struct FixedLog(Vec<StoredEvent>);

    #[async_trait]
    impl EventLog for FixedLog {
        async fn append(
            &self,
            _id: &CompanyId,
            _event: CompanyEvent,
        ) -> crate::Result<EventSeq> {
            unreachable!("the insight surface only reads")
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            seq: EventSeq,
            limit: usize,
        ) -> crate::Result<Vec<StoredEvent>> {
            Ok(self
                .0
                .iter()
                .filter(|e| e.seq.value() >= seq.value())
                .take(limit)
                .cloned()
                .collect())
        }
        fn subscribe(
            &self,
            _id: &CompanyId,
        ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    let company = CompanyId::new("acme");
    let mut history = vec![StoredEvent {
        seq: EventSeq::new(0),
        company: company.clone(),
        event: CompanyEvent::TaskDispatched {
            task_id: "t-1".to_string(),
            run_id: None,
        },
        at_millis: 1,
    }];
    // Twenty posts — twice the tail — on the one card, as an afternoon of
    // back-and-forth actually looks.
    for n in 0..20u64 {
        history.push(StoredEvent {
            seq: EventSeq::new(n + 1),
            company: company.clone(),
            event: CompanyEvent::TaskDiscussionPosted {
                task_id: "t-1".to_string(),
                text: format!("ping the vendor again ({n})"),
                by: None,
            },
            at_millis: 2 + n,
        });
    }
    history.push(StoredEvent {
        seq: EventSeq::new(21),
        company: company.clone(),
        event: CompanyEvent::DeskTaskCompleted {
            task_id: "t-1".to_string(),
            desk: "eng".to_string(),
            output: "shipped".to_string(),
            column: "done".to_string(),
            artifact_ids: Vec::new(),
            origin_chat_id: None,
            origin_parent: None,
        },
        at_millis: 30,
    });

    let log: Arc<dyn EventLog> = Arc::new(FixedLog(history));
    let tool = QueryCompanyTool::new(company, None, Some(log), None, None, None);
    let out = tool
        .execute(json!({}))
        .await
        .expect("execute")
        .output_for_llm(true);

    // Both run events survive the thread that buried them.
    assert!(out.contains("task dispatched"), "dispatch evicted: {out}");
    assert!(out.contains("task completed"), "completion evicted: {out}");
    // One folded line, not twenty rows — and no message text anywhere.
    assert!(out.contains("20 discussion posts"), "{out}");
    assert!(!out.contains("ping the vendor"), "post text quoted: {out}");
    assert_eq!(
        out.matches("discussion post").count(),
        1,
        "a post must not hold a slot of its own: {out}"
    );
}

/// Issue #420: the recent-activity tail keeps only [`RECENT_EVENTS`] rows,
/// and it used to drop everything older in silence — a full log read as
/// complete, the same silent-cut class the facts section one block down
/// already announces. The tail now names how many rows fell off the far end
/// (in the markdown) and reports the count (in the JSON summary). Discussion
/// posts pushed past the tail are dropped rows too, so they count toward it
/// rather than folding into their own line.
#[tokio::test]
async fn query_company_announces_the_dropped_event_tail() {
    use crate::ports::types::StoredEvent;
    use futures::stream::{self, BoxStream};

    /// A log that replays a fixed history.
    struct FixedLog(Vec<StoredEvent>);

    #[async_trait]
    impl EventLog for FixedLog {
        async fn append(
            &self,
            _id: &CompanyId,
            _event: CompanyEvent,
        ) -> crate::Result<EventSeq> {
            unreachable!("the insight surface only reads")
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            seq: EventSeq,
            limit: usize,
        ) -> crate::Result<Vec<StoredEvent>> {
            Ok(self
                .0
                .iter()
                .filter(|e| e.seq.value() >= seq.value())
                .take(limit)
                .cloned()
                .collect())
        }
        fn subscribe(
            &self,
            _id: &CompanyId,
        ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    let company = CompanyId::new("acme");

    // A distinct, non-discussion event so every row occupies a tail slot.
    let dispatch = |seq: u64| StoredEvent {
        seq: EventSeq::new(seq),
        company: company.clone(),
        event: CompanyEvent::TaskDispatched {
            task_id: format!("t-{seq}"),
            run_id: None,
        },
        at_millis: seq + 1,
    };

    // (a) Five more row-events than the tail is wide: the five oldest fall
    // off, the notice sits at the top, and the JSON summary counts them.
    let over: Vec<StoredEvent> = (0..(RECENT_EVENTS as u64 + 5)).map(dispatch).collect();
    let log: Arc<dyn EventLog> = Arc::new(FixedLog(over));
    let tool = QueryCompanyTool::new(company.clone(), None, Some(log), None, None, None);
    let result = tool.execute(json!({})).await.expect("execute");
    let md = result.output_for_llm(true);
    let activity = md
        .split("## Recent activity\n")
        .nth(1)
        .expect("recent activity section");
    assert!(
        activity.starts_with("- […5 earlier event(s) not shown]"),
        "the dropped tail must be announced at the top: {md}"
    );
    assert!(
        result
            .output_for_llm(false)
            .contains("\"events_not_shown\": 5"),
        "the JSON summary must count the drop: {}",
        result.output_for_llm(false)
    );

    // (b) Exactly the tail width: nothing was dropped, so nothing is said.
    let exact: Vec<StoredEvent> = (0..RECENT_EVENTS as u64).map(dispatch).collect();
    let log: Arc<dyn EventLog> = Arc::new(FixedLog(exact));
    let result = QueryCompanyTool::new(company.clone(), None, Some(log), None, None, None)
        .execute(json!({}))
        .await
        .expect("execute");
    assert!(
        !result
            .output_for_llm(true)
            .contains("earlier event(s) not shown"),
        "a complete tail must stay silent: {}",
        result.output_for_llm(true)
    );
    assert!(
        result
            .output_for_llm(false)
            .contains("\"events_not_shown\": 0"),
        "a complete tail reports zero dropped: {}",
        result.output_for_llm(false)
    );

    // (c) Discussion posts older than the tail are dropped rows: they count
    // toward the drop, not toward the fold line. Three posts (oldest) then
    // enough dispatches to fill the tail — the posts never get visited.
    let mut mixed: Vec<StoredEvent> = Vec::new();
    for seq in 0..3u64 {
        mixed.push(StoredEvent {
            seq: EventSeq::new(seq),
            company: company.clone(),
            event: CompanyEvent::TaskDiscussionPosted {
                task_id: "t-1".to_string(),
                text: format!("older chatter {seq}"),
                by: None,
            },
            at_millis: seq + 1,
        });
    }
    for seq in 3..(RECENT_EVENTS as u64 + 5) {
        mixed.push(dispatch(seq));
    }
    // total = 3 posts + (RECENT_EVENTS + 2) dispatches; the tail holds
    // RECENT_EVENTS dispatches, so 2 dispatches + 3 posts = 5 fall off.
    let log: Arc<dyn EventLog> = Arc::new(FixedLog(mixed));
    let result = QueryCompanyTool::new(company.clone(), None, Some(log), None, None, None)
        .execute(json!({}))
        .await
        .expect("execute");
    let md = result.output_for_llm(true);
    assert!(
        md.contains("- […5 earlier event(s) not shown]"),
        "dropped discussion posts must count toward the tail drop: {md}"
    );
    assert!(
        !md.contains("discussion post"),
        "an unvisited post must not also fold into its own line: {md}"
    );
    assert!(
        result
            .output_for_llm(false)
            .contains("\"events_not_shown\": 5"),
        "{}",
        result.output_for_llm(false)
    );
}

/// Issue #410, point 4 (audit the same silent-cut class elsewhere): the
/// fact list is capped at [`FACT_LIMIT`], and it used to be capped in
/// silence. A company past twenty facts handed the orchestrator a partial
/// memory that read as complete, so "we have no record of that" was a
/// conclusion it could reach from a truncated list. The cut now says it
/// happened and names the argument that narrows it.
#[tokio::test]
async fn query_company_says_when_the_fact_list_was_cut() {
    use crate::ports::FactStore;
    use crate::ports::facts::{FactKind, FactRecord};

    struct ManyFacts(usize);
    #[async_trait]
    impl FactStore for ManyFacts {
        async fn list(
            &self,
            _company: &CompanyId,
            _query: Option<&str>,
            _kind: Option<FactKind>,
        ) -> crate::Result<Vec<FactRecord>> {
            Ok((0..self.0)
                .map(|i| FactRecord {
                    id: format!("f-{i}"),
                    kind: FactKind::Fact,
                    title: format!("Fact {i}"),
                    body: format!("Body {i}"),
                    source: "ceo".to_string(),
                    updated_at_millis: i as u64,
                })
                .collect())
        }
        async fn upsert(&self, _c: &CompanyId, _f: &FactRecord) -> crate::Result<()> {
            Ok(())
        }
        async fn delete(&self, _c: &CompanyId, _id: &str) -> crate::Result<bool> {
            Ok(false)
        }
    }

    // Exactly at the cap: complete, so no notice.
    let exact: Arc<dyn FactStore> = Arc::new(ManyFacts(FACT_LIMIT));
    let out =
        QueryCompanyTool::new(CompanyId::new("acme"), Some(exact), None, None, None, None)
            .execute(json!({}))
            .await
            .expect("execute")
            .output_for_llm(true);
    assert!(!out.contains("TRUNCATED"), "nothing was cut: {out}");

    // Past the cap: the cut is announced, counted, and points at `query`.
    let many: Arc<dyn FactStore> = Arc::new(ManyFacts(FACT_LIMIT + 7));
    let out = QueryCompanyTool::new(CompanyId::new("acme"), Some(many), None, None, None, None)
        .execute(json!({}))
        .await
        .expect("execute")
        .output_for_llm(true);
    assert!(
        out.contains("TRUNCATED"),
        "the cut must be announced: {out}"
    );
    assert!(out.contains("7 more fact(s) not shown"), "{out}");
    assert!(out.contains("query_company"), "{out}");
}

/// Issue #420, the residual: the whole insight document is handed to the
/// model through the harness tool-result path, which hard-cuts anything past
/// its byte budget — blindly. A facts list long enough would carry that cut
/// into the sections below it, dropping the facts `[TRUNCATED]` marker and
/// the Desks list `delegate_to_desk` reads. So each fact body is capped and
/// the facts section is bounded in bytes; the marker and every later section
/// stay inside the outer budget. Cutting a body counts characters, never
/// bytes, so a multibyte body cannot panic mid-codepoint.
#[tokio::test]
async fn query_company_bounds_the_insight_document_size() {
    use crate::ports::FactStore;
    use crate::ports::facts::{FactKind, FactRecord};

    struct Facts(Vec<FactRecord>);
    #[async_trait]
    impl FactStore for Facts {
        async fn list(
            &self,
            _c: &CompanyId,
            _q: Option<&str>,
            _k: Option<FactKind>,
        ) -> crate::Result<Vec<FactRecord>> {
            Ok(self.0.clone())
        }
        async fn upsert(&self, _c: &CompanyId, _f: &FactRecord) -> crate::Result<()> {
            Ok(())
        }
        async fn delete(&self, _c: &CompanyId, _id: &str) -> crate::Result<bool> {
            Ok(false)
        }
    }

    let mk = |i: usize, body: String| FactRecord {
        id: format!("f-{i}"),
        kind: FactKind::Fact,
        title: format!("Fact {i}"),
        body,
        source: "ceo".to_string(),
        updated_at_millis: i as u64,
    };
    let render = |facts: Vec<FactRecord>| async move {
        let store: Arc<dyn FactStore> = Arc::new(Facts(facts));
        QueryCompanyTool::new(CompanyId::new("acme"), Some(store), None, None, None, None)
            .execute(json!({}))
            .await
            .expect("execute")
            .output_for_llm(true)
    };

    // (e) A single multi-KB multibyte body: cut on a char boundary, marked
    // with an ellipsis, exactly the cap wide, and no panic.
    let out = render(vec![mk(0, "é".repeat(5_000))]).await;
    let line = out
        .lines()
        .find(|l| l.starts_with("- **Fact 0**: "))
        .expect("fact line");
    let body = line.strip_prefix("- **Fact 0**: ").unwrap();
    assert!(body.ends_with('…'), "a cut body is marked: {body:?}");
    assert_eq!(
        body.chars().count(),
        MAX_FACT_BODY_CHARS,
        "the body is cut to exactly the cap"
    );
    assert!(
        body.chars().take(MAX_FACT_BODY_CHARS - 1).all(|c| c == 'é'),
        "the cut landed on a codepoint boundary, not inside one"
    );

    // (f) Enough capped bodies to blow the section byte budget. The count
    // reflects the budget cut, not merely FACT_LIMIT, and the marker plus
    // every section below Facts survives the outer tool-result cut.
    let heavy: Vec<FactRecord> = (0..FACT_LIMIT)
        .map(|i| mk(i, "é".repeat(MAX_FACT_BODY_CHARS)))
        .collect();
    let out = render(heavy).await;
    let shown = out.matches("- **Fact ").count();
    assert!(
        (1..FACT_LIMIT).contains(&shown),
        "the byte budget must cut before FACT_LIMIT yet keep at least one: shown={shown}"
    );
    assert!(
        out.contains(&format!("{} more fact(s) not shown", FACT_LIMIT - shown)),
        "the marker counts the budget cut: {out}"
    );
    for header in [
        "[TRUNCATED",
        "## Recent activity",
        "## Saved workflows",
        "## Team",
        "## Desks",
    ] {
        assert!(
            out.contains(header),
            "the facts cut must not carry the outer budget into `{header}`: {out}"
        );
    }

    // (g) A small document is byte-for-byte the pre-guard behavior: bodies
    // under the cap render verbatim and nothing is announced.
    let out = render(vec![
        mk(0, "Body 0".to_string()),
        mk(1, "Body 1".to_string()),
    ])
    .await;
    assert!(
        out.contains("## Facts\n- **Fact 0**: Body 0\n- **Fact 1**: Body 1\n"),
        "the small-document path is unchanged: {out}"
    );
    assert!(!out.contains("TRUNCATED"), "nothing was cut: {out}");
    assert!(!out.contains('…'), "nothing was truncated: {out}");
}

#[tokio::test]
async fn query_company_tool_reports_no_data_when_unwired() {
    let tool = QueryCompanyTool::new(CompanyId::new("acme"), None, None, None, None, None);
    let result = tool.execute(json!({})).await.expect("execute");
    // The insight surface lives in the markdown; `output()` is the summary.
    let out = result.output_for_llm(true);
    assert!(out.contains("No durable facts recorded"), "{out}");
    assert!(out.contains("No recent activity"), "{out}");
    // Not "no saved workflows" any more: the global baseline ships graphs
    // every company has, wired store or not.
    for workflow in crate::globals::workflows() {
        assert!(out.contains(&workflow.id), "{out}");
    }
}

/// Regression: a saved workflow (on disk) and an operator-added overlay
/// teammate both show up in `query_company`. Before this the orchestrator
/// had no way to enumerate either, so a freshly created workflow / added
/// teammate looked unpersisted when the operator asked about it.
#[tokio::test]
async fn query_company_tool_lists_saved_workflows_and_roster() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("workflows")).unwrap();
    std::fs::write(
        dir.path().join("workflows").join("daily-standup.toml"),
        r#"
id = "daily-standup"
name = "Daily Standup"
description = "Morning summary."
[[node]]
id = "start"
kind = "trigger"
name = "Morning"
"#,
    )
    .unwrap();

    // A record whose overlay adds a teammate — the `add_agent`
    // persistence shape.
    let mut record = seeded_record(&CompanyId::new("acme"));
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "fact-fetcher".to_string(),
        name: "Fact Fetcher".to_string(),
        role: "Researcher".to_string(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::seeded(record));

    let tool = QueryCompanyTool::new(
        CompanyId::new("acme"),
        None,
        None,
        Some(dir.path().to_path_buf()),
        Some(store),
        None,
    );
    let out = tool
        .execute(json!({}))
        .await
        .expect("execute")
        .output_for_llm(true);

    assert!(out.contains("Daily Standup"), "workflow missing: {out}");
    assert!(out.contains("daily-standup"), "workflow id missing: {out}");
    assert!(
        out.contains("Fact Fetcher"),
        "overlay teammate name missing: {out}"
    );
}

/// Issue #1162: the Team column is the one the orchestrator is told to take
/// a hand-off target from, so every row must lead with a token the
/// delegation tools can ground. An overlay teammate was listed under its
/// **display name** while a manifest agent was listed under its **id** —
/// two namespaces rendered identically, and `mint_agent_id` guarantees the
/// name is not the id.
///
/// The two halves are pinned together deliberately: the assertion is not
/// "the line contains `dana_designer`" but "the token the line prints
/// resolves", so a render that drifts from the resolver fails here rather
/// than in production.
#[tokio::test]
async fn query_company_lists_a_teammate_under_the_id_delegation_grounds() {
    let mut record = seeded_record(&CompanyId::new("acme"));
    let id = record.mint_agent_id("Dana Designer");
    assert_eq!(id, "dana_designer", "the id and the name must differ");
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: id.clone(),
        name: "Dana Designer".to_string(),
        role: "Designer".to_string(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    });
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::seeded(record.clone()));

    let out =
        QueryCompanyTool::new(CompanyId::new("acme"), None, None, None, Some(store), None)
            .execute(json!({}))
            .await
            .expect("execute")
            .output_for_llm(true);

    let line = out
        .lines()
        .find(|line| line.contains("Designer"))
        .unwrap_or_else(|| panic!("no teammate line: {out}"));
    assert!(
        line.contains(&id),
        "the row must lead with the groundable id: {line}"
    );
    assert!(
        line.contains("known as Dana Designer"),
        "the display name must survive as a label: {line}"
    );
    // The token the roster prints is the token delegation accepts.
    let printed = line
        .split("**")
        .nth(1)
        .unwrap_or_else(|| panic!("no bold token: {line}"));
    assert_eq!(
        record.resolve_teammate_key(printed),
        crate::ports::types::TeammateResolution::Agent(id),
        "the roster printed a token delegation cannot ground: {line}"
    );
}

/// Issue #272: `query_company` is the grounding surface the orchestrator is
/// told to consult, but it listed the roster and not the **desks** — so an
/// orchestrator about to delegate had no authoritative id to read and
/// reached for a teammate's name instead. Every desk is listed by the id
/// `delegate_to_desk` takes, with its lead, and a desk nobody leads says so.
#[tokio::test]
async fn query_company_tool_lists_the_desks_delegation_accepts() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::seeded(desks_record(&company)));
    let tool = QueryCompanyTool::new(company, None, None, None, Some(store), None);
    let out = tool
        .execute(json!({}))
        .await
        .expect("execute")
        .output_for_llm(true);

    assert!(out.contains("## Desks"), "{out}");
    assert!(
        out.contains("**strategy** — lead: writer"),
        "a delegatable desk must name its id and lead: {out}"
    );
    assert!(
        out.contains("**archive** — no member on the roster"),
        "a leadless desk must say it cannot be handed work: {out}"
    );
}

// --- add_agent (issue #71) ----------------------------------------------

/// An in-memory `CompanyStore` so `AddAgentTool` can be exercised without a
/// filesystem, mirroring `crate::server::ops::team`'s `add_member` write
/// path (load → push overlay → save).
#[derive(Default)]
struct MemStore {
    record: StdMutex<Option<CompanyRecord>>,
}

impl MemStore {
    fn seeded(record: CompanyRecord) -> Self {
        Self {
            record: StdMutex::new(Some(record)),
        }
    }
}

#[async_trait::async_trait]
impl CompanyStore for MemStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(self.record.lock().unwrap().clone())
    }
    async fn save(&self, record: &CompanyRecord) -> crate::Result<()> {
        *self.record.lock().unwrap() = Some(record.clone());
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

fn empty_manifest() -> crate::company::CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n").expect("valid manifest")
}

fn seeded_record(id: &CompanyId) -> CompanyRecord {
    CompanyRecord {
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
    }
}

#[tokio::test]
async fn add_agent_tool_persists_an_overlay_teammate() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({
            "name": "Jamie",
            "role": "Growth Lead",
            "description": "Owns acquisition experiments."
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "add_agent should succeed");

    let record = store
        .load(&company)
        .await
        .unwrap()
        .expect("record persisted");
    assert_eq!(record.overlay_agents.len(), 1);
    let added = &record.overlay_agents[0];
    assert_eq!(added.name, "Jamie");
    assert_eq!(added.role, "Growth Lead");
    assert_eq!(
        added.description.as_deref(),
        Some("Owns acquisition experiments.")
    );
    assert!(!added.id.is_empty(), "a stable id must be minted");
    // No `tools` given → inherit the standard company-wide grant, which for
    // an unscoped minter is `None` (keeps tracking `[tools].allow`), NOT an
    // explicit empty list (which since #1804 is a deny-all).
    assert!(
        added.tools.is_none(),
        "an add with no `tools` inherits the standard grant (None), not an empty deny-all shelf"
    );
}

/// A minter scoped to part of the company grant, for the #619 tests below.
/// `minter_tools` is the line it declares; `minter_grants` is that line
/// already narrowed by the company `allow` — what `build_agent` hands the
/// tool.
fn scoped_add_agent(company: CompanyId, store: Arc<dyn CompanyStore>) -> AddAgentTool {
    AddAgentTool::new(
        company,
        store,
        "ceo".to_string(),
        Some(vec!["workspace".to_string()]),
        vec!["workspace".to_string()],
    )
}

/// Issue #619: a teammate minted by a **scoped** agent inherits that
/// agent's line, not the company's whole grant.
///
/// #661 clamped an explicit `tools` argument to the company grant, which
/// leaves this open: omitting `tools` still yields the company's *entire*
/// grant, so a narrowly scoped agent could mint a teammate holding
/// everything the company holds. `add_agent` is `Reach::Nothing` and never
/// asks, so nothing else in the path would catch it.
#[tokio::test]
async fn a_minted_teammate_is_bounded_by_its_minter_not_the_company() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = scoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({ "name": "Jamie", "role": "Growth Lead" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "got {:?}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents[0].tools,
        Some(vec!["workspace".to_string()]),
        "the minted teammate must be bounded by the agent that minted it, \
         not by the company"
    );
}

/// An **unscoped** minter still mints an unscoped teammate — the pre-#619
/// behaviour, kept deliberately. Copying the minter's *line* rather than
/// its resolved grant is what keeps the teammate tracking `[tools].allow`
/// instead of freezing today's copy of it into the record.
#[tokio::test]
async fn an_unscoped_minter_mints_an_unscoped_teammate() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({ "name": "Jamie", "role": "Growth Lead" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "got {:?}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert!(
        record.overlay_agents[0].tools.is_none(),
        "an absent line (None) means the company's standard grant (#264/#1804), \
         and an unscoped minter hands on exactly it — None, not an empty deny-all"
    );
}

/// An explicit `tools` request is narrowed against what the **minter**
/// holds, so the tool cannot hand out a grant its caller does not have.
#[tokio::test]
async fn an_explicit_scope_is_narrowed_to_what_the_minter_holds() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = scoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({
            "name": "Jamie",
            "role": "Growth Lead",
            "tools": ["workspace", "composio"]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "got {:?}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents[0].tools,
        Some(vec!["workspace".to_string()]),
        "`composio` is outside the minter's own grant and must be dropped"
    );
}

/// A request that narrows to **nothing** is a refusal, not a stored empty
/// list.
///
/// This is the sharp edge: an empty `tools` list means "inherit the
/// company's standard grant". Storing the empty result of a narrowing
/// would turn the most deliberate narrowing an agent can ask for into the
/// widest grant in the company — the exact inversion #619 exists to remove.
#[tokio::test]
async fn a_scope_entirely_outside_the_minters_grant_is_refused() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = scoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({
            "name": "Jamie",
            "role": "Growth Lead",
            "tools": ["composio"]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "got {:?}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert!(
        record.overlay_agents.is_empty(),
        "and no teammate was written at all, scoped or otherwise"
    );
}

/// Issue #661 / L5: `add_agent` carries a per-teammate tool grant onto the
/// overlay record, trimming and dropping blank globs. The grant is narrowed
/// against `[tools].allow` later (at roster build); persistence keeps the
/// authored list verbatim so the Team tab and the roster read the same thing.
#[tokio::test]
async fn add_agent_tool_persists_a_tool_grant() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({
            "name": "Ravi",
            "role": "Researcher",
            "tools": ["docs.*", "   ", "email"]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());

    let record = store.load(&company).await.unwrap().expect("record");
    assert_eq!(
        record.overlay_agents[0].tools,
        Some(vec!["docs.*".to_string(), "email".to_string()]),
        "blanks are dropped and globs trimmed"
    );
}

/// Since issue #1804 an explicit empty `tools` array is a deliberate
/// **deny-all**, NOT the standard grant — the contract inversion. Omitting
/// the field entirely is what inherits the standard grant (`None`); passing
/// `[]` deliberately hands the teammate no tools, stored as `Some(vec![])`.
#[tokio::test]
async fn add_agent_tool_empty_tools_is_an_explicit_deny_all() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({ "name": "Ravi", "role": "Researcher", "tools": [] }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());
    assert!(
        result.text().contains("hold no tools"),
        "the mint result must state the deny-all plainly: {}",
        result.text()
    );

    let record = store.load(&company).await.unwrap().expect("record");
    assert_eq!(
        record.overlay_agents[0].tools,
        Some(Vec::new()),
        "an explicit empty array is a deny-all (Some(vec![])), not the standard grant (None)"
    );
}

/// A minter whose own line names `chargebee` (the shipped bookkeeper) hands
/// that line on when `tools` is omitted — but an unstated grant never
/// confers billing (#788/#789), so the copied line is filtered before it is
/// stored. The #619 copy-the-line rule still holds for the non-BYO parts.
#[tokio::test]
async fn an_unstated_mint_from_a_billing_holding_minter_withholds_chargebee() {
    let company = CompanyId::new("acme");
    let mut record = seeded_record(&company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [tools]\n\
         allow = [\"*\", \"workspace.*\", \"workspace.write\", \"media\", \"composio\", \
         \"search\", \"mcp:*\", \"chargebee\"]\n",
    )
    .expect("valid manifest");
    let store = Arc::new(MemStore::seeded(record));
    let belt = vec![
        "*".to_string(),
        "workspace.*".to_string(),
        "workspace.write".to_string(),
        "media".to_string(),
        "composio".to_string(),
        "search".to_string(),
        "mcp:*".to_string(),
        "chargebee".to_string(),
    ];
    let tool = AddAgentTool::new(
        company.clone(),
        store.clone(),
        "bookkeeper".to_string(),
        Some(belt.clone()),
        belt,
    );

    let result = tool
        .execute(json!({ "name": "Jamie", "role": "Data Entry" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    let added = &record.overlay_agents[0];
    assert!(
        !added
            .tools
            .iter()
            .flatten()
            .any(|g| g == "chargebee" || g.starts_with("chargebee.")),
        "an unstated mint must not hand on billing: {:?}",
        added.tools
    );
    assert!(
        added.tools.iter().flatten().any(|g| g == "*"),
        "the rest of the minter's line is still copied verbatim (#619): {:?}",
        added.tools
    );
}

/// An EXPLICIT `tools` request naming `chargebee` survives — an unstated
/// grant is withheld, a stated one is narrowed to what the minter holds.
#[tokio::test]
async fn an_explicit_chargebee_request_from_a_billing_minter_is_honored() {
    let company = CompanyId::new("acme");
    let mut record = seeded_record(&company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [tools]\n\
         allow = [\"*\", \"workspace.*\", \"workspace.write\", \"media\", \"composio\", \
         \"search\", \"mcp:*\", \"chargebee\"]\n",
    )
    .expect("valid manifest");
    let store = Arc::new(MemStore::seeded(record));
    let belt = vec![
        "*".to_string(),
        "workspace.*".to_string(),
        "workspace.write".to_string(),
        "media".to_string(),
        "composio".to_string(),
        "search".to_string(),
        "mcp:*".to_string(),
        "chargebee".to_string(),
    ];
    let tool = AddAgentTool::new(
        company.clone(),
        store.clone(),
        "bookkeeper".to_string(),
        Some(belt.clone()),
        belt,
    );

    let result = tool
        .execute(json!({ "name": "Jamie", "role": "Data Entry", "tools": ["chargebee"] }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents[0].tools,
        Some(vec!["chargebee".to_string()]),
        "a stated billing namespace is narrowed to the minter's grant, not dropped"
    );
}

/// A non-string `tools` item is a clean argument error, the same shape as a
/// missing `name`/`role` — a malformed grant must not persist a half-parsed
/// teammate.
#[tokio::test]
async fn add_agent_tool_rejects_a_non_string_tool() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    assert!(
        tool.execute(json!({ "name": "Ravi", "role": "Researcher", "tools": [123] }))
            .await
            .is_err(),
        "a non-string tool glob must be rejected"
    );
    // Also rejects a non-array `tools`.
    assert!(
        tool.execute(json!({ "name": "Ravi", "role": "Researcher", "tools": "docs.*" }))
            .await
            .is_err(),
        "a non-array `tools` must be rejected"
    );

    let record = store.load(&company).await.unwrap().expect("record");
    assert!(
        record.overlay_agents.is_empty(),
        "a rejected add must not persist a teammate"
    );
}

/// Issue #686 — the tool mints the same readable, name-derived id the
/// console route does, and hands it back in the result so the orchestrator
/// can delegate to the teammate it just created.
#[tokio::test]
async fn add_agent_tool_mints_a_readable_id_and_reports_it() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({ "name": "Dana Designer", "role": "Designer" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());
    assert!(
        result.text().contains("`dana_designer`"),
        "the id must be in the result, not only in the record: {}",
        result.text()
    );

    let record = store.load(&company).await.unwrap().expect("record");
    assert_eq!(record.overlay_agents[0].id, "dana_designer");
}

/// The name guard still fires, and it fires *before* minting — so a
/// duplicate display name is refused rather than quietly given a `_2` id.
/// Two teammates the orchestrator cannot tell apart is the thing that guard
/// exists to stop, and readable ids do not make it less true.
#[tokio::test]
async fn add_agent_tool_still_refuses_a_duplicate_display_name() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    for _ in 0..1 {
        let first = tool
            .execute(json!({ "name": "Dana Designer", "role": "Designer" }))
            .await
            .expect("execute");
        assert!(!first.is_error, "{}", first.text());
    }

    let second = tool
        .execute(json!({ "name": "dana designer", "role": "Illustrator" }))
        .await
        .expect("execute");
    assert!(second.is_error, "{}", second.text());
    assert!(
        second.text().contains("already exists"),
        "{}",
        second.text()
    );

    let record = store.load(&company).await.unwrap().expect("record");
    assert_eq!(
        record.overlay_agents.len(),
        1,
        "the refusal must not have persisted a `dana_designer_2`"
    );
}

/// A name colliding with a **manifest** agent's id passes the name guard —
/// it compares overlay names — and is caught by the minter instead. The
/// roster-level consequence is pinned in `harness::tests`.
#[tokio::test]
async fn add_agent_tool_suffixes_past_a_manifest_agent_id() {
    let company = CompanyId::new("acme");
    let mut record = seeded_record(&company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"backend_engineer\"\nrole = \"Backend Engineer\"\n",
    )
    .expect("valid manifest");
    let store = Arc::new(MemStore::seeded(record));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    let result = tool
        .execute(json!({ "name": "Backend Engineer", "role": "Platform" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", result.text());

    let record = store.load(&company).await.unwrap().expect("record");
    assert_eq!(record.overlay_agents[0].id, "backend_engineer_2");
}

#[tokio::test]
async fn add_agent_tool_requires_name_and_role() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    assert!(
        tool.execute(json!({ "role": "Growth Lead" }))
            .await
            .is_err(),
        "missing `name` must be rejected"
    );
    assert!(
        tool.execute(json!({ "name": "Jamie" })).await.is_err(),
        "missing `role` must be rejected"
    );
    let record = store.load(&company).await.unwrap().expect("record");
    assert!(
        record.overlay_agents.is_empty(),
        "a rejected call must not persist a half-formed teammate"
    );
}

#[tokio::test]
async fn add_agent_tool_reports_company_not_found() {
    let company = CompanyId::new("ghost");
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::default());
    let tool = unscoped_add_agent(company, store);

    let err = tool
        .execute(json!({ "name": "Jamie", "role": "Growth Lead" }))
        .await
        .expect_err("no record for this company id");
    assert!(err.to_string().contains("ghost"), "{err}");
}

// ---- run_workflow (issue #67) ----

/// A valid trigger → agent → output graph, mirroring the REST route's fixture.
const DEMO_WF: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
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
"#;

/// A [`WorkflowRunner`] test double: records the ids it was asked to run and
/// returns a canned [`WorkflowRun`].
struct StubRunner {
    calls: Arc<Mutex<Vec<String>>>,
    run: WorkflowRun,
}

impl StubRunner {
    fn new(run: WorkflowRun) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            run,
        }
    }

    fn empty() -> Self {
        Self::new(WorkflowRun {
            output: Value::Null,
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

#[async_trait::async_trait]
impl WorkflowRunner for StubRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        workflow: &WorkflowFile,
        _input: Value,
        _ctx: &crate::ports::WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        self.calls.lock().unwrap().push(workflow.id.clone());
        Ok(self.run.clone())
    }
}

/// A [`WorkflowRunner`] test double whose `run` always returns `Err` — the
/// engine-failed shape issue #1865's review comment 3877185396 flagged as
/// silent: `RunWorkflowTool`'s `Ok(Err(err))` arm journaled a finish but
/// filed no `workflow_run_failed` notification, unlike the console run
/// route, the cron scheduler, and the approval-resume path, which all
/// file one through `WorkflowSpawn`.
struct FailingRunner;

#[async_trait::async_trait]
impl WorkflowRunner for FailingRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &WorkflowFile,
        _input: Value,
        _ctx: &crate::ports::WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        Err(crate::error::OpenCompanyError::Harness(
            "the engine blew up".to_string(),
        ))
    }
}

/// Writes `DEMO_WF` to `<dir>/workflows/demo.toml`.
fn seed_demo_workflow(dir: &std::path::Path) {
    let wf = dir.join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("demo.toml"), DEMO_WF).unwrap();
}

#[test]
fn workflow_runner_handle_is_empty_until_filled() {
    let handle = WorkflowRunnerHandle::default();
    assert!(handle.get().is_none());
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::empty());
    handle.set(&runner);
    assert!(handle.get().is_some());
}

#[test]
fn workflow_runner_handle_holds_only_a_weak_reference() {
    // Proves the deps↔runner cell is not a strong cycle: once the sole strong
    // owner drops, the handle can no longer upgrade.
    let handle = WorkflowRunnerHandle::default();
    {
        let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::empty());
        handle.set(&runner);
        assert!(handle.get().is_some());
    }
    assert!(
        handle.get().is_none(),
        "the handle must not keep the runner alive"
    );
}

#[test]
fn orchestrator_tools_includes_all_sixteen() {
    use crate::harness::workflow_admin::{
        DELETE_WORKFLOW_TOOL, READ_WORKFLOW_TOOL, UPDATE_WORKFLOW_TOOL,
    };
    let queue = DelegationQueue::default();
    let tools = orchestrator_tools(
        CompanyId::new("acme"),
        None,
        None,
        None,
        None,
        None,
        &queue,
        None,
        WorkflowRunnerHandle::default(),
        crate::runtime::RunSupervisor::default(),
        Arc::new(MemStore::default()),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        "ceo".to_string(),
        None,
        vec!["fs:*".to_string()],
        None,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    // Six before #186; `assign_task` + `review_task` made eight; #418's
    // `read_run_output` makes nine; #661's read/update/delete_workflow
    // trio makes twelve; #884's `delegate_to_teammate` makes thirteen;
    // #1859's `list_tasks` / `read_task` / `read_run` trio makes sixteen.
    assert_eq!(names.len(), 16, "got {names:?}");
    assert!(names.contains(&DELEGATE_TO_TEAMMATE_TOOL), "got {names:?}");
    assert!(names.contains(&RUN_WORKFLOW_TOOL), "got {names:?}");
    assert!(names.contains(&READ_RUN_OUTPUT_TOOL), "got {names:?}");
    assert!(names.contains(&CREATE_WORKFLOW_TOOL), "got {names:?}");
    assert!(names.contains(&READ_WORKFLOW_TOOL), "got {names:?}");
    assert!(names.contains(&UPDATE_WORKFLOW_TOOL), "got {names:?}");
    assert!(names.contains(&DELETE_WORKFLOW_TOOL), "got {names:?}");
    assert!(names.contains(&ADD_AGENT_TOOL), "got {names:?}");
    assert!(names.contains(&QUERY_COMPANY_TOOL), "got {names:?}");
    assert!(names.contains(&SPAWN_TASK_TOOL), "got {names:?}");
    assert!(names.contains(&DELEGATE_TO_DESK_TOOL), "got {names:?}");
    assert!(names.contains(&ASSIGN_TASK_TOOL), "got {names:?}");
    assert!(names.contains(&REVIEW_TASK_TOOL), "got {names:?}");
    assert!(names.contains(&LIST_TASKS_TOOL), "got {names:?}");
    assert!(names.contains(&READ_TASK_TOOL), "got {names:?}");
    assert!(names.contains(&READ_RUN_TOOL), "got {names:?}");
    // `read_run_output` sits immediately after `run_workflow`.
    let run_at = names.iter().position(|n| *n == RUN_WORKFLOW_TOOL).unwrap();
    assert_eq!(names[run_at + 1], READ_RUN_OUTPUT_TOOL, "got {names:?}");
    // The #661 trio sits immediately after `create_workflow`: they are its
    // lifecycle, and a model reads the belt in order.
    let created_at = names
        .iter()
        .position(|n| *n == CREATE_WORKFLOW_TOOL)
        .unwrap();
    assert_eq!(
        &names[created_at + 1..created_at + 4],
        &[
            READ_WORKFLOW_TOOL,
            UPDATE_WORKFLOW_TOOL,
            DELETE_WORKFLOW_TOOL
        ],
        "got {names:?}"
    );
    // #1859's read trio sits immediately after `query_company`: all four
    // answer "what does the company know?" rather than acting on it.
    let query_at = names.iter().position(|n| *n == QUERY_COMPANY_TOOL).unwrap();
    assert_eq!(
        &names[query_at + 1..query_at + 4],
        &[LIST_TASKS_TOOL, READ_TASK_TOOL, READ_RUN_TOOL],
        "got {names:?}"
    );
}

/// A runner panic is converted into an agent-visible error, and the RAII
/// supervisor slot is gone when the tool returns. This covers both cleanup
/// obligations without changing the runner architecture.
#[tokio::test]
async fn panicking_run_cleans_up_its_active_attempt() {
    struct PanickingRunner;

    #[async_trait::async_trait]
    impl WorkflowRunner for PanickingRunner {
        async fn run(
            &self,
            _company: &CompanyId,
            _workflow: &WorkflowFile,
            _input: Value,
            _ctx: &crate::ports::WorkflowRunContext,
        ) -> crate::Result<WorkflowRun> {
            panic!("test runner panic")
        }
    }

    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());
    let runner: Arc<dyn WorkflowRunner> = Arc::new(PanickingRunner);
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let supervisor = crate::runtime::RunSupervisor::default();
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        supervisor.clone(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );

    let result = tool
        .execute(json!({"id": "demo"}))
        .await
        .expect("panic is converted to a tool result");
    assert!(result.is_error, "panic must be agent-visible: {result:?}");
    assert!(
        result.output_for_llm(false).contains("internal error"),
        "the result should not leak panic payload: {result:?}"
    );
    assert_eq!(supervisor.len(), 0, "the active attempt must be cleaned up");
}

#[tokio::test]
async fn run_workflow_tool_loads_and_invokes_the_runner() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner_impl = StubRunner::new(WorkflowRun {
        output: json!({
            "run": {},
            "nodes": { "worker": { "items": ["did the thing"] }, "done": { "items": [] } }
        }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    });
    let calls = runner_impl.calls.clone();
    let runner: Arc<dyn WorkflowRunner> = Arc::new(runner_impl);
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo", "input": { "seed": 1 } }))
        .await
        .expect("execute");

    assert!(!result.is_error, "expected success, got {result:?}");
    assert_eq!(calls.lock().unwrap().as_slice(), ["demo"]);
    let out = result.output_for_llm(true);
    assert!(out.contains("Demo flow"), "{out}");
    assert!(out.contains("did the thing"), "{out}");
    assert!(out.contains("without pausing for approval"), "{out}");
}

/// Issue #339: a run this tool started is staged for the dispatched card
/// that started it, carrying the run id so the card's link can open the
/// overlay showing what actually executed.
#[tokio::test]
async fn a_successful_run_stages_a_workflow_reference_for_the_card() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["did the thing"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let refs = WorkflowRefQueue::default();

    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs.clone(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{result:?}");

    let staged = refs.drain();
    assert_eq!(staged.len(), 1, "got {staged:?}");
    assert_eq!(staged[0].workflow_id, "demo");
    assert_eq!(staged[0].action, TaskOutputAction::Ran);
    assert!(
        staged[0].run_id.is_some(),
        "a run the card links to must name the run that happened"
    );
}

/// The other half, and the one worth pinning: a run that never happened
/// stages nothing. An unknown id, an unwired runner and a failed run all
/// produced no deliverable, so a card must not advertise one.
#[tokio::test]
async fn a_run_that_did_not_happen_stages_nothing() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());
    let refs = WorkflowRefQueue::default();

    // No runner wired.
    let unwired = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        WorkflowRunnerHandle::default(),
        crate::runtime::RunSupervisor::default(),
        None,
        refs.clone(),
        RunOutputCache::default(),
        None,
    );
    assert!(
        unwired
            .execute(json!({ "id": "demo" }))
            .await
            .expect("execute")
            .is_error
    );

    // A wired runner, but an id neither source has.
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::empty());
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let unknown = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs.clone(),
        RunOutputCache::default(),
        None,
    );
    assert!(
        unknown
            .execute(json!({ "id": "nope" }))
            .await
            .expect("execute")
            .is_error
    );

    assert_eq!(refs.queued(), 0, "nothing ran, so nothing may be linked");
}

/// A run an operator stopped is not a deliverable to put on a card. Its
/// partial steps stay in the run history either way, so nothing is lost —
/// what is avoided is a card advertising work somebody deliberately halted.
#[tokio::test]
async fn a_cancelled_run_stages_nothing() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": {} }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: true,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let refs = WorkflowRefQueue::default();

    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs.clone(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(result.is_error, "a cancelled run reports as a stop");
    assert_eq!(refs.queued(), 0);
}

/// Issue #1861: an agent-initiated run that ends blocked badges the
/// operator, exactly as the console's and the scheduler's runs do.
///
/// This is the one trigger nobody is watching a progress bar for, so the
/// badge is the only way a run that stopped waiting on a person becomes
/// visible without somebody thinking to open the run history.
#[tokio::test]
async fn a_blocked_agent_run_badges_the_operator() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": {} }),
        pending_approvals: vec!["worker".to_string()],
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: vec![crate::ports::workflow_runner::WorkflowBlockedNode {
            node_id: "worker".to_string(),
            tools: vec!["send_email".to_string()],
            approval_ids: vec!["ap-1".to_string()],
            unparkable: 0,
            stranded: 0,
            blockers: 0,
        }],
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let notifications: Arc<dyn crate::ports::notifications::NotificationStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        Some(notifications.clone()),
    );
    tool.execute(json!({ "id": "demo" }))
        .await
        .expect("execute");

    let feed = notifications
        .list(&CompanyId::new("acme"), "ceo")
        .await
        .expect("list");
    assert_eq!(feed.len(), 1, "one badge for one unhealthy run: {feed:?}");
    assert_eq!(feed[0].notification.kind, "workflow_run_blocked");
}

/// The same contract for the other unhealthy end: the run could not park
/// the approval at all, so nobody was asked and nothing is waiting.
#[tokio::test]
async fn a_stranded_agent_run_badges_the_operator() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": {} }),
        // Stranded is counted per pending *node* (`stranded_approvals`),
        // not per gated call: the node is pending, and every approval row
        // it owns failed to park, so there is nothing an operator can be
        // asked about. A fixture with no pending node at all is not a
        // stranded run under that reconciliation — it is an empty one.
        pending_approvals: vec!["worker".to_string()],
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: vec![crate::ports::workflow_runner::WorkflowRunApprovalRow {
            node_id: Some("worker".to_string()),
            tool: Some("send_email".to_string()),
            outcome: crate::ports::workflow_runner::WorkflowApprovalOutcome::ParkFailed,
            approval_id: None,
        }],
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let notifications: Arc<dyn crate::ports::notifications::NotificationStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        Some(notifications.clone()),
    );
    tool.execute(json!({ "id": "demo" }))
        .await
        .expect("execute");

    let feed = notifications
        .list(&CompanyId::new("acme"), "ceo")
        .await
        .expect("list");
    assert_eq!(feed.len(), 1, "{feed:?}");
    assert_eq!(feed[0].notification.kind, "workflow_run_stranded");
}

/// A run that finished cleanly badges nobody. The badge means "this needs
/// you"; one per successful run would train the operator to ignore it.
#[tokio::test]
async fn a_healthy_agent_run_badges_nobody() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": [] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let notifications: Arc<dyn crate::ports::notifications::NotificationStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        Some(notifications.clone()),
    );
    tool.execute(json!({ "id": "demo" }))
        .await
        .expect("execute");

    let feed = notifications
        .list(&CompanyId::new("acme"), "ceo")
        .await
        .expect("list");
    assert!(feed.is_empty(), "{feed:?}");
}

#[tokio::test]
async fn run_workflow_tool_surfaces_pending_approvals() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": [] } } }),
        pending_approvals: vec!["worker".to_string()],
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(!result.is_error);
    let out = result.output_for_llm(true);
    assert!(out.contains("Paused for approval"), "{out}");
    assert!(out.contains("worker"), "{out}");
}

/// Issue #900 (tinysweeper `missing-test`): `summarize_run`'s blocked branch
/// had no coverage at all, and the doc comment on `blocked` / `paused`
/// (issue #881) — that a blocked node and a paused gate need separate
/// sentences even though both ride `pending_approvals` — was untested along
/// with it. One node blocks, a second is an ordinary paused gate: the
/// summary must name the blocked node under "Blocked, waiting on a person"
/// (never under "Paused for approval", which would tell the agent the run
/// resumes on its own) and the paused node under "Paused for approval"
/// only. The structural JSON counts (issue #881) must agree.
#[tokio::test]
async fn run_workflow_tool_separates_blocked_nodes_from_paused_gates() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": [] } } }),
        // Issue #881: the union — the blocked node's id rides here too, and
        // `summarize_run` is what has to keep it out of the "Paused for
        // approval" line.
        pending_approvals: vec!["worker".to_string(), "gate".to_string()],
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: vec![crate::ports::WorkflowBlockedNode {
            node_id: "worker".to_string(),
            tools: vec!["publish_artifact".to_string()],
            approval_ids: vec!["appr-1".to_string()],
            unparkable: 0,
            stranded: 0,
            blockers: 0,
        }],
        approvals: vec![
            crate::ports::WorkflowRunApprovalRow {
                node_id: Some("worker".to_string()),
                tool: Some("publish_artifact".to_string()),
                outcome: crate::ports::WorkflowApprovalOutcome::Parked,
                approval_id: Some("appr-1".to_string()),
            },
            // Issue #900: a receipt for a call that did NOT land a card.
            // `run.approvals.len()` would count this as a second "parked"
            // approval; the JSON's `approvals_parked` must not.
            crate::ports::WorkflowRunApprovalRow {
                node_id: Some("worker".to_string()),
                tool: Some("publish_artifact".to_string()),
                outcome: crate::ports::WorkflowApprovalOutcome::ParkFailed,
                approval_id: None,
            },
        ],
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let refs = WorkflowRefQueue::default();
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs,
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(!result.is_error);
    let out = result.output_for_llm(true);
    assert!(
        out.contains("Blocked, waiting on a person") && out.contains("worker"),
        "{out}"
    );
    assert!(
        out.contains("Paused for approval") && out.contains("gate"),
        "{out}"
    );
    // The blocked node must not also read as an ordinary paused gate — that
    // sentence promises the run continues once it is decided, which is
    // false for a block (issue #881).
    let paused_line = out
        .lines()
        .find(|l| l.contains("Paused for approval"))
        .expect("a Paused for approval line");
    assert!(!paused_line.contains("worker"), "{out}");

    let payload = match &result.content[0] {
        openhuman_core::skills::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected JSON payload, got {other:?}"),
    };
    assert_eq!(
        payload.get("blocked_nodes").and_then(Value::as_u64),
        Some(1)
    );
    // Issue #900: two receipts on this run (one parked, one that failed to
    // park), and the JSON count must name only the decidable one.
    assert_eq!(
        payload.get("approvals_parked").and_then(Value::as_u64),
        Some(1),
        "approvals_parked must exclude the ParkFailed receipt: {payload}"
    );
}

#[tokio::test]
async fn run_workflow_tool_errors_when_no_runner_is_wired() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());
    // A valid workflow on disk, but an empty handle → not wired.
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        WorkflowRunnerHandle::default(),
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(result.is_error, "expected an error result");
    assert!(result.output_for_llm(false).contains("wired"), "{result:?}");
}

/// Issue #1865 (PR #1883 review comment 3877185396): an agent-started run
/// that the engine returns `Err` on is the second run-outcome chokepoint
/// `WorkflowSpawn` does not cover — console, scheduled, and resumed
/// failures all file a `workflow_run_failed` notification through that
/// type, but this tool's own `Ok(Err(err))` arm used to journal a finish
/// and stop, leaving every agent-started failure invisible to an operator
/// not watching this turn. Reused `crate::store::FsOps` as the
/// notification-store double, the same one `WorkflowSpawn`'s own
/// equivalent test (`a_failed_run_does_not_leak_the_raw_engine_error_into_its_notification`
/// in `runtime::workflow_spawn`) uses.
#[tokio::test]
async fn run_workflow_tool_files_a_notification_when_the_engine_run_fails() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());
    let runner: Arc<dyn WorkflowRunner> = Arc::new(FailingRunner);
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let company = CompanyId::new("acme");
    let notifications: Arc<dyn NotificationStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));

    let tool = RunWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        Some(notifications.clone()),
    );
    let result = tool
        .execute(json!({ "id": "demo" }))
        .await
        .expect("execute");
    assert!(result.is_error, "the engine failed: {result:?}");

    let notes = notifications
        .list(&company, "anyone")
        .await
        .expect("list notifications");
    let failed = notes
        .iter()
        .find(|n| n.notification.kind == "workflow_run_failed")
        .expect(
            "an agent-started run that fails must file the same durable notification a \
             console, scheduled, or resumed run does",
        );
    assert!(
        failed
            .notification
            .title
            .contains(crate::runtime::RUN_FAILED_DETAIL),
        "{:?}",
        failed.notification.title
    );
}

#[tokio::test]
async fn run_workflow_tool_errors_on_unknown_id() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::empty());
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);

    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "nope" }))
        .await
        .expect("execute");
    assert!(result.is_error);
    assert!(
        result.output_for_llm(false).contains("No workflow with id"),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_workflow_tool_requires_an_id() {
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        None,
        Arc::new(MemStore::default()),
        WorkflowRunnerHandle::default(),
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool.execute(json!({})).await.expect("execute");
    assert!(result.is_error);
    assert!(
        result.output_for_llm(false).contains("`id` is required"),
        "{result:?}"
    );
}

#[tokio::test]
async fn run_workflow_tool_rejects_traversal_ids() {
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::empty());
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(std::path::PathBuf::from("/tmp")),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = tool
        .execute(json!({ "id": "../secrets" }))
        .await
        .expect("execute");
    assert!(result.is_error);
}

// ---- create_workflow (issue #112) ----

/// A record with an `assistant` roster agent so an `agent`-node graph passes
/// the roster cross-check inside the create core.
fn record_with_assistant(company: &CompanyId) -> CompanyRecord {
    let manifest: crate::company::CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n",
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: company.clone(),
        manifest,
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
    }
}

/// The canonical happy graph the create tool accepts (camelCase body).
fn greeter_body() -> Value {
    json!({
        "id": "greeter",
        "name": "Greeter",
        "description": "Says hi.",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "worker", "kind": "agent", "name": "Worker", "agent": "assistant" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [
            { "from": "start", "to": "worker" },
            { "from": "worker", "to": "done", "label": "ok" }
        ]
    })
}

#[tokio::test]
async fn create_workflow_tool_then_run_workflow_tool() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));

    // Author the graph.
    let create = CreateWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let created = create.execute(greeter_body()).await.expect("execute");
    assert!(!created.is_error, "create should succeed: {created:?}");
    assert!(
        created.output_for_llm(true).contains("run_workflow"),
        "{created:?}"
    );

    // It's enabled on the record.
    let record = store.load(&company).await.unwrap().unwrap();
    assert!(
        record
            .manifest
            .workflows
            .enabled
            .contains(&"greeter".to_string())
    );

    // And immediately runnable via the run tool over the same source dir.
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["hi"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let run = RunWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = run
        .execute(json!({ "id": "greeter" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "run should succeed: {result:?}");
    assert!(
        result.output_for_llm(true).contains("Greeter"),
        "{result:?}"
    );
}

/// Issue #401: the orchestrator's run tool refuses when the company is
/// already at its in-flight run ceiling. The refusal is a
/// `ToolResult::error` the agent should treat as "wait / stop one", NOT an
/// `Err`, and it registers nothing — a held guard stands in for the
/// in-flight run, so no wall-clock and no real second run is needed.
#[tokio::test]
async fn run_workflow_tool_refuses_at_the_in_flight_cap() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));

    // Author a runnable graph on disk so `execute` reaches the cap check.
    let create = CreateWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    assert!(
        !create
            .execute(greeter_body())
            .await
            .expect("execute")
            .is_error
    );

    // A supervisor with room for one run, whose only slot is already taken
    // by a (simulated) in-flight run held for the length of the test.
    let supervisor = crate::runtime::RunSupervisor::with_limit(1);
    let (_ctx, _held) = supervisor
        .begin("greeter", false)
        .expect("the held run fills the cap of 1");

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({}),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let run = RunWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        handle,
        supervisor.clone(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );

    let result = run
        .execute(json!({ "id": "greeter" }))
        .await
        .expect("execute");
    assert!(result.is_error, "a run over the cap is refused: {result:?}");
    let text = result.output_for_llm(false);
    assert!(
        text.contains("wasn't started") && text.contains("maximum"),
        "the refusal names the cap and is actionable: {text}"
    );
    assert_eq!(
        supervisor.len(),
        1,
        "the refused run registered nothing — only the held run remains"
    );
}

/// Issue #339: the *"build us a process for this"* card. The graph is the
/// deliverable, so authoring it stages a link even though nothing has run —
/// and when the same turn goes on to run it, the pair collapses to the run,
/// which is the stronger link because it can show what executed.
#[tokio::test]
async fn authoring_a_workflow_stages_a_link_and_running_it_upgrades_it() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let refs = WorkflowRefQueue::default();

    let create = CreateWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        None,
        refs.clone(),
    );
    assert!(
        !create
            .execute(greeter_body())
            .await
            .expect("execute")
            .is_error
    );
    assert_eq!(refs.queued(), 1, "the saved graph is a deliverable");

    // A rejected draft persists nothing, so it must stage nothing either.
    assert!(
        create
            .execute(json!({ "id": "greeter", "name": "Greeter", "nodes": [] }))
            .await
            .expect("execute")
            .is_error
    );
    assert_eq!(refs.queued(), 1, "a rejected draft is not a deliverable");

    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["hi"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let run = RunWorkflowTool::new(
        company.clone(),
        Some(dir.path().to_path_buf()),
        store.clone(),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs.clone(),
        RunOutputCache::default(),
        None,
    );
    assert!(
        !run.execute(json!({ "id": "greeter" }))
            .await
            .expect("execute")
            .is_error
    );

    let staged = refs.drain();
    assert_eq!(staged.len(), 1, "one workflow, one link: {staged:?}");
    assert_eq!(staged[0].workflow_id, "greeter");
    assert_eq!(staged[0].action, TaskOutputAction::Ran);
    assert!(staged[0].run_id.is_some());
}

#[tokio::test]
async fn create_workflow_tool_guardrail_failure_is_error_result() {
    let dir = tempfile::tempdir().unwrap();
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(
        company,
        Some(dir.path().to_path_buf()),
        store,
        None,
        WorkflowRefQueue::default(),
    );
    // Zero triggers — a guardrail failure must be an is_error ToolResult, not
    // a raised anyhow error.
    let result = tool
        .execute(json!({
            "id": "bad",
            "name": "Bad",
            "nodes": [ { "id": "a", "kind": "output", "name": "A" } ],
            "edges": []
        }))
        .await
        .expect("execute returns a result, not an error");
    assert!(result.is_error, "{result:?}");
    assert!(
        result.output_for_llm(false).contains("trigger"),
        "{result:?}"
    );
}

/// Issue #168: a hosted tenant has no source directory, and the tool must
/// still create — the graph body is persisted on the record. It used to
/// refuse outright with "nowhere to save".
#[tokio::test]
async fn create_workflow_tool_creates_without_source_dir() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "hosted",
            "name": "Hosted",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                { "id": "done", "kind": "output", "name": "Done" }
            ],
            "edges": [ { "from": "start", "to": "done" } ]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{result:?}");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert_eq!(record.overlay_workflows[0].id, "hosted");

    // And it runs, with no source directory anywhere in the picture.
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(WorkflowRun {
        output: json!({ "nodes": { "done": { "items": ["ok"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let run = RunWorkflowTool::new(
        company,
        None,
        store,
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        WorkflowRefQueue::default(),
        RunOutputCache::default(),
        None,
    );
    let result = run
        .execute(json!({ "id": "hosted" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "run should succeed: {result:?}");
    assert!(result.output_for_llm(true).contains("Hosted"), "{result:?}");
}

#[tokio::test]
async fn create_workflow_tool_errors_on_unreadable_args() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::default());
    let tool = CreateWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.path().to_path_buf()),
        store,
        None,
        WorkflowRefQueue::default(),
    );
    // A non-object payload can't deserialize into the create body.
    let result = tool.execute(json!(42)).await.expect("execute");
    assert!(result.is_error);
    assert!(
        result.output_for_llm(false).contains("Couldn't read"),
        "{result:?}"
    );
}

/// Like [`record_with_assistant`], but the company `[tools].allow` grants the
/// `web` namespace so a `web_fetch` `tool_call` clears the author-time grant
/// gate under the `openhuman` build (issue #661).
fn record_granting_web(company: &CompanyId) -> CompanyRecord {
    let mut record = record_with_assistant(company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [\"web\"]\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n",
    )
    .expect("valid manifest");
    record
}

/// Issue #661 (H1): a `tool_call` node authored with `config.slug` persists
/// the slug into the saved graph — the tool advertises `tool_call` and can
/// now actually author a working one. Round-trip proof: the rendered TOML on
/// the record carries `slug = "web_fetch"`.
#[tokio::test]
async fn create_workflow_tool_persists_tool_call_config_slug() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_granting_web(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "fetcher",
            "name": "Fetcher",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "grab",
                    "kind": "tool_call",
                    "name": "Grab",
                    "config": { "slug": "web_fetch", "args": { "url": "https://example.com" } }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{result:?}");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert!(
        record.overlay_workflows[0]
            .toml
            .contains("slug = \"web_fetch\""),
        "the persisted graph carries the tool slug: {}",
        record.overlay_workflows[0].toml
    );
}

/// Issue #1882 (tinysweeper): every other external boundary that turns a
/// caller-supplied `ownerDesk` into a [`RawWorkflow`] runs it through
/// [`RawWorkflow::normalize_owner_desk`] — the HTTP create route
/// (`server::ops::workflows`) and the proposal-apply path
/// (`workflow_create::raw_workflow_from_spec`) — so a blank/whitespace
/// string is stored as `None`, not `Some("   ")`. The orchestrator's
/// `create_workflow` tool passed `args.owner_desk` straight through
/// instead, so a whitespace `ownerDesk` persisted verbatim in the graph's
/// TOML and would defeat the `is_none()` fallback
/// `apply_workflow_proposal` relies on later.
#[tokio::test]
async fn create_workflow_tool_normalizes_a_blank_owner_desk() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );

    let mut body = greeter_body();
    body["ownerDesk"] = json!("   ");
    let result = tool.execute(body).await.expect("execute");
    assert!(!result.is_error, "{result:?}");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert!(
        !record.overlay_workflows[0].toml.contains("owner_desk"),
        "a blank owner_desk must normalize to None and be omitted from the \
         persisted TOML, matching every other boundary that builds a \
         RawWorkflow: {}",
        record.overlay_workflows[0].toml
    );
}

/// PR #1882 review (bot finding on `orchestrator.rs:4788`).
/// `UpdateWorkflowTool`'s description (built from this same schema via
/// `create_graph_schema`) tells the agent to send `"ownerDesk": null` to
/// unassign a desk, and `an_update_can_explicitly_clear_owner_desk_with_null`
/// proves `execute` honors that. But `execute` is called directly in that
/// test, bypassing the boundary a schema-constrained tool-calling client
/// actually enforces: before this fix `ownerDesk` was declared bare
/// `"type": "string"`, so such a client would reject the `null` argument
/// before the call ever reached `execute`'s presence check, leaving the
/// advertised clear operation reachable in tests but not in the field.
#[test]
fn owner_desk_schema_permits_null() {
    let schema = create_workflow_parameters_schema();
    let owner_desk_type = &schema["properties"]["ownerDesk"]["type"];
    let permits_null = owner_desk_type
        .as_array()
        .map(|types| types.iter().any(|t| t == "null"))
        .unwrap_or(false);
    assert!(
        permits_null,
        "ownerDesk schema type must include \"null\" so a schema-constrained \
         client can send the explicit-clear value the tool description \
         promises; got {owner_desk_type:?}"
    );
}

/// Issue #661 (H1): an `output` node's `destination` flows through into the
/// saved graph — the persisted TOML carries the routed address.
#[tokio::test]
async fn create_workflow_tool_persists_output_destination() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "reporter",
            "name": "Reporter",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "done",
                    "kind": "output",
                    "name": "Report",
                    "destination": { "kind": "email", "target": "ada@example.com" }
                }
            ],
            "edges": [ { "from": "start", "to": "done" } ]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{result:?}");

    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    let saved = &record.overlay_workflows[0].toml;
    assert!(
        saved.contains("target = \"ada@example.com\""),
        "the persisted graph routes to the destination address: {saved}"
    );
}

/// Issue #661 (H1): a `tool_call` with no `slug` is still rejected — the
/// inherited author-time gate, now reachable with a useful message instead of
/// the tool being unable to author a `tool_call` at all.
#[tokio::test]
async fn create_workflow_tool_rejects_tool_call_without_slug() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "bad",
            "name": "Bad",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                { "id": "grab", "kind": "tool_call", "name": "Grab" },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    assert!(
        result.output_for_llm(false).contains("slug"),
        "the refusal names the missing slug: {result:?}"
    );
}

/// Issue #661 (H1): the exact GitHub/Composio failure mode — a `tool_call`
/// naming an agent-turn tool family (`composio_execute`) can never run on a
/// workflow `tool_call` node, so it is refused at save. Gated on `openhuman`
/// because the namespace resolution (`namespace_of`) lives behind it.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn create_workflow_tool_rejects_agent_turn_tool_call() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "gh",
            "name": "GitHub",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "call",
                    "kind": "tool_call",
                    "name": "Call",
                    "config": { "slug": "composio_execute" }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "call" },
                { "from": "call", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    assert!(
        result.output_for_llm(false).contains("agent-turn"),
        "the refusal explains it is an agent-turn family, not a workflow tool: {result:?}"
    );
}

/// Issue #661 (H1): a JSON `null` inside a node's `config` can't be stored —
/// TOML has no null — so the fallible `TryFrom` conversion refuses it as an
/// agent-actionable error, never a panic or a silently-dropped key.
#[tokio::test]
async fn create_workflow_tool_rejects_null_config_value() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "nullish",
            "name": "Nullish",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "grab",
                    "kind": "tool_call",
                    "name": "Grab",
                    "config": { "slug": "web_fetch", "args": { "url": null } }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    assert!(
        result.output_for_llm(false).contains("TOML has no null"),
        "the refusal explains why the config can't be stored: {result:?}"
    );
}

/// Issue #674 boundary: an agent-authored `tool_call` whose `config.args`
/// carries a templated `=`-expression is rejected — that node would take
/// saved-node runtime position with model-chosen templated args, collapsing
/// the two-operator-gate model. The refusal names the node and points at the
/// console for templated wiring. Feature-independent: the check runs in the
/// `TryFrom`, before any namespace/grant gate.
#[tokio::test]
async fn create_workflow_tool_rejects_tool_call_expression_args() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_granting_web(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "templated",
            "name": "Templated",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "grab",
                    "kind": "tool_call",
                    "name": "Grab",
                    "config": { "slug": "web_fetch", "args": { "url": "=item.url" } }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    let msg = result.output_for_llm(false);
    assert!(
        msg.contains("=`-expression") && msg.contains("config.args.url"),
        "the refusal names the templated expression and its location: {result:?}"
    );
    // Nothing was persisted — the reject happens before the store write.
    let record = store.load(&company).await.unwrap().unwrap();
    assert!(
        record.overlay_workflows.is_empty(),
        "a rejected draft persists nothing"
    );
}

/// Issue #674 boundary, positive half: the same `tool_call` with a LITERAL
/// arg (no `=` prefix) persists — the restriction is on templated
/// `=`-expressions, not on args as such.
#[tokio::test]
async fn create_workflow_tool_persists_tool_call_literal_args() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_granting_web(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "literal",
            "name": "Literal",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "grab",
                    "kind": "tool_call",
                    "name": "Grab",
                    "config": { "slug": "web_fetch", "args": { "url": "https://example.com" } }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{result:?}");
    let record = store.load(&company).await.unwrap().unwrap();
    assert_eq!(record.overlay_workflows.len(), 1);
    assert!(
        record.overlay_workflows[0]
            .toml
            .contains("url = \"https://example.com\""),
        "the persisted graph carries the literal arg: {}",
        record.overlay_workflows[0].toml
    );
}

/// Issue #661 (H1): the `=`-expression restriction is scoped to `tool_call`.
/// A `condition` node legitimately branches on a `config.field` expression,
/// so a `=`-prefixed field must NOT be rejected — proving the guard doesn't
/// over-reach into the kinds that resolve expressions by design.
#[tokio::test]
async fn create_workflow_tool_allows_condition_expression_field() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(
        company.clone(),
        None,
        store.clone(),
        None,
        WorkflowRefQueue::default(),
    );
    let result = tool
        .execute(json!({
            "id": "brancher",
            "name": "Brancher",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "check",
                    "kind": "condition",
                    "name": "Check",
                    "config": { "field": "=item.ok" }
                },
                { "id": "yes", "kind": "output", "name": "Yes" },
                { "id": "no", "kind": "output", "name": "No" }
            ],
            "edges": [
                { "from": "start", "to": "check" },
                { "from": "check", "to": "yes", "label": "yes" },
                { "from": "check", "to": "no", "label": "no" }
            ]
        }))
        .await
        .expect("execute");
    assert!(
        !result.is_error,
        "a condition's `=`-expression field is allowed: {result:?}"
    );
}

/// Issue #661 (H1): a non-object `config` (here a bare string on an
/// `http_request` node — the path that would otherwise persist silently) is
/// refused with an agent-actionable message, not saved as an inert TOML
/// scalar.
#[tokio::test]
async fn create_workflow_tool_rejects_non_object_config() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "scalar",
            "name": "Scalar",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "call",
                    "kind": "http_request",
                    "name": "Call",
                    "config": "GET https://example.com"
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "call" },
                { "from": "call", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    assert!(
        result.output_for_llm(false).contains("non-object `config`"),
        "the refusal explains config must be a JSON object: {result:?}"
    );
}

/// Issue #661 (H1) — item #2: a `destination` on a non-`output` node is
/// already rejected end-to-end by the shared `validate` (`render_workflow` →
/// `parse_workflow` inside `create_company_workflow`), so the create_workflow
/// tool inherits the catch with no duplicated validation of its own. This
/// pins that end-to-end behaviour; the shared-validator hardening is #682's.
#[tokio::test]
async fn create_workflow_tool_rejects_destination_on_non_output() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "misrouted",
            "name": "Misrouted",
            "nodes": [
                {
                    "id": "start",
                    "kind": "trigger",
                    "name": "Start",
                    "destination": { "kind": "owner" }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [ { "from": "start", "to": "done" } ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    assert!(
        result
            .output_for_llm(false)
            .contains("only `output` nodes route a report"),
        "the shared validator's destination-placement message surfaces: {result:?}"
    );
}

/// Issue #661 (H1) — item #3: the JSON→TOML conversion remedy is conditional.
/// A failure that is NOT about a null (here a `u64` beyond `i64` range) must
/// get the converter's own message WITHOUT the misleading "TOML has no null"
/// hint — the null case keeps that hint (`create_workflow_tool_rejects_null_config_value`).
#[tokio::test]
async fn create_workflow_tool_non_null_conversion_error_omits_null_hint() {
    let company = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> =
        Arc::new(MemStore::seeded(record_with_assistant(&company)));
    let tool = CreateWorkflowTool::new(company, None, store, None, WorkflowRefQueue::default());
    let result = tool
        .execute(json!({
            "id": "toobig",
            "name": "TooBig",
            "nodes": [
                { "id": "start", "kind": "trigger", "name": "Start" },
                {
                    "id": "grab",
                    "kind": "tool_call",
                    "name": "Grab",
                    "config": { "slug": "web_fetch", "args": { "n": 18446744073709551615u64 } }
                },
                { "id": "done", "kind": "output", "name": "Report" }
            ],
            "edges": [
                { "from": "start", "to": "grab" },
                { "from": "grab", "to": "done" }
            ]
        }))
        .await
        .expect("execute");
    assert!(result.is_error, "{result:?}");
    let msg = result.output_for_llm(false);
    assert!(
        msg.contains("can't be stored"),
        "names the failure: {result:?}"
    );
    assert!(
        !msg.contains("TOML has no null"),
        "a non-null conversion failure must not misdirect to the null remedy: {result:?}"
    );
}

#[test]
fn first_expression_location_walks_nested_config() {
    // Matches tinyflows' `is_expression`: a leading `=` (no trim).
    assert_eq!(
        first_expression_location(&json!({ "args": { "command": "=item.x" } }), ""),
        Some("args.command".to_string())
    );
    // Array elements become numeric segments.
    assert_eq!(
        first_expression_location(&json!({ "args": { "cc": ["a", "=item.y"] } }), ""),
        Some("args.cc.1".to_string())
    );
    // Literals — including a `=` in the MIDDLE — are not expressions.
    assert_eq!(
        first_expression_location(&json!({ "args": { "q": "a=b", "s": "ls -la" } }), ""),
        None
    );
}

#[test]
fn json_contains_null_is_recursive() {
    assert!(json_contains_null(&json!({ "args": { "url": null } })));
    assert!(json_contains_null(&json!(["ok", [null]])));
    assert!(!json_contains_null(
        &json!({ "args": { "url": "https://x" } })
    ));
}

// ---- read_run_output (issue #418) ----

/// Builds a `RunWorkflowTool` over the demo graph in `dir`, a stub runner
/// returning `run`, and the given caches — the shared setup the round-trip
/// tests need.
/// Returns the tool **and** the runner `Arc` — the handle keeps only a weak
/// reference, so the caller must hold the returned runner alive for the
/// duration of the test or the run tool reports "no runner wired".
fn run_tool_over(
    dir: &std::path::Path,
    run: WorkflowRun,
    refs: WorkflowRefQueue,
    cache: RunOutputCache,
) -> (RunWorkflowTool, Arc<dyn WorkflowRunner>) {
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(run));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs,
        cache,
        None,
    );
    (tool, runner)
}

/// T1: a clipped preview names exactly how many characters it dropped, and
/// counts them in `chars()` — so a multibyte string past the boundary
/// reports codepoints dropped, never bytes, and never panics on a byte
/// index that lands mid-character.
#[test]
fn preview_marks_the_exact_dropped_char_count_including_multibyte() {
    // 130 ASCII chars → 120 kept, 10 dropped.
    let ascii = "a".repeat(130);
    let preview = preview_item(&json!(ascii));
    assert!(preview.ends_with("… (+10 chars)"), "{preview}");
    assert!(preview.starts_with(&"a".repeat(120)), "{preview}");
    // 120 kept chars, then the '…' and the marker — the kept body is exactly
    // the cap, not one over.
    assert_eq!(preview.chars().take_while(|c| *c == 'a').count(), 120);

    // A multibyte fill: 130 'é' (2 bytes each). The marker must count the 10
    // dropped *characters*, not their 20 bytes, and the boundary must not
    // split a codepoint.
    let multibyte = "é".repeat(130);
    let preview = preview_item(&json!(multibyte));
    assert!(preview.ends_with("… (+10 chars)"), "{preview}");
    assert_eq!(preview.chars().take_while(|c| *c == 'é').count(), 120);

    // At or below the cap there is no marker at all.
    let short = "x".repeat(ITEM_PREVIEW_CHARS);
    assert_eq!(preview_item(&json!(short)), short);
}

/// T2: a node with more than one item is labelled `last of N items`, and the
/// summary footer names the companion tool via the `READ_RUN_OUTPUT_TOOL`
/// const (so wording can't drift) and embeds the run id.
#[test]
fn summary_labels_multi_item_nodes_and_footers_the_companion() {
    let file = crate::company::parse_workflow(DEMO_WF).unwrap();
    let run = WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["first", "second", "third"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let md = summarize_run(&file, &run, "run-xyz", RunOutputStored::Stored);
    assert!(md.contains("last of 3 items — third"), "{md}");
    assert!(md.contains(READ_RUN_OUTPUT_TOOL), "{md}");
    assert!(md.contains("run-xyz"), "{md}");

    // A single-item node keeps the plain "1 item(s)" phrasing.
    let run_one = WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["only"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let md = summarize_run(&file, &run_one, "run-1", RunOutputStored::Stored);
    assert!(md.contains("1 item(s) — only"), "{md}");
    assert!(!md.contains("last of"), "{md}");

    // The oversized footer sends the agent to the console run drawer instead
    // of to a `read_run_output` call that would find nothing cached.
    let md = summarize_run(
        &file,
        &run,
        "run-big",
        RunOutputStored::Oversized { bytes: 999 },
    );
    assert!(md.contains("console"), "{md}");
    assert!(md.contains("run drawer"), "{md}");
    assert!(md.contains("999 bytes"), "{md}");
    assert!(!md.contains("Read any node's full output"), "{md}");
}

/// Issue #981 (part 2): the summary says a report did not go out.
///
/// Before this, `summarize_run` never read `deliveries`, so a run whose
/// report was refused closed with "The run reached its terminal node(s)
/// without pausing for approval" and nothing else — a true sentence about a
/// run that had just dropped its only output, which the model then reported
/// upward as a clean run.
#[test]
fn the_summary_says_when_a_report_did_not_go_out() {
    let file = crate::company::parse_workflow(DEMO_WF).unwrap();
    let dropped = WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["the report"] } } }),
        pending_approvals: Vec::new(),
        deliveries: vec![crate::ports::DeliveryReport {
            node: "worker".into(),
            kind: "channel".into(),
            target: Some("operator".into()),
            status: crate::ports::DeliveryStatus::Failed,
            detail: "`operator` is not an automation delivery channel — this runtime has:                          engineering"
                .into(),
            reason: crate::ports::DeliveryReason::ChannelNotWired,
        }],
        cancelled: false,
        nodes: Vec::new(),
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let md = summarize_run(&file, &dropped, "run-drop", RunOutputStored::Stored);
    assert!(
        md.contains("1 report(s) did NOT reach a destination"),
        "{md}"
    );
    assert!(md.contains("`worker` (channel)"), "{md}");
    // The reason, from the closed set — never `detail`, which quotes what a
    // transport said and is for the operator's own surfaces (issue #248).
    assert!(
        md.contains(&crate::ports::DeliveryReason::ChannelNotWired.to_string()),
        "{md}"
    );
    assert!(
        !md.contains("this runtime has: engineering"),
        "the operator-only `detail` must not ride the summary: {md}"
    );
    // And it does not claim the graph broke: the per-node line still reports
    // what the node produced.
    assert!(md.contains("1 item(s) — the report"), "{md}");

    // A run that delivered fine says nothing about delivery at all, so an
    // ordinary summary is unchanged.
    let clean = WorkflowRun {
        deliveries: vec![crate::ports::DeliveryReport {
            node: "worker".into(),
            kind: "owner".into(),
            target: Some("ada@example.com".into()),
            status: crate::ports::DeliveryStatus::Sent,
            detail: "emailed the company's admin".into(),
            reason: crate::ports::DeliveryReason::OwnerEmailed,
        }],
        ..dropped.clone()
    };
    let md = summarize_run(&file, &clean, "run-ok", RunOutputStored::Stored);
    assert!(!md.contains("did NOT reach a destination"), "{md}");

    // A report parked for an operator's approval is waiting on a person,
    // not lost — counting it here would tell the model to go fix a queue
    // that is working.
    let parked = WorkflowRun {
        deliveries: vec![crate::ports::DeliveryReport {
            node: "worker".into(),
            kind: "email".into(),
            target: Some("new@example.com".into()),
            status: crate::ports::DeliveryStatus::Pending,
            detail: "waiting in Approvals".into(),
            reason: crate::ports::DeliveryReason::ParkedForApproval,
        }],
        ..dropped.clone()
    };
    let md = summarize_run(&file, &parked, "run-parked", RunOutputStored::Stored);
    assert!(!md.contains("did NOT reach a destination"), "{md}");

    // Issue #981, the second half. This paragraph's own prose is the
    // argument: it says the report "did not go out, and it will not without
    // a change". Neither is true of a test run, which attempted nothing on
    // purpose, nor of a continuation whose report an earlier run in the
    // lineage already sent — so telling the model to "fix the destination"
    // for either would send it at a graph that is behaving as designed.
    for reason in [
        crate::ports::DeliveryReason::DryRun,
        crate::ports::DeliveryReason::AlreadyDelivered,
    ] {
        let accounted = WorkflowRun {
            deliveries: vec![crate::ports::DeliveryReport {
                node: "worker".into(),
                kind: "channel".into(),
                target: Some("engineering".into()),
                status: crate::ports::DeliveryStatus::Skipped,
                detail: "nothing was sent".into(),
                reason,
            }],
            ..dropped.clone()
        };
        let md = summarize_run(&file, &accounted, "run-skip", RunOutputStored::Stored);
        assert!(
            !md.contains("did NOT reach a destination"),
            "{reason:?}: {md}"
        );
    }

    // The deliberate non-move: an `output` node with nowhere to send DID
    // lose its report, and the model is exactly the reader that should be
    // told (issues #925 / #947 / #963).
    let nowhere = WorkflowRun {
        deliveries: vec![crate::ports::DeliveryReport {
            node: "worker".into(),
            kind: "none".into(),
            target: None,
            status: crate::ports::DeliveryStatus::Skipped,
            detail: "this output node has no destination".into(),
            reason: crate::ports::DeliveryReason::NoDestinationConfigured,
        }],
        ..dropped.clone()
    };
    let md = summarize_run(&file, &nowhere, "run-nowhere", RunOutputStored::Stored);
    assert!(
        md.contains("1 report(s) did NOT reach a destination"),
        "{md}"
    );
}

/// Codex review on #1990 (#3905407434): a `halt_benign` judge verdict
/// scrubs the declined node from `run.output`, so this summary — which
/// derives its per-node lines from that map and separately inspects only
/// `Error` rows — called the node "not reached" and still claimed the run
/// reached its terminal nodes. The intentional stop was invisible to the
/// agent that started the run.
#[test]
fn the_summary_reports_a_declined_node_as_not_needed() {
    let file = crate::company::parse_workflow(DEMO_WF).unwrap();
    let declined = WorkflowRun {
        output: json!({ "nodes": { "start": { "items": ["go"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: vec![crate::ports::WorkflowRunNodeRow {
            node_id: "worker".into(),
            status: WorkflowNodeStatus::Declined,
            elapsed_ms: 12,
            diagnostics: Vec::new(),
        }],
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let md = summarize_run(&file, &declined, "run-declined", RunOutputStored::Stored);
    assert!(
        md.contains("not needed"),
        "a declined node must read as an intentional stop: {md}"
    );
    assert!(
        !md.contains("**Worker** (`worker`, agent): not reached"),
        "a declined node is not an unreached one: {md}"
    );
    assert!(
        !md.contains("reached its terminal node(s) without pausing"),
        "the run stopped on purpose; the happy-path sentence is false: {md}"
    );
    assert!(
        !md.contains("NOT a clean run"),
        "a declined node is not an error: {md}"
    );
}

/// Codex (PR #1883 review comment 3892522591): a node under `on_error =
/// "continue"`/`"route"` settles the run `Degraded`, and `runner.rs`
/// already writes a per-node notice for it — but `summarize_run` never
/// read `run.nodes`, so an agent-started run through this exact case
/// summarized as "reached its terminal node(s) without pausing for
/// approval" with no hint anything went wrong. This pins the fix: a row
/// still `Error` after settle must show up in the tool result.
#[test]
fn the_summary_says_when_a_node_errored_and_the_run_continued() {
    let file = crate::company::parse_workflow(DEMO_WF).unwrap();
    let degraded = WorkflowRun {
        output: json!({ "nodes": { "worker": { "items": ["partial"] } } }),
        pending_approvals: Vec::new(),
        deliveries: Vec::new(),
        cancelled: false,
        nodes: vec![crate::ports::WorkflowRunNodeRow {
            node_id: "worker".into(),
            status: WorkflowNodeStatus::Error,
            elapsed_ms: 12,
            diagnostics: Vec::new(),
        }],
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    };
    let md = summarize_run(&file, &degraded, "run-degraded", RunOutputStored::Stored);
    assert!(
        md.contains("did not finish cleanly, and the run continued past it"),
        "{md}"
    );
    assert!(md.contains("worker"), "{md}");
    assert!(
        md.contains("NOT a clean run"),
        "an agent skimming for the happy-path sentence must not miss this: {md}"
    );

    // A node that finished clean says nothing about it — an ordinary
    // summary is unchanged.
    let clean = WorkflowRun {
        nodes: vec![crate::ports::WorkflowRunNodeRow {
            node_id: "worker".into(),
            status: WorkflowNodeStatus::Ok,
            elapsed_ms: 12,
            diagnostics: Vec::new(),
        }],
        ..degraded.clone()
    };
    let md = summarize_run(&file, &clean, "run-clean", RunOutputStored::Stored);
    assert!(!md.contains("did not finish cleanly"), "{md}");
    assert!(!md.contains("NOT a clean run"), "{md}");

    // A blocked node must not ALSO print here — it is already named by the
    // "Blocked, waiting on a person" paragraph above, sourced from
    // `blocked_nodes`, not from a node row's own status (the host never
    // leaves a blocked row `Error`; see `WorkflowNodeStatus::Blocked`'s doc).
    let blocked = WorkflowRun {
        nodes: vec![crate::ports::WorkflowRunNodeRow {
            node_id: "worker".into(),
            status: WorkflowNodeStatus::Blocked,
            elapsed_ms: 12,
            diagnostics: Vec::new(),
        }],
        blocked_nodes: vec![crate::ports::WorkflowBlockedNode {
            node_id: "worker".into(),
            tools: vec!["send_email".into()],
            approval_ids: vec!["appr-1".into()],
            unparkable: 0,
            stranded: 0,
            blockers: 0,
        }],
        ..degraded.clone()
    };
    let md = summarize_run(&file, &blocked, "run-blocked", RunOutputStored::Stored);
    assert!(md.contains("Blocked, waiting on a person"), "{md}");
    assert!(
        !md.contains("did not finish cleanly"),
        "a blocked row must not double up with the degraded paragraph: {md}"
    );
}

/// T3: a successful run populates the cache and the tool's JSON payload
/// carries the run id; a cancelled or failed run stores nothing.
#[tokio::test]
async fn success_populates_cache_and_payload_carries_run_id() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let cache = RunOutputCache::default();
    let (ok, _runner) = run_tool_over(
        dir.path(),
        WorkflowRun {
            output: json!({ "nodes": { "worker": { "items": ["did the thing"] } } }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: false,
            nodes: Vec::new(),
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        },
        WorkflowRefQueue::default(),
        cache.clone(),
    );
    let result = ok.execute(json!({ "id": "demo" })).await.expect("execute");
    assert!(!result.is_error, "{result:?}");
    assert_eq!(cache.len(), 1, "a successful run must be cached");
    // The payload the brain sees carries a run id string.
    let payload = match &result.content[0] {
        openhuman_core::skills::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected JSON payload, got {other:?}"),
    };
    assert!(
        payload.get("run_id").and_then(Value::as_str).is_some(),
        "{payload}"
    );

    // A cancelled run caches nothing.
    let cancel_cache = RunOutputCache::default();
    let (cancelled, _cancel_runner) = run_tool_over(
        dir.path(),
        WorkflowRun {
            output: json!({ "nodes": {} }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: true,
            nodes: Vec::new(),
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        },
        WorkflowRefQueue::default(),
        cancel_cache.clone(),
    );
    assert!(
        cancelled
            .execute(json!({ "id": "demo" }))
            .await
            .expect("execute")
            .is_error
    );
    assert_eq!(cancel_cache.len(), 0, "a cancelled run stores nothing");
}

/// T4: the full agent round-trip. A run whose node holds multiple >120-char
/// items is summarised (clipping the preview), then `read_run_output` over
/// the shared cache returns every item — including every character the
/// preview dropped — verbatim.
#[tokio::test]
async fn read_run_output_returns_every_dropped_char_after_a_run() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let long_a = "A".repeat(400);
    let long_b = format!("B{}", "b".repeat(500));
    let cache = RunOutputCache::default();
    let (run, _runner) = run_tool_over(
        dir.path(),
        WorkflowRun {
            output: json!({ "nodes": { "worker": { "items": [long_a, long_b] } } }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: false,
            nodes: Vec::new(),
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        },
        WorkflowRefQueue::default(),
        cache.clone(),
    );
    let summary = run.execute(json!({ "id": "demo" })).await.expect("execute");
    // The summary only previews the last item, clipped.
    assert!(
        summary.output_for_llm(true).contains("last of 2 items"),
        "{summary:?}"
    );

    let reader = ReadRunOutputTool::new(CompanyId::new("acme"), cache);
    // Pass the display name "Worker" — the case-insensitive fallback resolves
    // it to id `worker`.
    let read = reader
        .execute(json!({ "run_id": "", "node": "Worker" }))
        .await
        .expect("execute");
    // Empty run_id is rejected before lookup.
    assert!(read.is_error, "empty run_id must be rejected");

    // Read with the real run id (recover it from the payload).
    let payload = match &summary.content[0] {
        openhuman_core::skills::types::ToolContent::Json { data } => data.clone(),
        other => panic!("{other:?}"),
    };
    let run_id = payload.get("run_id").and_then(Value::as_str).unwrap();
    let read = reader
        .execute(json!({ "run_id": run_id, "node": "Worker" }))
        .await
        .expect("execute");
    assert!(!read.is_error, "{read:?}");
    let text = read.output_for_llm(false);
    assert!(text.contains("Item 1 of 2:"), "{text}");
    assert!(text.contains("Item 2 of 2:"), "{text}");
    assert!(text.contains(&"A".repeat(400)), "item 1 must be verbatim");
    assert!(text.contains(&"b".repeat(500)), "item 2 must be verbatim");
}

/// T4b: the run summary lists a node by its display name but the cache is
/// keyed by id, and in `DEMO_WF` the two genuinely differ — `id = "done"`,
/// `name = "Report"`. Both the display name the summary prints ("Report")
/// and the raw id ("done") must resolve through `read_run_output`. This is
/// the non-degenerate name/id pair the case-only fallback never covered.
#[tokio::test]
async fn read_run_output_resolves_display_name_and_id() {
    let dir = tempfile::tempdir().unwrap();
    seed_demo_workflow(dir.path());

    let cache = RunOutputCache::default();
    let (run, _runner) = run_tool_over(
        dir.path(),
        WorkflowRun {
            // The terminal node's id is `done`; the summary shows its name,
            // "Report".
            output: json!({ "nodes": { "done": { "items": ["the report body"] } } }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: false,
            nodes: Vec::new(),
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        },
        WorkflowRefQueue::default(),
        cache.clone(),
    );
    let summary = run.execute(json!({ "id": "demo" })).await.expect("execute");
    // The summary prints the display name and now the id alongside it.
    let md = summary.output_for_llm(true);
    assert!(md.contains("**Report**"), "{md}");
    assert!(md.contains("`done`"), "{md}");
    let run_id = match &summary.content[0] {
        openhuman_core::skills::types::ToolContent::Json { data } => data
            .get("run_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        other => panic!("{other:?}"),
    };
    let reader = ReadRunOutputTool::new(CompanyId::new("acme"), cache);

    // The display name from the summary resolves to id `done`.
    let by_name = reader
        .execute(json!({ "run_id": run_id, "node": "Report" }))
        .await
        .expect("execute");
    assert!(!by_name.is_error, "display name must resolve: {by_name:?}");
    assert!(
        by_name.output_for_llm(false).contains("the report body"),
        "{by_name:?}"
    );

    // The raw id resolves too, so both paths are live.
    let by_id = reader
        .execute(json!({ "run_id": run_id, "node": "done" }))
        .await
        .expect("execute");
    assert!(!by_id.is_error, "id must resolve: {by_id:?}");
    assert!(
        by_id.output_for_llm(false).contains("the report body"),
        "{by_id:?}"
    );
}

/// T5: paging. Two windows concatenate to the original, each page stays
/// within budget, and a boundary that lands on a multibyte char never
/// splits a codepoint.
#[test]
fn paging_reassembles_and_never_splits_a_codepoint() {
    // 300 'é' (2 bytes each) = 600 bytes. A 401-byte budget forces a break
    // right where the next 'é' would cross it — proving whole-char taking.
    let full: String = "é".repeat(300);
    let budget = 401;
    let (p1, next) = page_run_output(&full, 0, budget);
    let n1 = next.expect("more remains");
    assert!(p1.len() <= budget, "page 1 is {} bytes", p1.len());
    // A page of whole 'é' has even byte length — never an odd split.
    assert_eq!(p1.len() % 2, 0, "a split codepoint would make this odd");
    let (p2, next2) = page_run_output(&full, n1, budget);
    assert!(p2.len() <= budget, "page 2 is {} bytes", p2.len());
    // Continue to the end and prove the concatenation reconstructs the whole.
    let mut assembled = p1.clone();
    assembled.push_str(&p2);
    let mut off = next2;
    while let Some(o) = off {
        let (p, nxt) = page_run_output(&full, o, budget);
        assert!(p.len() <= budget);
        assembled.push_str(&p);
        off = nxt;
    }
    assert_eq!(assembled, full, "the pages must reassemble the original");

    // Every char is valid UTF-8 by construction (String), so decoding the
    // reassembly back is lossless.
    assert_eq!(assembled.chars().count(), 300);
}

/// T5b: the tool's own paging clips a huge single item under the budget and
/// hands back an offset that reads the remainder.
#[tokio::test]
async fn read_run_output_pages_a_huge_item_under_budget() {
    let dir = tempfile::tempdir().unwrap();
    // One item far larger than the 16 KiB tool-result budget.
    let huge = "z".repeat(40_000);
    let cache = RunOutputCache::default();
    cache.store(
        "run-huge",
        "demo",
        json!({ "worker": { "items": [huge] } }),
        Vec::new(),
    );
    let _ = dir;
    let reader = ReadRunOutputTool::new(CompanyId::new("acme"), cache);

    let first = reader
        .execute(json!({ "run_id": "run-huge", "node": "worker" }))
        .await
        .expect("execute");
    assert!(!first.is_error, "{first:?}");
    let text = first.output_for_llm(false);
    assert!(
        text.len() <= crate::harness::build::TOOL_RESULT_BUDGET_BYTES,
        "page too big"
    );
    assert!(text.contains("Continue with offset="), "{text}");
    // Pull the offset out and read the next page.
    let off: usize = text
        .rsplit("offset=")
        .next()
        .and_then(|t| t.trim_end_matches('.').parse().ok())
        .expect("an offset to continue from");
    let second = reader
        .execute(json!({ "run_id": "run-huge", "node": "worker", "offset": off }))
        .await
        .expect("execute");
    assert!(!second.is_error, "{second:?}");
    assert!(
        off > 0 && off < 40_100,
        "offset {off} advances into the item"
    );
}

/// T6: the error arms are actionable — unknown run names the console
/// fallback, unknown node lists the valid ids with item counts, and an empty
/// node says so rather than returning nothing.
#[tokio::test]
async fn read_run_output_error_arms_are_actionable() {
    let cache = RunOutputCache::default();
    cache.store(
        "run-1",
        "demo",
        json!({
            "worker": { "items": ["one", "two"] },
            "done": { "items": [] }
        }),
        Vec::new(),
    );
    let reader = ReadRunOutputTool::new(CompanyId::new("acme"), cache);

    // Unknown run → names the cache scope + the console fallback.
    let unknown_run = reader
        .execute(json!({ "run_id": "ghost", "node": "worker" }))
        .await
        .expect("execute");
    assert!(unknown_run.is_error);
    let t = unknown_run.output_for_llm(false);
    assert!(t.contains("console"), "{t}");

    // Unknown node → lists valid ids + counts.
    let unknown_node = reader
        .execute(json!({ "run_id": "run-1", "node": "nope" }))
        .await
        .expect("execute");
    assert!(unknown_node.is_error);
    let t = unknown_node.output_for_llm(false);
    assert!(t.contains("`worker` (2 item(s))"), "{t}");
    assert!(t.contains("`done` (0 item(s))"), "{t}");

    // Empty node → a success that says it is empty, not an error, not silence.
    let empty = reader
        .execute(json!({ "run_id": "run-1", "node": "done" }))
        .await
        .expect("execute");
    assert!(!empty.is_error, "{empty:?}");
    assert!(
        empty.output_for_llm(false).contains("no items"),
        "{empty:?}"
    );
}

/// T7: eviction (oldest run drops past the run-count bound) and the
/// oversized-run announce (a run over the hard per-run ceiling is refused,
/// reported as `Oversized`, and never cached).
#[test]
fn cache_evicts_oldest_and_refuses_an_oversized_run() {
    let cache = RunOutputCache::default();
    for i in 0..(RUN_OUTPUT_CACHE_RUNS + 3) {
        let outcome = cache.store(
            &format!("run-{i}"),
            "demo",
            json!({ "worker": { "items": [format!("item-{i}")] } }),
            Vec::new(),
        );
        assert!(matches!(outcome, RunOutputStored::Stored));
    }
    assert_eq!(cache.len(), RUN_OUTPUT_CACHE_RUNS, "bounded to the run cap");
    // The three oldest runs were evicted; the newest survive.
    assert!(cache.get("run-0").is_none(), "oldest must be evicted");
    assert!(
        cache
            .get(&format!("run-{}", RUN_OUTPUT_CACHE_RUNS + 2))
            .is_some(),
        "newest must survive"
    );

    // A run whose node map serializes past the hard per-run ceiling is
    // refused, announced (not silently dropped), and never cached.
    let fresh = RunOutputCache::default();
    let giant = "g".repeat(RUN_OUTPUT_ENTRY_MAX_BYTES + 1);
    let outcome = fresh.store(
        "run-giant",
        "demo",
        json!({ "worker": { "items": [giant] } }),
        Vec::new(),
    );
    assert!(
        matches!(outcome, RunOutputStored::Oversized { .. }),
        "must refuse"
    );
    assert_eq!(fresh.len(), 0, "an oversized run must not be cached");
    assert!(fresh.get("run-giant").is_none());
}

// -----------------------------------------------------------------------
// Issue #661: the queue is scoped per claimant
// -----------------------------------------------------------------------

/// A card, titled so a drain can be identified by what it carried.
fn card(title: &str) -> Delegation {
    Delegation::SpawnTask {
        title: title.to_string(),
        note: None,
        assignee: None,
    }
}

fn hand_off() -> Delegation {
    Delegation::DelegateToDesk {
        desk: "design".to_string(),
        instruction: "have a look".to_string(),
    }
}

fn titles(drained: Vec<Delegation>) -> Vec<String> {
    drained
        .into_iter()
        .map(|d| match d {
            Delegation::SpawnTask { title, .. } => title,
            other => panic!("expected a card, got {other:?}"),
        })
        .collect()
}

fn stage(queue: &DelegationQueue, d: Delegation) -> Staged {
    queue.push_within_cap(d, MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND)
}

/// **The regression this whole change exists for.**
///
/// A workflow run taking a claim while the chat cycle has work staged must
/// leave that work alone. Before the scoping, `claim_as` opened with a
/// global `clear()`, so this exact interleaving destroyed a chat turn's
/// staged card and its refusal — and the turn had already told the operator
/// the card was opened.
///
/// Runs are `tokio::spawn`ed and are not under the cycle lock (#401 allows
/// several at once), so this interleaving is reachable rather than
/// theoretical.
#[tokio::test]
async fn a_workflow_claim_leaves_a_concurrent_chat_turns_staged_work_intact() {
    let queue = DelegationQueue::default();

    // A chat turn is mid-flight with a card staged and a refusal recorded.
    let _chat = queue.claim();
    assert_eq!(stage(&queue, card("chat")), Staged::Queued);
    queue.push_refusal("nonexistent-desk".to_string());

    // A workflow run claims, concurrently. This is the moment that used to
    // wipe the chat's bucket.
    let run = queue.claim_board("run-1");
    run.scoped(async { assert_eq!(stage(&queue, card("run")), Staged::Queued) })
        .await;

    // The chat's staged card and refusal are both still there…
    assert_eq!(queue.queued(), 1);
    assert_eq!(queue.refusals_queued(), 1);
    assert_eq!(titles(queue.drain(MAX_DELEGATIONS_PER_TURN)), ["chat"]);
    assert_eq!(
        queue.drain_refusals(MAX_DELEGATIONS_PER_TURN),
        ["nonexistent-desk"]
    );

    // …and the run still has its own, drained separately.
    let run_drained = run
        .scoped(async { queue.drain(MAX_DELEGATIONS_PER_TURN) })
        .await;
    assert_eq!(titles(run_drained), ["run"]);
}

/// The half of the defect that is **live today**, with no drain wired and
/// nothing else changed.
///
/// `DelegateToDeskTool` calls [`DelegationQueue::push_refusal`] on the
/// ungrounded path *before* it consults the claim, so an invented desk named
/// by a workflow node already reaches the shared vector. A chat turn's
/// `drain_refusals` would then take it, record it on that turn's card, and
/// clear it — a hand-off nobody on that turn attempted.
#[tokio::test]
async fn a_runs_ungrounded_hand_off_is_not_recorded_on_a_chat_turns_card() {
    let queue = DelegationQueue::default();
    let _chat = queue.claim();

    let run = queue.claim_board("run-1");
    run.scoped(async { queue.push_refusal("marketing".to_string()) })
        .await;

    // Nothing to report on the chat turn's card: it attempted no hand-off.
    assert_eq!(queue.refusals_queued(), 0);
    assert!(queue.drain_refusals(MAX_DELEGATIONS_PER_TURN).is_empty());

    // The run's own refusal is intact and still its own to read.
    let seen = run
        .scoped(async { queue.drain_refusals(MAX_DELEGATIONS_PER_TURN) })
        .await;
    assert_eq!(seen, ["marketing"]);
}

/// Two runs and the chat cycle interleaved: each sees only its own, and
/// neither draining nor claiming reaches across.
#[tokio::test]
async fn two_runs_and_the_chat_cycle_neither_drain_nor_clear_each_other() {
    let queue = DelegationQueue::default();

    let _chat = queue.claim();
    assert_eq!(stage(&queue, card("chat")), Staged::Queued);

    let run_a = queue.claim_board("run-a");
    run_a
        .scoped(async { assert_eq!(stage(&queue, card("a")), Staged::Queued) })
        .await;

    // B claims *after* A staged — the acquire-time clear must not reach A.
    let run_b = queue.claim_board("run-b");
    run_b
        .scoped(async { assert_eq!(stage(&queue, card("b")), Staged::Queued) })
        .await;

    assert_eq!(queue.queued(), 1, "the chat cycle sees only its own");
    assert_eq!(run_a.scoped(async { queue.queued() }).await, 1);
    assert_eq!(run_b.scoped(async { queue.queued() }).await, 1);

    // Draining A takes A's and only A's.
    let drained_a = run_a
        .scoped(async { queue.drain(MAX_DELEGATIONS_PER_TURN) })
        .await;
    assert_eq!(titles(drained_a), ["a"]);
    assert_eq!(queue.queued(), 1);
    assert_eq!(run_b.scoped(async { queue.queued() }).await, 1);

    assert_eq!(titles(queue.drain(MAX_DELEGATIONS_PER_TURN)), ["chat"]);
    let drained_b = run_b
        .scoped(async { queue.drain(MAX_DELEGATIONS_PER_TURN) })
        .await;
    assert_eq!(titles(drained_b), ["b"]);
}

/// A claim's `Drop` discards its own bucket and un-claims its own scope —
/// and reaches nothing else. A cancelled run's staged writes dying with the
/// run is the intended semantics; a chat turn's surviving it is the point.
#[tokio::test]
async fn dropping_a_claim_discards_only_its_own_bucket() {
    let queue = DelegationQueue::default();

    let _chat = queue.claim();
    assert_eq!(stage(&queue, card("chat")), Staged::Queued);

    {
        let run = queue.claim_board("run-1");
        run.scoped(async { assert_eq!(stage(&queue, card("run")), Staged::Queued) })
            .await;
        run.scoped(async { queue.push_refusal("ghost".to_string()) })
            .await;
    } // the run is cancelled here

    // Its bucket went with it, and its scope is claimable again from
    // scratch rather than left committed.
    let after = CURRENT_SCOPE
        .scope(DelegationScope::Run("run-1".to_string()), async {
            (queue.queued(), queue.refusals_queued(), queue.claim_state())
        })
        .await;
    assert_eq!(after, (0, 0, DrainClaim::Unclaimed));

    // The chat turn is untouched — still claimed, still holding its card.
    assert_eq!(queue.claim_state(), DrainClaim::Full);
    assert_eq!(titles(queue.drain(MAX_DELEGATIONS_PER_TURN)), ["chat"]);
}

/// The #176 scope chain is per claimant, and its depth accounting is
/// unchanged by that.
///
/// Depth is still exactly `chain.len()` and still gates a hand-off at the
/// bound; what it no longer does is count another claimant's nesting.
#[tokio::test]
async fn a_scope_chain_is_per_claimant_and_depth_is_unchanged() {
    let queue = DelegationQueue::default();
    let _chat = queue.claim();

    let _outer = queue.enter_scope("design".to_string());
    assert_eq!(queue.scope_depth(), 1);
    assert_eq!(queue.scope_chain(), ["design"]);

    let run = queue.claim_board("run-1");
    run.scoped(async {
        // A run opens its own chain at depth 0 however deep the chat is.
        assert_eq!(queue.scope_depth(), 0);
        assert!(queue.scope_chain().is_empty());

        let _a = queue.enter_scope("eng".to_string());
        let _b = queue.enter_scope("qa".to_string());
        assert_eq!(queue.scope_depth(), 2);
        assert_eq!(queue.scope_chain(), ["eng", "qa"]);
    })
    .await;

    // The chat's chain is exactly as deep as it was left, and its guard
    // popped from its own chain rather than the run's.
    assert_eq!(queue.scope_depth(), 1);
    assert_eq!(queue.scope_chain(), ["design"]);

    // Depth still gates at the bound, counting this claimant's chain only:
    // one level deep against a bound of 1 refuses…
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 1),
        Staged::NoDrain(NoDrainReason::Depth)
    );
    // …and against a bound of 2 it stages, which a run's two levels would
    // have blocked had they been counted here.
    assert_eq!(
        queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, 2),
        Staged::Queued
    );
}

/// The [`DrainClaim::Board`] permit matrix: both kinds a run may perform
/// stage, and both it may not are refused — each for its own reason.
#[tokio::test]
async fn a_board_claim_permits_cards_and_refuses_review_and_hand_off() {
    let queue = DelegationQueue::default();
    let run = queue.claim_board("run-1");

    run.scoped(async {
        assert_eq!(stage(&queue, card("open a card")), Staged::Queued);
        assert_eq!(
            stage(
                &queue,
                Delegation::AssignTask {
                    task_id: "t1".to_string(),
                    assignee: "design".to_string(),
                    note: None,
                }
            ),
            Staged::Queued
        );

        // Lifecycle is the operator's lane.
        assert_eq!(
            stage(
                &queue,
                Delegation::ReviewTask {
                    task_id: "t1".to_string(),
                    decision: ReviewDecision::Approve,
                    note: None,
                }
            ),
            Staged::NoDrain(NoDrainReason::WorkflowLifecycle)
        );
        // A hand-off has nowhere to put the reply it exists for.
        assert_eq!(
            stage(&queue, hand_off()),
            Staged::NoDrain(NoDrainReason::WorkflowHandOff)
        );
    })
    .await;
}

/// The refusal text is what a model reads and reacts to, so both wordings
/// have to name the real cause and what the run *can* do instead — and must
/// not be each other's.
#[test]
fn the_two_workflow_refusals_say_what_the_run_can_do_instead() {
    let lifecycle = no_drain(
        REVIEW_TASK_TOOL,
        "the card was NOT reviewed",
        NoDrainReason::WorkflowLifecycle,
    );
    assert!(
        lifecycle.contains("running inside a workflow"),
        "{lifecycle}"
    );
    assert!(lifecycle.contains("operator's call"), "{lifecycle}");
    assert!(
        lifecycle.contains("`spawn_task`") && lifecycle.contains("`assign_task`"),
        "it must name what the run can do instead: {lifecycle}"
    );
    assert!(
        !lifecycle.contains("no conversation"),
        "the lifecycle refusal must not borrow the hand-off's cause: {lifecycle}"
    );

    let hand_off = no_drain(
        DELEGATE_TO_DESK_TOOL,
        "nothing was handed to the design desk",
        NoDrainReason::WorkflowHandOff,
    );
    assert!(hand_off.contains("running inside a workflow"), "{hand_off}");
    assert!(hand_off.contains("no conversation"), "{hand_off}");
    assert!(
        hand_off.contains("`spawn_task`"),
        "it must name the durable alternative: {hand_off}"
    );
    assert!(
        !hand_off.contains("operator's call"),
        "the hand-off refusal must not borrow the lifecycle's cause: {hand_off}"
    );

    // Both keep the do-not-report-it-as-done half every refusal here needs.
    for text in [&lifecycle, &hand_off] {
        assert!(text.contains("Do not retry this call"), "{text}");
        assert!(text.contains("do NOT report"), "{text}");
    }

    // …and they stay countable apart in the logs, from each other and from
    // the three that came before.
    let labels = [
        NoDrainReason::Unwired,
        NoDrainReason::Triage,
        NoDrainReason::Depth,
        NoDrainReason::WorkflowLifecycle,
        NoDrainReason::WorkflowHandOff,
    ]
    .map(|r| r.as_str());
    let unique: std::collections::BTreeSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

// -----------------------------------------------------------------------
// Issue #1859: the execution-state read trio (`list_tasks` / `read_task` /
// `read_run`) and `query_company`'s `## Board` section.
// -----------------------------------------------------------------------

/// A minimal board card, for fixtures below. Named `task_card` rather than
/// `card` — that name is already the `Delegation` fixture above.
fn task_card(id: &str, title: &str, column: &str, assignee: &str) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        title: TaskTitle::authored(title),
        note: None,
        column: column.to_string(),
        priority: "medium".to_string(),
        assignee: assignee.to_string(),
        updated_at_millis: 1,
        origin: crate::ports::TaskOrigin::new(None, None),
        parent_task_id: None,
        output: None,
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: crate::ports::tasks::TaskDeliverable::Once,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
        origin_message_seq: None,
        bounced: None,
    }
}

#[tokio::test]
async fn list_tasks_filters_by_column_and_assignee_and_excludes_done_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs;
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Draft the memo",
                crate::ports::tasks::COLUMN_TODO,
                "maya",
            ),
        )
        .await
        .unwrap();
    tasks
        .upsert(
            &company,
            &task_card(
                "t-2",
                "Fix the flaky test",
                crate::ports::tasks::COLUMN_PAUSED,
                "engineer",
            ),
        )
        .await
        .unwrap();
    tasks
        .upsert(
            &company,
            &task_card("t-3", "Ship the release", COLUMN_DONE, "maya"),
        )
        .await
        .unwrap();

    let tool = ListTasksTool::new(company, Some(tasks), None);

    let default_view = tool.execute(json!({})).await.unwrap().output_for_llm(true);
    assert!(default_view.contains("Draft the memo"), "{default_view}");
    assert!(
        default_view.contains("Fix the flaky test"),
        "{default_view}"
    );
    assert!(
        !default_view.contains("Ship the release"),
        "done cards must be excluded by default: {default_view}"
    );

    let by_column = tool
        .execute(json!({ "column": "paused" }))
        .await
        .unwrap()
        .output_for_llm(true);
    assert!(by_column.contains("Fix the flaky test"), "{by_column}");
    assert!(!by_column.contains("Draft the memo"), "{by_column}");

    let by_assignee = tool
        .execute(json!({ "assignee": "MAYA" }))
        .await
        .unwrap()
        .output_for_llm(true);
    assert!(
        by_assignee.contains("Draft the memo"),
        "case-insensitive assignee match: {by_assignee}"
    );
    assert!(!by_assignee.contains("Fix the flaky test"), "{by_assignee}");

    let done_explicit = tool
        .execute(json!({ "column": "done" }))
        .await
        .unwrap()
        .output_for_llm(true);
    assert!(
        done_explicit.contains("Ship the release"),
        "an explicit `column: done` must still answer: {done_explicit}"
    );
}

#[tokio::test]
async fn list_tasks_truncates_with_an_honest_marker() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    for n in 0..(LIST_TASKS_LIMIT + 5) {
        tasks
            .upsert(
                &company,
                &task_card(
                    &format!("t-{n}"),
                    &format!("Card {n}"),
                    crate::ports::tasks::COLUMN_TODO,
                    "maya",
                ),
            )
            .await
            .unwrap();
    }

    let tool = ListTasksTool::new(company, Some(tasks), None);
    let out = tool.execute(json!({})).await.unwrap().output_for_llm(true);
    assert!(out.contains("TRUNCATED"), "{out}");
    assert!(out.contains("5 more card"), "{out}");
}

#[tokio::test]
async fn list_tasks_reports_unavailable_when_the_board_is_unwired() {
    let tool = ListTasksTool::new(CompanyId::new("acme"), None, None);
    let result = tool.execute(json!({})).await.unwrap();
    assert!(
        result.is_error,
        "no task board wired must be a refusal, not an empty board"
    );
    assert!(
        result.output_for_llm(true).contains("No task board wired"),
        "{:?}",
        result.output_for_llm(true)
    );
}

#[tokio::test]
async fn read_task_renders_header_every_attempt_and_falls_back_to_the_cards_output_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let runs: Arc<dyn RunStore> = fs;
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Investigate the outage",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "engineer",
    );
    card.note = Some("check the load balancer first".to_string());
    card.output = Some(crate::ports::tasks::TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-2".to_string(),
            attempt: Some(2),
        },
        at_millis: 5,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    });
    tasks.upsert(&company, &card).await.unwrap();

    let mut r1 = runs
        .create_run(
            &company,
            crate::ports::runs::NewRun::for_task("r-1", "t-1", "engineer"),
        )
        .await
        .unwrap();
    r1.status = RunStatus::Failed;
    r1.error = Some("timed out".to_string());
    runs.put_run(&company, &r1).await.unwrap();
    let mut r2 = runs
        .create_run(
            &company,
            crate::ports::runs::NewRun::for_task("r-2", "t-1", "engineer"),
        )
        .await
        .unwrap();
    r2.status = RunStatus::Succeeded;
    runs.put_run(&company, &r2).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), Some(runs), None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(out.contains("Investigate the outage"), "{out}");
    assert!(out.contains("check the load balancer first"), "{out}");
    assert!(out.contains("attempt 1"), "{out}");
    assert!(out.contains("attempt 2"), "{out}");
    assert!(out.contains("timed out"), "{out}");
    // No artifact store wired: falls back to the card's own recorded
    // output stamp rather than fabricating anything.
    assert!(out.contains("run `r-2`"), "{out}");
    assert!(out.contains("attempt 2)"), "{out}");
}

#[tokio::test]
async fn read_task_errors_on_an_unknown_id_instead_of_fabricating_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let tool = ReadTaskTool::new(CompanyId::new("acme"), Some(tasks), None, None);
    let result = tool.execute(json!({ "task_id": "nope" })).await.unwrap();
    assert!(result.is_error, "an unknown task_id must error");
    let text = result.output_for_llm(true);
    assert!(text.contains("nope"), "{text}");
    assert!(text.contains("list_tasks"), "{text}");
}

/// AUTH-axis (HT-072): `read_task` is company-scoped only through
/// `tasks.list(&self.company)` — a real backend (here `FsOps`, the
/// production store, not a hand-rolled fake) partitions its data by
/// company on disk, so a `task_id` that exists but is filed under a
/// DIFFERENT company must read as not-found, never leak.
#[tokio::test]
async fn read_task_never_leaks_a_task_id_belonging_to_another_company() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    tasks
        .upsert(
            &CompanyId::new("beta"),
            &task_card(
                "t-secret",
                "beta's confidential rollout plan",
                crate::ports::tasks::COLUMN_TODO,
                "",
            ),
        )
        .await
        .unwrap();

    let tool = ReadTaskTool::new(CompanyId::new("acme"), Some(tasks), None, None);
    let result = tool
        .execute(json!({ "task_id": "t-secret" }))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "acme asking about beta's task_id must read as not-found: {}",
        result.text()
    );
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("confidential rollout"),
        "must not render beta's card: {text}"
    );
}

/// Fail-closed by construction (issue #1859's approved redaction posture):
/// `read_task` never reads [`RunRecord::usage`], so a run's USD cost cannot
/// reach its rendering no matter what that run cost.
#[tokio::test]
async fn read_task_never_renders_a_runs_usd_cost() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let runs: Arc<dyn RunStore> = fs;
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Send the invoice",
                crate::ports::tasks::COLUMN_IN_PROGRESS,
                "finance",
            ),
        )
        .await
        .unwrap();
    let mut run = runs
        .create_run(
            &company,
            crate::ports::runs::NewRun::for_task("r-1", "t-1", "finance"),
        )
        .await
        .unwrap();
    run.status = RunStatus::Succeeded;
    run.usage = crate::ports::types::TokenUsage {
        input: 500,
        output: 200,
        cached_input: 0,
        cost_usd: 4.20,
    };
    runs.put_run(&company, &run).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), Some(runs), None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        !out.contains("4.2") && !out.to_lowercase().contains("cost") && !out.contains("usd"),
        "a run's USD cost must never reach read_task: {out}"
    );
}

#[tokio::test]
async fn read_run_reads_an_agent_attempt_row() {
    let dir = tempfile::tempdir().unwrap();
    let runs: Arc<dyn RunStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    let mut run = runs
        .create_run(
            &company,
            crate::ports::runs::NewRun::for_task("r-1", "t-1", "engineer"),
        )
        .await
        .unwrap();
    run.status = RunStatus::Failed;
    run.error = Some("connection refused".to_string());
    runs.put_run(&company, &run).await.unwrap();

    let tool = ReadRunTool::new(company, Some(runs), None);
    let out = tool
        .execute(json!({ "run_id": "r-1" }))
        .await
        .unwrap()
        .output_for_llm(true);
    assert!(out.contains("failed"), "{out}");
    assert!(out.contains("connection refused"), "{out}");
    assert!(out.contains("t-1"), "{out}");
}

/// The dual-source lookup's second half: no [`RunStore`] row named
/// `run_id`, so `read_run` folds it out of the journal via
/// [`crate::server::ops::workflows::fold_run_events`] instead — the same
/// fold the console's run-history route reads.
#[tokio::test]
async fn read_run_folds_a_workflow_run_out_of_the_journal_when_no_attempt_row_exists() {
    use crate::ports::types::{StoredEvent, WorkflowNodeStatus};
    use futures::stream::{self, BoxStream};

    struct FixedLog(Vec<StoredEvent>);

    #[async_trait]
    impl EventLog for FixedLog {
        async fn append(
            &self,
            _id: &CompanyId,
            _event: CompanyEvent,
        ) -> crate::Result<EventSeq> {
            unreachable!("read_run only reads")
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            seq: EventSeq,
            limit: usize,
        ) -> crate::Result<Vec<StoredEvent>> {
            Ok(self
                .0
                .iter()
                .filter(|e| e.seq.value() >= seq.value())
                .take(limit)
                .cloned()
                .collect())
        }
        fn subscribe(
            &self,
            _id: &CompanyId,
        ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    let company = CompanyId::new("acme");
    let history = vec![
        StoredEvent {
            seq: EventSeq::new(0),
            company: company.clone(),
            event: CompanyEvent::WorkflowRunStarted {
                workflow_id: "demo".to_string(),
                run_id: "wf-run-1".to_string(),
                scheduled: false,
                started_by: None,
                resume_semantic: None,
            },
            at_millis: 1,
        },
        StoredEvent {
            seq: EventSeq::new(1),
            company: company.clone(),
            event: CompanyEvent::WorkflowNodeFinished {
                workflow_id: "demo".to_string(),
                run_id: "wf-run-1".to_string(),
                node_id: "fetch".to_string(),
                status: WorkflowNodeStatus::Ok,
                elapsed_ms: 10,
                diagnostics: Vec::new(),
                agent_run_id: None,
            },
            at_millis: 2,
        },
        StoredEvent {
            seq: EventSeq::new(2),
            company: company.clone(),
            event: CompanyEvent::WorkflowRunFinished {
                workflow_id: "demo".to_string(),
                scheduled: false,
                run_id: Some("wf-run-1".to_string()),
                deliveries: Vec::new(),
                pending_approvals: vec!["gate-1".to_string()],
                error: None,
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            },
            at_millis: 3,
        },
    ];
    let events: Arc<dyn EventLog> = Arc::new(FixedLog(history));

    let tool = ReadRunTool::new(company, None, Some(events));
    let out = tool
        .execute(json!({ "run_id": "wf-run-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(out.contains("demo"), "{out}");
    assert!(out.contains("fetch"), "{out}");
    assert!(out.contains("1 pending approval"), "{out}");
    // Summarized, never dumped: no step trace, no node output/argument text
    // rides this fold in the first place (see `WorkflowNodeFinished`'s own
    // doc comment), so there is nothing here to assert absent beyond what
    // the fixture itself never supplied.
}

#[tokio::test]
async fn read_run_errors_on_an_id_that_is_neither_an_attempt_nor_a_workflow_run() {
    let dir = tempfile::tempdir().unwrap();
    let runs: Arc<dyn RunStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let tool = ReadRunTool::new(CompanyId::new("acme"), Some(runs), None);
    let result = tool.execute(json!({ "run_id": "nope" })).await.unwrap();
    assert!(result.is_error, "an unknown run_id must error");
    assert!(result.output_for_llm(true).contains("nope"));
}

#[tokio::test]
async fn query_company_board_section_groups_open_cards_by_column_and_omits_done() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Draft the memo",
                crate::ports::tasks::COLUMN_TODO,
                "maya",
            ),
        )
        .await
        .unwrap();
    tasks
        .upsert(
            &company,
            &task_card("t-2", "Ship the release", COLUMN_DONE, "maya"),
        )
        .await
        .unwrap();

    let tool = QueryCompanyTool::new(company, None, None, None, None, Some(tasks));
    let out = tool.execute(json!({})).await.unwrap().output_for_llm(true);

    assert!(out.contains("## Board"), "{out}");
    assert!(out.contains("Draft the memo"), "{out}");
    assert!(
        !out.contains("Ship the release"),
        "the Board section must exclude Done, like `list_tasks`: {out}"
    );
    // Desks stays present AND after Board never gets to run — Board is the
    // LAST section, so this just pins Desks is still there at all.
    assert!(out.contains("## Desks"), "{out}");
}

#[tokio::test]
async fn query_company_board_section_is_unavailable_when_the_board_is_unwired() {
    let tool = QueryCompanyTool::new(CompanyId::new("acme"), None, None, None, None, None);
    let out = tool.execute(json!({})).await.unwrap().output_for_llm(true);
    assert!(out.contains("## Board"), "{out}");
    assert!(out.contains("Board unavailable"), "{out}");
}

/// The ordering guarantee the byte-budget reasoning depends on: Board is
/// the LAST section, so a company with an oversized board can never push
/// the Desks list — which `delegate_to_desk` needs to ground a hand-off —
/// out of the tool result ahead of it.
#[tokio::test]
async fn query_company_desks_section_still_renders_after_the_board_section() {
    let tool = QueryCompanyTool::new(CompanyId::new("acme"), None, None, None, None, None);
    let out = tool.execute(json!({})).await.unwrap().output_for_llm(true);
    let desks_at = out.find("## Desks").expect("Desks section present");
    let board_at = out.find("## Board").expect("Board section present");
    assert!(
        board_at > desks_at,
        "Board must render after Desks, never before: {out}"
    );
}

/// A task board that cannot answer, so a read failure never collapses
/// into an empty or missing board.
struct BrokenTaskStore;

#[async_trait]
impl TaskStore for BrokenTaskStore {
    async fn list(&self, _company: &CompanyId) -> crate::Result<Vec<TaskRecord>> {
        Err(OpenCompanyError::Store(
            "simulated board read failure".into(),
        ))
    }
    async fn upsert(&self, _company: &CompanyId, _task: &TaskRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn update_if_column(
        &self,
        _company: &CompanyId,
        _task: &TaskRecord,
        _observed: &TaskRecord,
        _expected_column: &str,
    ) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
}

#[tokio::test]
async fn list_tasks_reports_a_read_failure_instead_of_an_empty_board() {
    let tasks: Arc<dyn TaskStore> = Arc::new(BrokenTaskStore);
    let tool = ListTasksTool::new(CompanyId::new("acme"), Some(tasks), None);
    let result = tool.execute(json!({})).await.unwrap();
    assert!(
        result.is_error,
        "a board read failure must be a refusal, not a silently empty board"
    );
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("No matching cards"),
        "must not claim the board is simply empty: {text}"
    );
    assert!(text.contains("Couldn't read the task board"), "{text}");
}

#[tokio::test]
async fn read_task_reports_a_read_failure_instead_of_a_missing_card() {
    let tasks: Arc<dyn TaskStore> = Arc::new(BrokenTaskStore);
    let tool = ReadTaskTool::new(CompanyId::new("acme"), Some(tasks), None, None);
    let result = tool.execute(json!({ "task_id": "t-1" })).await.unwrap();
    assert!(
        result.is_error,
        "a board read failure must be a refusal, not a fabricated missing-card error"
    );
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("No card `t-1`"),
        "must not claim the card doesn't exist when the board couldn't be read: {text}"
    );
    assert!(text.contains("Couldn't read the task board"), "{text}");
}

#[tokio::test]
async fn query_company_board_section_reports_unavailable_on_a_read_failure_not_empty() {
    let tasks: Arc<dyn TaskStore> = Arc::new(BrokenTaskStore);
    let tool =
        QueryCompanyTool::new(CompanyId::new("acme"), None, None, None, None, Some(tasks));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "the whole tool must still answer");
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("No open cards"),
        "must not claim the board is empty when it could not be read: {text}"
    );
    assert!(text.contains("Board unavailable"), "{text}");

    let payload = match &result.content[0] {
        openhuman_core::skills::types::ToolContent::Json { data } => data.clone(),
        other => panic!("expected a JSON content block, got {other:?}"),
    };
    assert_eq!(
        payload["board_open"], 0,
        "board_open must stay at zero on a read failure, not report a fabricated count: \
         {payload}"
    );
}

#[tokio::test]
async fn read_task_falls_back_to_the_output_stamp_when_the_artifact_store_is_wired_but_empty() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let artifacts: Arc<dyn ArtifactStore> = fs;
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Reply to the customer",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "engineer",
    );
    card.output = Some(TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-9".to_string(),
            attempt: Some(3),
        },
        at_millis: 5,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    });
    tasks.upsert(&company, &card).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, Some(artifacts));
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        out.contains("run `r-9`"),
        "an artifact store wired but empty must still surface the card's own output \
         stamp instead of claiming nothing published: {out}"
    );
    assert!(out.contains("attempt 3)"), "{out}");
    assert!(
        !out.contains("Nothing published yet"),
        "must not claim nothing happened when the card recorded an attempt: {out}"
    );
}

/// A run store that cannot answer, so a run-history read failure never
/// collapses into "no attempts" or a missing run — the same distinction
/// `list_tasks`/`read_task`'s board read already makes for [`TaskStore`].
struct BrokenRunStore;

#[async_trait]
impl RunStore for BrokenRunStore {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by these tests")
    }
    async fn get_run(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-store read failure".into(),
        ))
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-history read failure".into(),
        ))
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by these tests")
    }
}

#[tokio::test]
async fn read_task_reports_run_history_unavailable_instead_of_no_attempts_on_a_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Investigate the outage",
                crate::ports::tasks::COLUMN_IN_REVIEW,
                "engineer",
            ),
        )
        .await
        .unwrap();

    let runs: Arc<dyn RunStore> = Arc::new(BrokenRunStore);
    let tool = ReadTaskTool::new(company, Some(tasks), Some(runs), None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        !out.contains("No attempts yet"),
        "a run-history read failure must not look like a card nobody attempted: {out}"
    );
    assert!(out.contains("Run history unavailable"), "{out}");
}

#[tokio::test]
async fn list_tasks_reports_attempt_status_unavailable_on_a_run_history_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Draft the memo",
                crate::ports::tasks::COLUMN_TODO,
                "maya",
            ),
        )
        .await
        .unwrap();

    let runs: Arc<dyn RunStore> = Arc::new(BrokenRunStore);
    let tool = ListTasksTool::new(company, Some(tasks), Some(runs));
    let out = tool.execute(json!({})).await.unwrap().output_for_llm(true);

    assert!(
        out.contains("attempt status unavailable"),
        "a per-card run-history read failure must not render identically to a card with \
         no attempt clause at all: {out}"
    );
}

/// A run store that answers `get_run` but never `list_runs`, to isolate
/// [`ReadRunTool`]'s agent-attempt lookup from its journal fallback.
struct FailingGetRun;

#[async_trait]
impl RunStore for FailingGetRun {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by these tests")
    }
    async fn get_run(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-store read failure".into(),
        ))
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        unimplemented!("not exercised by these tests")
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by these tests")
    }
}

#[tokio::test]
async fn read_run_reports_a_run_store_failure_instead_of_a_missing_run() {
    let runs: Arc<dyn RunStore> = Arc::new(FailingGetRun);
    let tool = ReadRunTool::new(CompanyId::new("acme"), Some(runs), None);
    let result = tool.execute(json!({ "run_id": "r-1" })).await.unwrap();
    assert!(
        result.is_error,
        "a run-store read failure must be a refusal, not a fabricated miss"
    );
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("No run"),
        "must not claim the run doesn't exist when the run store couldn't be read: {text}"
    );
}

/// An event log that always fails `read_from`, to prove
/// [`ReadRunTool`]'s workflow-run fallback distinguishes a journal read
/// failure from a genuinely absent run.
struct BrokenEventLog;

#[async_trait]
impl EventLog for BrokenEventLog {
    async fn append(&self, _id: &CompanyId, _event: CompanyEvent) -> crate::Result<EventSeq> {
        unreachable!("read_run only reads")
    }
    async fn read_from(
        &self,
        _id: &CompanyId,
        _seq: EventSeq,
        _limit: usize,
    ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
        Err(OpenCompanyError::Store(
            "simulated event-log read failure".into(),
        ))
    }
    fn subscribe(
        &self,
        _id: &CompanyId,
    ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem> {
        Box::pin(futures::stream::empty())
    }
}

#[tokio::test]
async fn read_run_reports_an_event_log_failure_instead_of_a_missing_run() {
    let events: Arc<dyn EventLog> = Arc::new(BrokenEventLog);
    let tool = ReadRunTool::new(CompanyId::new("acme"), None, Some(events));
    let result = tool.execute(json!({ "run_id": "wf-1" })).await.unwrap();
    assert!(
        result.is_error,
        "an event-log read failure must be a refusal, not a fabricated miss"
    );
    let text = result.output_for_llm(true);
    assert!(
        !text.contains("not an agent attempt and not a workflow run"),
        "must not claim the run doesn't exist when the event log couldn't be read: {text}"
    );
}

/// An artifact store that cannot answer, so an output-surface read
/// failure never collapses into "nothing published".
struct BrokenArtifactStore;

#[async_trait]
impl ArtifactStore for BrokenArtifactStore {
    async fn list(
        &self,
        _company: &CompanyId,
        _task_id: Option<&str>,
    ) -> crate::Result<Vec<crate::ports::artifacts::ArtifactRecord>> {
        Err(OpenCompanyError::Store(
            "simulated artifact-store read failure".into(),
        ))
    }
    async fn get(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<crate::ports::artifacts::ArtifactRecord>> {
        unimplemented!("not exercised by these tests")
    }
    async fn upsert(
        &self,
        _company: &CompanyId,
        _artifact: &crate::ports::artifacts::ArtifactRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
}

#[tokio::test]
async fn read_task_reports_output_unavailable_on_an_artifact_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Reply to the customer",
                crate::ports::tasks::COLUMN_IN_REVIEW,
                "engineer",
            ),
        )
        .await
        .unwrap();

    let artifacts: Arc<dyn ArtifactStore> = Arc::new(BrokenArtifactStore);
    let tool = ReadTaskTool::new(company, Some(tasks), None, Some(artifacts));
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        !out.contains("Nothing published yet"),
        "an artifact-store read failure must not look like a genuinely empty store: {out}"
    );
}

#[tokio::test]
async fn read_task_includes_each_attempts_run_id_so_read_run_is_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let runs: Arc<dyn RunStore> = fs;
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Investigate the outage",
                crate::ports::tasks::COLUMN_IN_REVIEW,
                "engineer",
            ),
        )
        .await
        .unwrap();
    let mut run = runs
        .create_run(
            &company,
            crate::ports::runs::NewRun::for_task("r-1", "t-1", "engineer"),
        )
        .await
        .unwrap();
    run.status = RunStatus::Failed;
    run.error = Some("timed out".to_string());
    runs.put_run(&company, &run).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), Some(runs), None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        out.contains("r-1"),
        "an attempt's run id must be discoverable from read_task, since read_run requires \
         it: {out}"
    );
}

#[tokio::test]
async fn read_task_bounds_rendered_attempts_so_output_cannot_be_pushed_out_of_budget() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let runs: Arc<dyn RunStore> = fs;
    let company = CompanyId::new("acme");
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                "Flaky deploy",
                crate::ports::tasks::COLUMN_IN_REVIEW,
                "engineer",
            ),
        )
        .await
        .unwrap();
    for n in 1..=(READ_TASK_ATTEMPTS_LIMIT + 3) {
        let mut run = runs
            .create_run(
                &company,
                crate::ports::runs::NewRun::for_task(format!("r-{n}"), "t-1", "engineer"),
            )
            .await
            .unwrap();
        run.status = RunStatus::Failed;
        run.error = Some("boom".to_string());
        runs.put_run(&company, &run).await.unwrap();
    }

    let tool = ReadTaskTool::new(company, Some(tasks), Some(runs), None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        out.contains("3 earlier attempt(s) omitted"),
        "must report how many older attempts were cut: {out}"
    );
    let output_at = out.find("## Output").expect("Output section present");
    let attempts_at = out.find("## Attempts").expect("Attempts section present");
    assert!(
        output_at > attempts_at,
        "the Output section must still be reachable after a long attempt history: {out}"
    );
}

/// LIMIT-axis (HT-072): `ReadTaskTool::description()` used to promise the
/// model "every attempt's status" while the render silently truncates to
/// `READ_TASK_ATTEMPTS_LIMIT` rows — a false completeness claim the model
/// reads before ever calling the tool, independent of the honest
/// `_N earlier attempt(s) omitted_` notice the render itself carries (see
/// `read_task_bounds_rendered_attempts_...` above). The description must
/// name the same cap the render enforces, and the two are pinned against
/// the same literal so a change to one is forced to touch the other
/// instead of silently drifting out of step the way they did to get here.
#[test]
fn read_task_description_names_the_same_cap_the_render_enforces() {
    let tool = ReadTaskTool::new(CompanyId::new("acme"), None, None, None);
    assert!(
        !tool.description().contains("every attempt's status"),
        "the description must not promise completeness the render does not keep: {}",
        tool.description()
    );
    assert!(
        tool.description().contains("10 most recent"),
        "the description should name the cap the render enforces: {}",
        tool.description()
    );
    assert_eq!(
        READ_TASK_ATTEMPTS_LIMIT, 10,
        "pinned against the same literal the description names, so a cap change cannot \
         drift silently out of step with the sentence the model reads"
    );
}

#[tokio::test]
async fn read_task_bounds_the_rendered_title_so_attempts_and_output_stay_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    let long_title = "x".repeat(5_000);
    tasks
        .upsert(
            &company,
            &task_card(
                "t-1",
                &long_title,
                crate::ports::tasks::COLUMN_IN_REVIEW,
                "engineer",
            ),
        )
        .await
        .unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    let header_line = out.lines().next().expect("header line present");
    assert!(
        header_line.chars().count() <= READ_TASK_TITLE_LIMIT + 2,
        "an operator-pasted title must not render verbatim and unbounded, or it can \
         consume the whole tool-result budget before later sections: {} chars",
        header_line.chars().count()
    );
    let output_at = out.find("## Output").expect("Output section present");
    let attempts_at = out.find("## Attempts").expect("Attempts section present");
    assert!(
        output_at > attempts_at,
        "the Output section must stay reachable behind a very long card title: {} bytes total",
        out.len()
    );
}

#[tokio::test]
async fn read_task_resolves_the_pinned_artifact_version_not_a_later_operator_edit() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let artifacts: Arc<dyn ArtifactStore> = fs;
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Draft the memo",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "engineer",
    );
    card.output = Some(TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-1".to_string(),
            attempt: Some(1),
        },
        at_millis: 5,
        artifacts: vec![crate::ports::tasks::TaskOutputArtifact {
            artifact_id: "art-1".to_string(),
            version: 1,
            title: "Memo".to_string(),
            kind: crate::ports::artifacts::ArtifactKind::Markdown,
        }],
        workflows: Vec::new(),
    });
    tasks.upsert(&company, &card).await.unwrap();

    let mut record = crate::ports::artifacts::ArtifactRecord::new(
        "art-1",
        "t-1",
        "Memo",
        crate::ports::artifacts::ArtifactKind::Markdown,
        "the agent's draft body",
        "engineer",
        5,
    );
    record.push_version(
        "an operator edited this after the attempt settled",
        crate::ports::artifacts::ArtifactAuthor::Operator,
        "operator",
        10,
        None,
    );
    artifacts.upsert(&company, &record).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, Some(artifacts));
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        out.contains("the agent's draft body"),
        "must render the version the task's output pinned, not the latest: {out}"
    );
    assert!(
        !out.contains("an operator edited this"),
        "a later operator edit must not render as what the task produced: {out}"
    );
}

#[tokio::test]
async fn read_task_only_renders_artifacts_pinned_by_the_current_output_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let artifacts: Arc<dyn ArtifactStore> = fs;
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Draft the memo",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "engineer",
    );
    card.output = Some(TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-2".to_string(),
            attempt: Some(2),
        },
        at_millis: 10,
        artifacts: vec![crate::ports::tasks::TaskOutputArtifact {
            artifact_id: "art-b".to_string(),
            version: 1,
            title: "Follow-up".to_string(),
            kind: crate::ports::artifacts::ArtifactKind::Markdown,
        }],
        workflows: Vec::new(),
    });
    tasks.upsert(&company, &card).await.unwrap();

    let record_a = crate::ports::artifacts::ArtifactRecord::new(
        "art-a",
        "t-1",
        "First draft",
        crate::ports::artifacts::ArtifactKind::Markdown,
        "attempt 1's body — superseded, no longer part of the latest output",
        "engineer",
        5,
    );
    artifacts.upsert(&company, &record_a).await.unwrap();
    let record_b = crate::ports::artifacts::ArtifactRecord::new(
        "art-b",
        "t-1",
        "Follow-up",
        crate::ports::artifacts::ArtifactKind::Markdown,
        "attempt 2's body",
        "engineer",
        10,
    );
    artifacts.upsert(&company, &record_b).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, Some(artifacts));
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        out.contains("attempt 2's body"),
        "the artifact pinned by the current output stamp must render: {out}"
    );
    assert!(
        !out.contains("First draft") && !out.contains("superseded"),
        "an artifact from an earlier attempt that the current output stamp does not pin \
         must not render as part of the latest output: {out}"
    );
}

#[tokio::test]
async fn read_task_treats_an_empty_output_stamp_as_the_latest_attempt_publishing_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(crate::store::FsOps::new(dir.path()));
    let tasks: Arc<dyn TaskStore> = fs.clone();
    let artifacts: Arc<dyn ArtifactStore> = fs;
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Draft the memo",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "engineer",
    );
    card.output = Some(TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-2".to_string(),
            attempt: Some(2),
        },
        at_millis: 10,
        artifacts: Vec::new(),
        workflows: Vec::new(),
    });
    tasks.upsert(&company, &card).await.unwrap();

    let record_a = crate::ports::artifacts::ArtifactRecord::new(
        "art-a",
        "t-1",
        "First draft",
        crate::ports::artifacts::ArtifactKind::Markdown,
        "attempt 1's body — attempt 2 published nothing",
        "engineer",
        5,
    );
    artifacts.upsert(&company, &record_a).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, Some(artifacts));
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(
        !out.contains("First draft") && !out.contains("attempt 1's body"),
        "an earlier attempt's artifact must not render as the latest attempt's output when \
         the current output stamp pins an empty (non-absent) artifact list: {out}"
    );
    assert!(
        out.contains("No artifacts published"),
        "an empty-but-present output stamp must render as the latest attempt publishing \
         nothing, not fall through to the all-artifacts legacy fallback: {out}"
    );
}

#[tokio::test]
async fn read_task_renders_workflows_recorded_in_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let tasks: Arc<dyn TaskStore> = Arc::new(crate::store::FsOps::new(dir.path()));
    let company = CompanyId::new("acme");

    let mut card = task_card(
        "t-1",
        "Automate the weekly report",
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "orchestrator",
    );
    card.output = Some(TaskOutput {
        source: crate::ports::tasks::TaskOutputSource::Run {
            run_id: "r-1".to_string(),
            attempt: Some(1),
        },
        at_millis: 5,
        artifacts: Vec::new(),
        workflows: vec![TaskOutputWorkflow {
            workflow_id: "wf-weekly-report".to_string(),
            run_id: Some("wf-run-1".to_string()),
            action: TaskOutputAction::Ran,
        }],
    });
    tasks.upsert(&company, &card).await.unwrap();

    let tool = ReadTaskTool::new(company, Some(tasks), None, None);
    let out = tool
        .execute(json!({ "task_id": "t-1" }))
        .await
        .unwrap()
        .output_for_llm(true);

    assert!(out.contains("### Workflows"), "{out}");
    assert!(out.contains("wf-weekly-report"), "{out}");
    assert!(
        out.contains("wf-run-1"),
        "the workflow's run id must be surfaced for read_run: {out}"
    );
}

// -- FAIL-axis: cross-tenant reach, ungrounded targets, unbounded growth -

/// A `RunStore` that genuinely partitions by company — the shape every
/// real backend promises — so a lookup under one company can never answer
/// with a row filed under another.
struct TenantScopedRunStore {
    rows: std::sync::Mutex<Vec<RunRecord>>,
}

#[async_trait]
impl RunStore for TenantScopedRunStore {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by this test")
    }
    async fn get_run(&self, company: &CompanyId, id: &str) -> crate::Result<Option<RunRecord>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| &r.company == company && r.id == id)
            .cloned())
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        unimplemented!("not exercised by this test")
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by this test")
    }
}

fn tenant_run(company: &str, id: &str) -> RunRecord {
    RunRecord {
        id: id.to_string(),
        company: CompanyId::new(company),
        task_id: None,
        chat_id: None,
        agent_id: "ceo".to_string(),
        attempt: 1,
        status: crate::ports::runs::RunStatus::Running,
        trigger_event_seq: None,
        thread_root: None,
        created_at_millis: 1_000,
        started_at_millis: None,
        finished_at_millis: None,
        error: None,
        usage: crate::ports::types::TokenUsage::default(),
        step_count: 0,
        workflow_run_id: None,
        node_id: None,
    }
}

/// FAIL-axis (HT-073): `ReadRunTool` is company-scoped only by
/// construction — `self.company` is the sole company argument it ever
/// passes to the run store, never anything derived from the `run_id`
/// argument. This pins that structural argument against a store that
/// genuinely partitions by company: a `run_id` that exists, but filed
/// under a DIFFERENT company, must read as not found, never leak.
#[tokio::test]
async fn read_run_never_leaks_a_run_id_belonging_to_another_company() {
    let runs: Arc<dyn RunStore> = Arc::new(TenantScopedRunStore {
        rows: std::sync::Mutex::new(vec![tenant_run("beta", "r-secret")]),
    });
    let tool = ReadRunTool::new(CompanyId::new("acme"), Some(runs), None);
    let out = tool
        .execute(json!({ "run_id": "r-secret" }))
        .await
        .unwrap()
        .output_for_llm(true);
    assert!(
        out.contains("No run"),
        "acme asking about beta's run_id must read as not-found: {out}"
    );
    assert!(
        !out.contains("Attempt"),
        "must not render beta's run: {out}"
    );
}

/// INPUT/STATE-axis (HT-074): `spawn_task` now grounds `assignee` on the
/// same terms `delegate_to_desk`/`delegate_to_teammate` already do (issue
/// #272) — a name that resolves to nobody on the roster is refused here,
/// in the model's own turn, rather than surviving as a queued card the
/// drain silently opens unowned with no signal anywhere that the assignee
/// was bogus.
///
/// The store is seeded (not empty) so grounding actually resolves the
/// roster rather than taking the fail-open path — an empty store would
/// pass this test for the wrong reason.
#[tokio::test]
async fn spawn_task_refuses_an_assignee_that_names_nobody_on_the_roster() {
    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = SpawnTaskTool::new(
        queue.clone(),
        company.clone(),
        Arc::new(MemStore::seeded(seeded_record(&company))),
    );

    let outcome = tool
        .execute(json!({
            "title": "Investigate the outage",
            "assignee": "totally-nonexistent-agent-id",
        }))
        .await
        .unwrap();
    assert!(
        outcome.is_error,
        "an assignee naming nobody on the roster must be refused before queuing"
    );
    assert!(
        outcome.text().contains("totally-nonexistent-agent-id"),
        "the refusal names the target the model typed: {}",
        outcome.text()
    );
    assert_eq!(queue.queued(), 0, "nothing should have been staged");
}

/// The other half: a real teammate id grounds and queues under its
/// canonical form, and a blank/absent `assignee` opens the card unowned
/// without ever touching the store.
#[tokio::test]
async fn spawn_task_grounds_a_real_teammate_and_leaves_a_blank_assignee_alone() {
    let company = CompanyId::new("acme");
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "eng"
role = "Engineer"
"#,
    )
    .expect("valid manifest");
    let record = CompanyRecord {
        manifest,
        ..seeded_record(&company)
    };
    let store = Arc::new(MemStore::seeded(record));

    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = SpawnTaskTool::new(queue.clone(), company.clone(), store.clone());
    let grounded = tool
        .execute(json!({ "title": "Fix the outage", "assignee": "ENG" }))
        .await
        .expect("execute");
    assert!(!grounded.is_error, "{}", grounded.text());

    let unassigned_tool = SpawnTaskTool::new(queue.clone(), company, store);
    let unassigned = unassigned_tool
        .execute(json!({ "title": "Untargeted work" }))
        .await
        .expect("execute");
    assert!(!unassigned.is_error, "{}", unassigned.text());

    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(
        drained,
        vec![
            Delegation::SpawnTask {
                title: "Fix the outage".to_string(),
                note: None,
                assignee: Some("eng".to_string()),
            },
            Delegation::SpawnTask {
                title: "Untargeted work".to_string(),
                note: None,
                assignee: None,
            },
        ],
        "a display name grounds to the canonical roster id, and no assignee is queued as \
         None rather than being pushed through the resolver at all"
    );
}

/// A `CompanyStore` that genuinely partitions by company — unlike
/// `MemStore`, which ignores the `id` argument and answers for whichever
/// company it was seeded with regardless of who asks. Needed to prove
/// `spawn_task`'s grounding actually scopes its lookup to `self.company`
/// rather than happening to work because every test fixture only ever
/// holds one company's record.
struct TenantScopedCompanyStore {
    records: std::collections::HashMap<String, CompanyRecord>,
}

#[async_trait::async_trait]
impl CompanyStore for TenantScopedCompanyStore {
    async fn load(&self, id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(self.records.get(id.as_ref()).cloned())
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

/// AUTH-axis (HT-074): `spawn_task`'s grounding must scope its roster
/// lookup to the tool's OWN company (`self.company`), never to a
/// different one — a teammate id that is real, but only on ANOTHER
/// company's roster, must be refused exactly as an invented id would be,
/// not accidentally admitted through a leaked cross-tenant read.
#[tokio::test]
async fn spawn_task_grounds_only_against_its_own_companys_roster() {
    let acme = CompanyId::new("acme");
    let beta = CompanyId::new("beta");
    let beta_manifest = toml::from_str(
        r#"
[company]
name = "Beta"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "eng"
role = "Engineer"
"#,
    )
    .expect("valid manifest");
    let mut records = std::collections::HashMap::new();
    records.insert("acme".to_string(), seeded_record(&acme));
    records.insert(
        "beta".to_string(),
        CompanyRecord {
            manifest: beta_manifest,
            ..seeded_record(&beta)
        },
    );
    let store = Arc::new(TenantScopedCompanyStore { records });

    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = SpawnTaskTool::new(queue.clone(), acme, store);

    let outcome = tool
        .execute(json!({ "title": "Fix the outage", "assignee": "eng" }))
        .await
        .unwrap();
    assert!(
        outcome.is_error,
        "a teammate id real only on a DIFFERENT company's roster must be refused, not \
         leaked in: {}",
        outcome.text()
    );
    assert_eq!(queue.queued(), 0);
}

/// FAIL-axis (HT-074): when the company record cannot be read at all —
/// the same store failure `DelegateToDeskTool`/`DelegateToTeammateTool`
/// fail OPEN on for the orchestrator's own unrestricted copy (see
/// `Grounding::ungrounded`) — `spawn_task` must fail open too, not refuse
/// to open a card just because the roster could not be checked this
/// instant. The assignee is queued exactly as typed, unresolved, the same
/// as it has always been for a request with no assignee to ground.
#[tokio::test]
async fn spawn_task_fails_open_when_the_company_record_cannot_be_read() {
    struct BrokenStore;
    #[async_trait::async_trait]
    impl CompanyStore for BrokenStore {
        async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
            Err(crate::OpenCompanyError::Store("store is down".to_string()))
        }
        async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
            Ok(())
        }
        async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
            Ok(Vec::new())
        }
        async fn append_ledger(
            &self,
            _id: &CompanyId,
            _entry: LedgerEntry,
        ) -> crate::Result<()> {
            Ok(())
        }
    }

    let company = CompanyId::new("acme");
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = SpawnTaskTool::new(queue.clone(), company, Arc::new(BrokenStore));

    let outcome = tool
        .execute(json!({ "title": "Investigate the outage", "assignee": "eng" }))
        .await
        .unwrap();
    assert!(
        !outcome.is_error,
        "a store failure must not block opening the card: {}",
        outcome.text()
    );
    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(
        drained,
        vec![Delegation::SpawnTask {
            title: "Investigate the outage".to_string(),
            note: None,
            assignee: Some("eng".to_string()),
        }],
        "the assignee is queued as typed, unresolved, when grounding could not run at all"
    );
}

/// FAIL-axis (HT-076): every fixture in this module hand-writes its
/// `CompanyRecord` manifest with short, convenient agent ids ("ceo",
/// "writer"). The real setup pipeline
/// (`company::setup::manifest_from_setup`) derives ids from the agent's
/// ROLE text via `unique_agent_id`/`snake_id` — multi-word,
/// underscore-separated ids no hand fixture happens to produce. This
/// proves `delegate_to_teammate`'s grounding
/// (`CompanyRecord::resolve_teammate_key`) agrees with that real shape,
/// not just the fixtures' convenient one.
#[tokio::test]
async fn delegate_to_teammate_grounds_against_a_realistically_derived_roster_id() {
    let agents = vec![
        crate::company::setup::ProposedAgent {
            name: "Head".to_string(),
            role: "Head of Product Strategy".to_string(),
            description: "Owns the roadmap.".to_string(),
            focus: None,
        },
        crate::company::setup::ProposedAgent {
            name: "Ops".to_string(),
            role: "Chief Operating Officer".to_string(),
            description: "Runs the business.".to_string(),
            focus: None,
        },
    ];
    let manifest = crate::company::setup::manifest_from_setup(
        &crate::company::setup::SetupAnswers::default(),
        &agents,
        None,
    );
    let real_id = manifest.agents[0].id.clone();
    assert!(
        real_id.contains('_'),
        "the real roster builder derives multi-word ids, unlike this module's short hand \
         fixtures: got {real_id:?}"
    );

    let company = CompanyId::new("acme");
    let record = CompanyRecord {
        manifest,
        ..seeded_record(&company)
    };
    let store: Arc<dyn CompanyStore> = Arc::new(MemStore::seeded(record));
    let queue = DelegationQueue::default();
    let _claim = queue.claim();
    let tool = DelegateToTeammateTool::new(queue.clone(), company, store);

    let out = tool
        .execute(json!({ "teammate": real_id.clone(), "instruction": "review the roadmap" }))
        .await
        .unwrap();
    assert!(
        !out.is_error,
        "grounding must resolve a real setup-derived id, not just the hand fixtures' short \
         ones: {}",
        out.text()
    );
    let drained = queue.drain(MAX_DELEGATIONS_PER_TURN);
    assert_eq!(
        drained,
        vec![Delegation::DelegateToTeammate {
            teammate: real_id,
            instruction: "review the roadmap".to_string(),
        }]
    );
}

/// The cap counts the company's own roster, and every load appends more.
///
/// `apply_globals` puts the host's baseline teammates into `agents` on
/// every production load. A cap that counted the whole list would spend
/// most of its budget on teammates the company neither added nor can
/// remove, and a company with a designed roster would be refused its first
/// mint. The manifest here is built the way production builds one, so the
/// baseline is present and the count has to see past it.
#[tokio::test]
async fn add_agent_counts_manifest_teammates_toward_the_roster_cap() {
    let company = CompanyId::new("acme");
    let mut record = seeded_record(&company);
    let mut manifest: crate::company::CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"designer\"\nrole = \"Designer\"\n",
    )
    .expect("valid manifest");
    manifest.apply_globals();
    assert!(
        manifest.agents.len() > manifest.own_agents().count(),
        "this test is only meaningful while the baseline is appended to a roster"
    );
    record.manifest = manifest;
    let store = Arc::new(MemStore::seeded(record));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    for i in 1..crate::company::setup::MAX_AGENTS {
        let result = tool
            .execute(json!({ "name": format!("Teammate {i}"), "role": "Generalist" }))
            .await
            .expect("execute");
        assert!(
            !result.is_error,
            "mint {i} unexpectedly refused: {}",
            result.text()
        );
    }

    let result = tool
        .execute(json!({ "name": "One too many", "role": "Generalist" }))
        .await
        .expect("execute");
    assert!(result.is_error, "{}", result.text());
    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents.len(),
        crate::company::setup::MAX_AGENTS - 1,
        "refusal must not persist another teammate"
    );
}

/// A roster at the setup cap refuses further minting.
#[tokio::test]
async fn add_agent_refuses_once_the_roster_reaches_the_setup_cap() {
    let company = CompanyId::new("acme");
    let store = Arc::new(MemStore::seeded(seeded_record(&company)));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    for i in 0..crate::company::setup::MAX_AGENTS {
        let result = tool
            .execute(json!({ "name": format!("Teammate {i}"), "role": "Generalist" }))
            .await
            .unwrap();
        assert!(!result.is_error);
    }
    let result = tool
        .execute(json!({ "name": "One too many", "role": "Generalist" }))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "a roster already at the setup cap must refuse further minting"
    );
}

#[tokio::test]
async fn add_agent_does_not_count_retired_teammates_toward_the_roster_cap() {
    let company = CompanyId::new("acme");
    let mut record = seeded_record(&company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"designer\"\nrole = \"Designer\"\n",
    )
    .expect("valid manifest");
    record.overlay_retired_agents.push("designer".to_string());
    let store = Arc::new(MemStore::seeded(record));
    let tool = unscoped_add_agent(company.clone(), store.clone());

    for i in 0..crate::company::setup::MAX_AGENTS {
        let result = tool
            .execute(json!({ "name": format!("Teammate {i}"), "role": "Generalist" }))
            .await
            .expect("execute");
        assert!(!result.is_error, "{}", result.text());
    }
    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents.len(),
        crate::company::setup::MAX_AGENTS
    );
}

#[tokio::test]
async fn concurrent_add_agent_calls_cannot_exceed_the_roster_cap() {
    let company = CompanyId::new("acme");
    let store = Arc::new(YieldingStore {
        record: StdMutex::new(Some(seeded_record(&company))),
    });
    let first = unscoped_add_agent(company.clone(), store.clone());
    let second = unscoped_add_agent(company.clone(), store.clone());
    for i in 1..crate::company::setup::MAX_AGENTS {
        let result = first
            .execute(json!({ "name": format!("Teammate {i}"), "role": "Generalist" }))
            .await
            .expect("execute");
        assert!(!result.is_error, "{}", result.text());
    }

    let (a, b) = tokio::join!(
        first.execute(json!({ "name": "Jamie", "role": "Growth Lead" })),
        second.execute(json!({ "name": "Alex", "role": "Support Lead" })),
    );
    let (a, b) = (a.expect("execute"), b.expect("execute"));
    assert_eq!([a, b].iter().filter(|result| result.is_error).count(), 1);
    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents.len(),
        crate::company::setup::MAX_AGENTS
    );
}

/// A store that yields between reading a record and writing it back, so two
/// concurrent `add_agent` calls genuinely interleave their load → push →
/// save cycle rather than each running to completion uncontended.
struct YieldingStore {
    record: StdMutex<Option<CompanyRecord>>,
}

#[async_trait::async_trait]
impl CompanyStore for YieldingStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        let snapshot = self.record.lock().expect("record").clone();
        tokio::task::yield_now().await;
        Ok(snapshot)
    }
    async fn save(&self, record: &CompanyRecord) -> crate::Result<()> {
        tokio::task::yield_now().await;
        *self.record.lock().expect("record") = Some(record.clone());
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

/// FAIL-axis (HT-079, the concurrency half): `add_agent` is a read-modify-
/// write over the whole record — load, push onto `overlay_agents`, save —
/// with awaits on both ends. `company_write_lock` is what stops two of them
/// interleaving; without it the second save writes a record built from a
/// snapshot taken before the first landed, and one minted teammate simply
/// disappears while its caller is told it was added.
#[tokio::test]
async fn concurrent_add_agent_calls_cannot_lose_a_mint_to_the_load_push_save_race() {
    let company = CompanyId::new("acme");
    let store = Arc::new(YieldingStore {
        record: StdMutex::new(Some(seeded_record(&company))),
    });
    let first = unscoped_add_agent(company.clone(), store.clone());
    let second = unscoped_add_agent(company.clone(), store.clone());

    let (a, b) = tokio::join!(
        first.execute(json!({ "name": "Jamie", "role": "Growth Lead" })),
        second.execute(json!({ "name": "Alex", "role": "Support Lead" })),
    );
    assert!(!a.expect("execute").is_error);
    assert!(!b.expect("execute").is_error);

    let record = store.load(&company).await.unwrap().expect("persisted");
    let names: Vec<&str> = record
        .overlay_agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "both mints were acknowledged, so both must survive the race: {names:?}"
    );
    assert!(
        names.contains(&"Jamie") && names.contains(&"Alex"),
        "{names:?}"
    );
}

/// The other half of the same window: the duplicate-name guard reads
/// `overlay_agents` from a snapshot and pushes onto it, so two concurrent
/// mints of the SAME name are exactly the check-then-act the write lock has
/// to serialise. One must be refused, and the roster must hold one entry.
#[tokio::test]
async fn concurrent_add_agent_calls_for_one_name_mint_it_once() {
    let company = CompanyId::new("acme");
    let store = Arc::new(YieldingStore {
        record: StdMutex::new(Some(seeded_record(&company))),
    });
    let first = unscoped_add_agent(company.clone(), store.clone());
    let second = unscoped_add_agent(company.clone(), store.clone());

    let (a, b) = tokio::join!(
        first.execute(json!({ "name": "Jamie", "role": "Growth Lead" })),
        second.execute(json!({ "name": "Jamie", "role": "Growth Lead" })),
    );
    let (a, b) = (a.expect("execute"), b.expect("execute"));
    assert_eq!(
        [&a, &b].iter().filter(|r| r.is_error).count(),
        1,
        "exactly one of two identical mints must be refused.\nfirst: {}\nsecond: {}",
        a.text(),
        b.text()
    );

    let record = store.load(&company).await.unwrap().expect("persisted");
    assert_eq!(
        record.overlay_agents.len(),
        1,
        "the duplicate guard must leave exactly one teammate: {:?}",
        record
            .overlay_agents
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>()
    );
}
