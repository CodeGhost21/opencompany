use super::*;

fn class_of(message: &str) -> Option<BlockerClass> {
    classify_blocker_message(message)
}

#[test]
fn a_rejected_model_id_is_infrastructure() {
    let class = class_of("dispatch failed: the model `gpt-nonexistent` does not exist or you do not have access to it")
        .expect("a rejected model id is recognised");
    assert_eq!(class.kind, BlockerKind::Infrastructure);
    assert_eq!(class.source, BlockerSource::Provider);
    assert!(class.kind.parks());
}

#[test]
fn a_bad_key_is_infrastructure() {
    let class = class_of("hosted inference returned 401: invalid api key")
        .expect("an auth failure is recognised");
    assert_eq!(class.kind, BlockerKind::Infrastructure);
    assert_eq!(class.source, BlockerSource::Provider);
}

#[test]
fn a_disconnected_integration_is_infrastructure_from_a_tool() {
    let class = class_of("tool call failed: could not connect to mcp server `slack`")
        .expect("an MCP connection failure is recognised");
    assert_eq!(class.kind, BlockerKind::Infrastructure);
    assert_eq!(class.source, BlockerSource::Tool);
}

/// The point of carrying `Transient` in the taxonomy: it is recognised, and
/// recognising it is how we know **not** to ask anybody.
#[test]
fn a_rate_limit_is_recognised_but_does_not_park() {
    let class = class_of("hosted inference returned 429: rate limit exceeded")
        .expect("a rate limit is recognised");
    assert_eq!(class.kind, BlockerKind::Transient);
    assert!(
        !class.kind.parks(),
        "a rate limit resolves itself; asking a person about it wastes their attention"
    );
}

/// Rate limiting outranks the auth row, because a 429 body routinely names
/// the key it is throttling. Getting this backwards would park a
/// self-resolving stop as a broken credential.
#[test]
fn a_throttled_key_reads_as_transient_not_as_bad_auth() {
    let class = class_of("429 too many requests for this api key").expect("recognised");
    assert_eq!(class.kind, BlockerKind::Transient);
}

/// The boundary check reads every occurrence, not just the first.
///
/// A provider line that names a request id before its status code —
/// `req-4290 … http 429` — used to stop at the `429` inside `4290`, reject
/// its trailing `0`, and never reach the real code. The miss was not
/// neutral: the auth row sits below the transient one, so the same body
/// mentioning a key would then park a self-resolving throttle as a broken
/// credential and wait for a person with nothing to fix.
#[test]
fn a_status_code_is_found_past_an_earlier_unbounded_lookalike() {
    let class = class_of("request req-4290 received HTTP 429").expect("recognised");
    assert_eq!(class.kind, BlockerKind::Transient);

    let auth_flavoured =
        class_of("request req-4290 failed: http 429 for this api key").expect("recognised");
    assert_eq!(
        auth_flavoured.kind,
        BlockerKind::Transient,
        "the real 429 must still outrank the auth row it is quoted beside"
    );
}

/// …and the boundary itself still holds: a longer number that merely starts
/// with a status code is not that status code.
#[test]
fn a_longer_number_is_not_a_status_code() {
    assert_eq!(class_of("dispatch failed on port 4010"), None);
    assert_eq!(class_of("dispatch failed: worker 4290 died"), None);
}

/// A bare `401` is enough on its own, whatever punctuation follows it.
///
/// It was not, and nothing said so: the leaf was spelled as the pair
/// `"401 "` / `" 401"` — a boundary check written into the leaf instead of
/// around it. The leading-space form made the start-boundary test read the
/// character *before* the space, a letter in every real provider message,
/// so it rejected all of them; the trailing-space form missed `401:` and a
/// line ending in `401`. Every existing 401 test passed anyway, because
/// each message also carried `invalid api key` or `unauthorized` — the
/// prose phrases were doing all the work and the status code none of it.
#[test]
fn a_bare_401_is_an_auth_blocker_whatever_follows_it() {
    for message in [
        "hosted inference returned 401: invalid credentials",
        "provider rejected the call with http 401",
        "401 returned by the upstream",
    ] {
        let class = class_of(message).unwrap_or_else(|| panic!("unrecognised: {message}"));
        assert_eq!(
            class.kind,
            BlockerKind::Infrastructure,
            "message: {message}"
        );
        assert_eq!(class.source, BlockerSource::Provider, "message: {message}");
    }
}

/// The conservative default. An error we cannot name keeps today's
/// behaviour rather than guessing at a question for somebody.
#[test]
fn an_unrecognised_failure_is_not_a_blocker() {
    assert_eq!(class_of("dispatch failed: index out of bounds"), None);
    assert_eq!(
        class_of("hand-off failed: the delegate produced nothing"),
        None
    );
    assert_eq!(class_of(""), None);
}

/// Whole phrases, not loose words — a provider body that merely mentions a
/// credential must not be read as a broken one.
#[test]
fn a_body_that_merely_mentions_a_key_is_not_an_auth_blocker() {
    assert_eq!(
        class_of("the document describes how to store an api key safely"),
        None,
        "matching the bare word `api key` would park an unrelated failure"
    );
}

#[test]
fn matching_is_case_insensitive() {
    assert!(class_of("HTTP 401 UNAUTHORIZED").is_some());
}

/// Every row promises something a person can act on, including the
/// transient row (whose promise is that there is nothing to do).
#[test]
fn every_shape_says_what_is_needed() {
    for shape in SHAPES {
        assert!(
            !shape.class.needed.trim().is_empty(),
            "a blocker with nothing in `needed` reaches a person with nothing to do"
        );
        assert!(
            !shape.leaves.is_empty(),
            "a shape with no phrases can never match"
        );
    }
    assert!(!PREREQ_BLOCKER.needed.is_empty());
    assert!(!AGENT_QUESTION_BLOCKER.needed.is_empty());
}

/// The two host-declared classes park by construction — they exist because
/// something already established a person is needed.
#[test]
fn declared_classes_park() {
    assert!(PREREQ_BLOCKER.kind.parks());
    assert!(AGENT_QUESTION_BLOCKER.kind.parks());
}

/// Two cards stalled on the same integration carry the same group key, so
/// the console folds them into one question instead of two.
#[test]
fn a_named_connection_groups_by_its_name() {
    let a = connection_group_key("could not connect to mcp server `slack`")
        .expect("a named connection groups");
    let b = connection_group_key("dispatch failed: could not connect to mcp server `slack`")
        .expect("the same connection, a different reason");
    assert_eq!(a, b);
    assert_eq!(a, "connection:slack");
    assert_ne!(
        connection_group_key("could not connect to mcp server `notion`"),
        Some(a),
        "a different server is a different question"
    );
}

/// The gate is that the reason matched a connection shape — a backticked
/// token in an auth or model-id reason is not a connection and must not
/// group those distinct stops together.
#[test]
fn a_backtick_outside_a_connection_shape_does_not_group() {
    assert_eq!(connection_group_key("unknown model `gpt-nope`"), None);
    assert_eq!(connection_group_key("invalid api key `sk-123`"), None);
    assert_eq!(
        connection_group_key("could not connect to mcp server"),
        None,
        "a connection shape with no name has nothing to group by"
    );
}

/// A backticked token before the connection marker — a tool name in the
/// prose — is not the connection; the name after the marker is.
#[test]
fn the_name_is_read_after_the_connection_marker() {
    assert_eq!(
        connection_group_key("tool `search` failed: could not connect to mcp server `slack`"),
        Some("connection:slack".to_string()),
        "the connection is the server after the marker, not the earlier tool token"
    );
}
}

// ───────────────────────────────────────────────────────────────────────────
// The agent's own door (issue #1861)
// ───────────────────────────────────────────────────────────────────────────

/// The `escalate_to_human` tool name.
pub const ESCALATE_TO_HUMAN_TOOL: &str = "escalate_to_human";

/// Queues an [`Information`](BlockerKind::Information) blocker for the operator.
/// The turn's drain parks accepted questions through the approval lifecycle.
///
/// Escalation establishes the turn boundary: subsequent tool calls are refused
/// until a new turn. An accepted question is queued for the operator; a full
/// batch returns an explicit refusal.
pub struct EscalateToHumanTool {
requests: crate::harness::built_in::policy::ApprovalRequestQueue,
agent: String,
}

impl EscalateToHumanTool {
/// Builds the tool over the shared approval-request queue, for one agent.
pub fn new(
    requests: crate::harness::built_in::policy::ApprovalRequestQueue,
    agent: String,
) -> Self {
    Self { requests, agent }
}
}

#[async_trait::async_trait]
impl openhuman_core::tools::traits::Tool for EscalateToHumanTool {
fn name(&self) -> &str {
    ESCALATE_TO_HUMAN_TOOL
}

fn description(&self) -> &str {
    "Ask the operator a question you cannot answer yourself, when the work genuinely cannot \
     proceed without it — a missing prerequisite, a choice only they can make, two \
     instructions that contradict each other. Provide the `question` in plain words, and \
     optionally the `context` you already gathered. The card parks and waits for their \
     answer rather than failing. Use it instead of guessing, and instead of finishing with \
     prose explaining that you were stuck. Do NOT use it for something you can look up, for \
     something a teammate would know, or to confirm a decision you have already been given."
}

fn parameters_schema(&self) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "description": "What you need the operator to tell you, in one or two plain sentences."
            },
            "context": {
                "type": "string",
                "description": "Optional: what you already tried or found, so they can answer without re-deriving it."
            }
        },
        "required": ["question"],
        "additionalProperties": false
    })
}

fn permission_level(&self) -> openhuman_core::tools::traits::PermissionLevel {
    openhuman_core::tools::traits::PermissionLevel::Write
}

async fn execute(
    &self,
    args: serde_json::Value,
) -> anyhow::Result<openhuman_core::tools::traits::ToolResult> {
    use openhuman_core::tools::traits::ToolResult;

    let question = args
        .get("question")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow::anyhow!("`question` is required"))?
        .to_string();
    let context = args
        .get("context")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty());

    // The reason a person reads is the question plus whatever the agent
    // already worked out — not a wrapper sentence about escalation, which
    // would push the actual question down the card.
    let reason = match context {
        Some(context) => format!("{question}\n\nWhat {} already has: {context}", self.agent),
        None => question.clone(),
    };

    let payload = crate::ports::blockers::BlockerPayload {
        kind: AGENT_QUESTION_BLOCKER.kind,
        source: AGENT_QUESTION_BLOCKER.source,
        // No step: a question asked mid-conversation has no card behind it,
        // and where one does exist the approval's own task link already
        // names it. See `BlockerPayload::step`.
        step: None,
        reason: reason.clone(),
        needed: AGENT_QUESTION_BLOCKER.needed.to_string(),
        // A question is particular to its own card; nothing else shares its
        // answer, so it groups with nothing.
        group_key: None,
    };
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null),
        // `None`, even though an agent did raise this and the field exists
        // to name one. `Some(agent)` means "a tool call openhuman blocked",
        // and approving one mints a single-use grant and re-dispatches the
        // agent to run that exact call again — which here would call
        // `escalate_to_human` a second time and park the same question.
        // Carrying the operator's answer back into the turn is #1863; until
        // it lands, approving a blocker is deliberately inert.
        agent: None,
        // Stamped by the dispatch boundary's `stamp_run`, which retro-fills
        // every request this turn queued.
        run_id: None,
    };
    if !self
        .requests
        .push_blocker(crate::harness::built_in::policy::ApprovalRequest {
            tool: ESCALATE_TO_HUMAN_TOOL.to_string(),
            reason,
            effect,
        })
    {
        return Ok(ToolResult::error(format!(
            "Your question was not raised: this batch already has the maximum of {} approval \
             requests. Stop and wait for the queued requests to be resolved, then ask again.",
            crate::harness::built_in::policy::MAX_APPROVAL_REQUESTS_PER_TURN
        )));
    }

    Ok(ToolResult::success(format!(
        "Raised your question with the operator: \"{question}\". This card parks until they \
         answer. Stop and wait for their answer; do not ask it again."
    )))
}
}

#[cfg(test)]
mod tool_test {
use super::*;
use crate::harness::built_in::policy::ApprovalRequestQueue;
use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource};
use openhuman_core::tools::traits::Tool;

fn tool(queue: &ApprovalRequestQueue) -> EscalateToHumanTool {
    EscalateToHumanTool::new(queue.clone(), "engineer".to_string())
}

#[tokio::test]
async fn a_question_parks_as_an_information_blocker() {
    let queue = ApprovalRequestQueue::default();
    let result = tool(&queue)
        .execute(serde_json::json!({ "question": "staging or prod?" }))
        .await
        .expect("the tool runs");
    assert!(
        !result.is_error,
        "asking is not a failure: {}",
        result.text()
    );

    let drained = queue.drain(8);
    assert_eq!(drained.requests.len(), 1);
    let request = &drained.requests[0];
    assert_eq!(request.tool, ESCALATE_TO_HUMAN_TOOL);
    assert_eq!(request.effect.kind, "blocker.information");

    let payload: BlockerPayload =
        serde_json::from_value(request.effect.payload.clone()).expect("payload round-trips");
    assert_eq!(payload.kind, BlockerKind::Information);
    assert_eq!(payload.source, BlockerSource::AgentQuestion);
    assert_eq!(
        payload.step, None,
        "a question asked mid-turn names no step; the approval's task link does"
    );
    assert!(payload.reason.contains("staging or prod?"));
}

/// The context the agent already gathered rides along, so the operator can
/// answer without re-deriving it — and it is joined into the reason rather
/// than dropped into a field nothing renders yet.
#[tokio::test]
async fn gathered_context_reaches_the_question() {
    let queue = ApprovalRequestQueue::default();
    tool(&queue)
        .execute(serde_json::json!({
            "question": "which brief is current?",
            "context": "the Jan and Mar briefs contradict on pricing"
        }))
        .await
        .expect("the tool runs");

    let drained = queue.drain(8);
    let payload: BlockerPayload =
        serde_json::from_value(drained.requests[0].effect.payload.clone()).expect("payload");
    assert!(payload.reason.contains("which brief is current?"));
    assert!(payload.reason.contains("contradict on pricing"));
    assert!(
        payload.reason.contains("engineer"),
        "the context is attributed to the agent that gathered it"
    );
}

/// A blank question is refused rather than parked: an empty card reaches a
/// person with nothing to answer and still costs them the interruption.
#[tokio::test]
async fn an_empty_question_is_refused_and_parks_nothing() {
    let queue = ApprovalRequestQueue::default();
    assert!(
        tool(&queue)
            .execute(serde_json::json!({ "question": "   " }))
            .await
            .is_err()
    );
    assert!(queue.drain(8).requests.is_empty());
}

/// Approving an escalation must not re-dispatch the agent into calling the
/// same tool again — see the `agent` field's note.
#[tokio::test]
async fn an_escalation_mints_no_grant() {
    let queue = ApprovalRequestQueue::default();
    tool(&queue)
        .execute(serde_json::json!({ "question": "staging or prod?" }))
        .await
        .expect("runs");
    assert!(queue.drain(8).requests[0].effect.agent.is_none());
}

/// Two agents asking distinct questions in the same turn race through
/// `execute` concurrently — nothing upstream of this tool serialises the
/// calls — so both must still land their own card rather than one
/// silently losing to the other on the shared queue's `Mutex`.
///
/// Driven from two worker threads through a [`Barrier`], not from
/// `tokio::join!`: `execute` has no suspension point around its
/// synchronous `push`, so joined futures are polled to completion one
/// after the other on a single task. That arrangement exercises two serial
/// inserts and would pass unchanged if simultaneous calls could lose a
/// card — which is the only thing this test exists to rule out.
///
/// Repeated, because the barrier releases both workers before either
/// reaches `push` rather than at `push` itself: a single round can
/// interleave benignly. Making the window certain would mean a test hook
/// inside the queue every caller pays for, so the rounds buy it instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_questions_from_different_agents_both_park() {
    use std::sync::{Arc, Barrier};

    for round in 0..20 {
        let queue = ApprovalRequestQueue::default();
        let finance = EscalateToHumanTool::new(queue.clone(), "finance".to_string());
        let legal = EscalateToHumanTool::new(queue.clone(), "legal".to_string());
        let gate = Arc::new(Barrier::new(2));

        let ask = |tool: EscalateToHumanTool, question: &'static str, gate: Arc<Barrier>| {
            tokio::task::spawn_blocking(move || {
                gate.wait();
                tokio::runtime::Handle::current()
                    .block_on(tool.execute(serde_json::json!({ "question": question })))
            })
        };
        let a = ask(finance, "approve the Q3 budget?", gate.clone());
        let b = ask(legal, "sign the NDA as-is?", gate.clone());
        assert!(!a.await.expect("joins").expect("runs").is_error);
        assert!(!b.await.expect("joins").expect("runs").is_error);

        let drained = queue.drain(8);
        let reasons: Vec<&String> = drained.requests.iter().map(|r| &r.reason).collect();
        assert_eq!(
            drained.requests.len(),
            2,
            "round {round}: both concurrent questions must reach the queue, not just \
             whichever wins the race: {reasons:?}"
        );
        for question in ["approve the Q3 budget?", "sign the NDA as-is?"] {
            assert!(
                reasons.iter().any(|reason| reason.contains(question)),
                "round {round}: the queue must hold each agent's own question, not one of \
                 them twice: {reasons:?}"
            );
        }
    }
}

/// The other half of `an_escalation_mints_no_grant`, and the half that is
/// load-bearing in the opposite direction.
///
/// `agent: None` is what stops an approval re-dispatching the agent into
/// asking the same question again. But `None` is also what
/// `CycleRunner::settle_approval` reads as *a native effect the runtime
/// performs*, and its fall-through hands the effect to
/// `execute_effect_once` — which for a blocker payload ledgers a phantom
/// spend and routes nothing while reporting success. The only thing
/// standing between those two is
/// [`is_blocker_effect`](crate::ports::blockers::is_blocker_effect), which
/// matches on the effect **kind string**. Nothing else couples the kind
/// this tool stamps to the prefix that guard looks for, so a rename on
/// either side reopens the fall-through silently.
#[tokio::test]
async fn an_escalation_is_recognisable_as_a_blocker_so_approval_cannot_execute_it_natively() {
    let queue = ApprovalRequestQueue::default();
    tool(&queue)
        .execute(serde_json::json!({ "question": "staging or prod?" }))
        .await
        .expect("runs");
    let effect = queue.drain(8).requests[0].effect.clone();
    assert!(
        effect.agent.is_none(),
        "a grant here would re-ask the question"
    );
    assert!(
        crate::ports::blockers::is_blocker_effect(&effect),
        "an agent-None effect that is not recognised as a blocker falls through to native \
         execution on approval: {}",
        effect.kind
    );
    assert!(
        serde_json::from_value::<BlockerPayload>(effect.payload.clone()).is_ok(),
        "the resolve path reads the payload back off the parked effect to carry the step: {:?}",
        effect.payload
    );
    assert!(
        effect.amount_usd.is_none(),
        "a question costs nothing; an amount here is what a phantom spend would be ledgered \
         from"
    );
}

/// STATE-axis (REQ-002): `push`'s de-duplication is per [`ApprovalScope`]
/// (issue #439) — two different turns asking the identical question are
/// two requests, not one collapsed into the other. This is the flip side
/// of `a_repeated_identical_escalation_collapses_but_a_distinct_one_survives`
/// (`policy.rs`), which proves the collapse WITHIN one turn; this proves a
/// prior turn's already-drained card does not leave state that suppresses
/// an identical question asked again in a later, separate turn.
#[tokio::test]
async fn escalate_to_human_repeated_across_different_turns_is_not_deduped() {
    let queue = ApprovalRequestQueue::default();
    let tool = tool(&queue);

    let first_turn = queue.claim(crate::harness::built_in::policy::ApprovalScope::Run(
        "run-1".to_string(),
    ));
    let first_drain = first_turn
        .scoped(async {
            tool.execute(serde_json::json!({ "question": "staging or prod?" }))
                .await
                .expect("first turn runs");
            queue.drain(8)
        })
        .await;
    assert_eq!(
        first_drain.requests.len(),
        1,
        "the first turn's own question lands"
    );
    drop(first_turn);

    let second_turn = queue.claim(crate::harness::built_in::policy::ApprovalScope::Run(
        "run-2".to_string(),
    ));
    let second_drain = second_turn
        .scoped(async {
            tool.execute(serde_json::json!({ "question": "staging or prod?" }))
                .await
                .expect("second turn runs");
            queue.drain(8)
        })
        .await;
    assert_eq!(
        second_drain.requests.len(),
        1,
        "a later, separate turn asking the identical question must not read as a duplicate \
         of a card the first turn already drained and lost scope of"
    );
}

#[tokio::test]
async fn escalate_to_human_exactly_at_the_cap_produces_no_overflow() {
    use crate::harness::built_in::policy::MAX_APPROVAL_REQUESTS_PER_TURN;

    let queue = ApprovalRequestQueue::default();
    let tool = tool(&queue);
    for i in 0..MAX_APPROVAL_REQUESTS_PER_TURN {
        let outcome = tool
            .execute(serde_json::json!({ "question": format!("question {i}?") }))
            .await
            .expect("runs");
        assert!(!outcome.is_error, "question {i}: {}", outcome.text());
    }

    let drained = queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN);
    assert_eq!(
        drained.requests.len(),
        MAX_APPROVAL_REQUESTS_PER_TURN,
        "exactly the cap's worth of distinct questions must all land"
    );
    assert_eq!(
        drained.discarded, 0,
        "at exactly the cap, nothing overflows"
    );
    assert!(
        drained.overflow_notice().is_none(),
        "no notice is owed when nothing was dropped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_questions_compete_for_the_final_slot_without_silent_loss() {
    use crate::harness::built_in::policy::MAX_APPROVAL_REQUESTS_PER_TURN;
    use std::sync::{Arc, Barrier};

    for round in 0..20 {
        let queue = ApprovalRequestQueue::default();
        for i in 0..MAX_APPROVAL_REQUESTS_PER_TURN - 1 {
            let asked = tool(&queue)
                .execute(serde_json::json!({ "question": format!("existing question {i}") }))
                .await
                .expect("the tool runs");
            assert!(!asked.is_error, "{}", asked.text());
        }

        let barrier = Arc::new(Barrier::new(2));
        let ask = |agent: &str, question: &'static str| {
            let tool = EscalateToHumanTool::new(queue.clone(), agent.to_string());
            let barrier = barrier.clone();
            tokio::task::spawn_blocking(move || {
                barrier.wait();
                tokio::runtime::Handle::current()
                    .block_on(tool.execute(serde_json::json!({ "question": question })))
            })
        };
        let finance = ask("finance", "approve the final budget?");
        let legal = ask("legal", "approve the final contract?");
        let results = [
            (
                "approve the final budget?",
                finance.await.expect("joins").expect("the tool runs"),
            ),
            (
                "approve the final contract?",
                legal.await.expect("joins").expect("the tool runs"),
            ),
        ];
        assert_eq!(
            results
                .iter()
                .filter(|(_, result)| !result.is_error)
                .count(),
            1,
            "round {round}: one remaining blocker slot must have exactly one successful caller"
        );

        let drained = queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN);
        assert_eq!(drained.requests.len(), MAX_APPROVAL_REQUESTS_PER_TURN);
        assert_eq!(
            drained.discarded, 0,
            "no accepted question may be discarded"
        );
        for i in 0..MAX_APPROVAL_REQUESTS_PER_TURN - 1 {
            assert!(
                drained
                    .requests
                    .iter()
                    .any(|r| r.reason == format!("existing question {i}"))
            );
        }
        for (question, result) in results {
            let retained = drained.requests.iter().any(|r| r.reason == question);
            assert_eq!(retained, !result.is_error, "round {round}: {question}");
            if result.is_error {
                assert!(result.text().contains("not raised"), "{}", result.text());
            }
        }
    }
}

#[tokio::test]
async fn a_full_run_accepts_its_duplicate_without_consuming_another_runs_capacity() {
    use crate::harness::built_in::policy::{ApprovalScope, MAX_APPROVAL_REQUESTS_PER_TURN};

    let queue = ApprovalRequestQueue::default();
    let full = queue.claim(ApprovalScope::Run("full".to_string()));
    let other = queue.claim(ApprovalScope::Run("other".to_string()));
    let tool = tool(&queue);
    full.scoped(async {
        for i in 0..MAX_APPROVAL_REQUESTS_PER_TURN {
            let asked = tool
                .execute(serde_json::json!({ "question": format!("question {i}") }))
                .await
                .expect("the tool runs");
            assert!(!asked.is_error, "{}", asked.text());
        }
        let duplicate = tool
            .execute(serde_json::json!({ "question": "question 0" }))
            .await
            .expect("the tool runs");
        assert!(
            !duplicate.is_error,
            "the existing question is already queued"
        );
        let refused = tool
            .execute(serde_json::json!({ "question": "new question" }))
            .await
            .expect("the tool runs");
        assert!(
            refused.is_error,
            "a new question must be refused at the cap"
        );
    })
    .await;

    let independent = other
        .scoped(tool.execute(serde_json::json!({ "question": "question 0" })))
        .await
        .expect("the tool runs");
    assert!(
        !independent.is_error,
        "a different run has its own capacity"
    );
    let full_drain = full
        .scoped(async { queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN) })
        .await;
    assert_eq!(full_drain.requests.len(), MAX_APPROVAL_REQUESTS_PER_TURN);
    assert_eq!(full_drain.discarded, 0);
    let other_drain = other
        .scoped(async { queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN) })
        .await;
    assert_eq!(other_drain.requests.len(), 1);
    assert_eq!(other_drain.requests[0].reason, "question 0");
    assert_eq!(other_drain.discarded, 0);
}
