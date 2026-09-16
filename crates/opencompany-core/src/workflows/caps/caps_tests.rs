use super::*;

/// The single-harness turn over a fresh pool, as the non-lane entrypoint
/// wraps — what a workflow agent node runs on when no lanes are declared.
fn single_turn(deps: &HarnessDeps) -> Arc<dyn RunTurn> {
    Arc::new(crate::harness::built_in::run_turn::HarnessRunTurn::new(
        Arc::new(crate::harness::HarnessPool::new()),
        Arc::new(deps.clone()),
    ))
}

/// A [`RunTurn`] that records the workflow-route ids each
/// `run_background_workflow` call receives, standing in for the harness pool
/// so the #1702 dispatch test can assert the run and node ids actually reach
/// the turn rather than being silently dropped by a fallback to the
/// un-streamed `run_background`.
struct RecordingWorkflowTurn {
    /// `(agent_ref, workflow_run_id, node_id)` per call, in order.
    calls: std::sync::Mutex<Vec<(String, String, String)>>,
}

impl RecordingWorkflowTurn {
    fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
}

/// The shape every recorded turn answers with — the dispatch under test only
/// cares about the ids it is handed, not what the (absent) agent did.
fn ok_outcome() -> crate::harness::TurnOutcome {
    crate::harness::TurnOutcome {
        reply: "ok".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }
}

#[async_trait]
impl RunTurn for RecordingWorkflowTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(ok_outcome())
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(ok_outcome())
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(ok_outcome())
    }

    async fn run_background_workflow(
        &self,
        _company: &CompanyId,
        agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        workflow_run_id: &str,
        node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.calls.lock().expect("calls").push((
            agent_id.to_string(),
            workflow_run_id.to_string(),
            node_id.to_string(),
        ));
        Ok(ok_outcome())
    }
}

/// Issue #1702: the workflow agent-node dispatch routes through
/// `run_background_workflow`, not the un-streamed `run_background`, so the
/// node's live tool frames stream tagged with the run and node ids. This
/// pins the forward: a regression that swapped the arguments or fell back
/// to `run_background` would leave the node functional but its live
/// activity silently gone.
#[tokio::test]
async fn an_agent_node_dispatches_through_run_background_workflow_with_run_and_node_ids() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1702-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RecordingWorkflowTurn::new());
    let board_claim = Arc::new(deps.delegations.claim_board("run-1702"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1702"));
    let runner = HarnessAgentRunner::new(
        turn.clone(),
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1".to_string(),
        "run-1702".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    // A node resolved from the graph: `node_id` present, so the resolved
    // `lineage_node` is that id, and the turn must receive the runner's OWN
    // run id.
    let (_, outcome) = runner
        .run_turn(
            "researcher",
            json!({ "node_id": "gather", "prompt": "collect the numbers" }),
        )
        .await
        .expect("agent node turn");
    assert_eq!(outcome.reply, "ok");
    assert_eq!(
        turn.calls.lock().expect("calls").as_slice(),
        &[(
            "researcher".to_string(),
            "run-1702".to_string(),
            "gather".to_string(),
        )],
        "the node's live frames must be tagged with the runner's run id and the resolved node id"
    );

    // A node with no graph id (a hand-built request, or a graph compiled
    // before #881) resolves lineage to the agent ref — and the ids still
    // route through, tagged with that fallback.
    runner
        .run_turn("researcher", json!({ "prompt": "no node id" }))
        .await
        .expect("agent node turn without a node id");
    assert_eq!(
        turn.calls.lock().expect("calls").as_slice(),
        &[
            (
                "researcher".to_string(),
                "run-1702".to_string(),
                "gather".to_string(),
            ),
            (
                "researcher".to_string(),
                "run-1702".to_string(),
                "researcher".to_string(),
            ),
        ],
        "a node with no graph id resolves lineage to the agent ref"
    );
}

/// A turn double that answers every call by reporting it truncated at the
/// iteration cap (issue #1865) — the one signal `reclassify_capped_nodes`
/// keys off, so a fake this narrow is enough to drive the arm under test
/// without a scripted model.
struct CappedWorkflowTurn;

#[async_trait]
impl RunTurn for CappedWorkflowTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_background_workflow(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        _workflow_run_id: &str,
        _node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(crate::harness::TurnOutcome {
            reply: "partial answer, still going".to_string(),
            steps: Vec::new(),
            hit_iteration_cap: true,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
}

/// Issue #1865: the two halves of the disagreement the issue reports —
/// closed at their source. `run_turn` settles the attempt row `Failed` for
/// a capped turn (issue #926); this pins that the SAME turn also feeds
/// `RunCappedNodes`, the one channel `reclassify_capped_nodes` reads to
/// bring the run-level node row into agreement.
///
/// Not an end-to-end `run_workflow` proof (that would need the scripted
/// HTTP model `iteration_cap_turn_test` documents as the only way to
/// genuinely spend `max_tool_iterations`) — this pins the host-side HALF
/// of the mechanism this module owns: given the engine already told the
/// host "this turn was capped", both the attempt row and the sideways
/// channel agree about it. `runner::reclassify_capped_nodes`'s own test
/// pins the other half — that the channel's contents actually flip a
/// node's row from `Ok` to `Error`.
#[tokio::test]
async fn a_capped_turn_settles_failed_and_feeds_run_capped_nodes() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1865-capped-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(CappedWorkflowTurn);
    let board_claim = Arc::new(deps.delegations.claim_board("run-1865"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1865"));
    let capped = RunCappedNodes::default();
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1865".to_string(),
        "run-1865".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        capped.clone(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let (_, outcome) = runner
        .run_turn(
            "researcher",
            json!({ "node_id": "loop_step", "prompt": "keep going" }),
        )
        .await
        .expect("a capped turn is still Ok — the reply is a real, partial checkpoint");
    assert!(outcome.hit_iteration_cap);

    // Half 1: the sideways channel `reclassify_capped_nodes` reads.
    assert_eq!(
        capped.take(),
        vec!["loop_step".to_string()],
        "the capped node's id must reach the channel the runner reconciles against"
    );

    // Half 2: the attempt row this run's Observatory/task-detail surfaces
    // read — issue #926's pre-existing settle, pinned here so a future
    // change cannot decouple it from the #1865 signal above without a
    // test noticing.
    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1865".to_string()),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1, "one attempt for one node turn");
    assert_eq!(attempts[0].status, crate::ports::RunStatus::Failed);
    assert_eq!(
        attempts[0].error.as_deref(),
        Some("agent stopped at the max_tool_iterations cap before finishing")
    );
}

/// A turn double that reports truncation at the iteration cap, the same
/// shape as [`CappedWorkflowTurn`], for a node that also declares `verify`.
struct CappedVerifiedWorkflowTurn;

#[async_trait]
impl RunTurn for CappedVerifiedWorkflowTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_background_workflow(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        _workflow_run_id: &str,
        _node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(crate::harness::TurnOutcome {
            reply: "partial answer, still going".to_string(),
            steps: Vec::new(),
            hit_iteration_cap: true,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
}

/// Codex review on #1990 (issue #1866): a node whose turn truncated at the
/// iteration cap is still handed to the semantic judge when `verify` is
/// declared — and the judge is told `execution_failed: false` regardless,
/// so a judge that answers `halt_benign` for the truncated partial reply
/// was never caught by `enforce_anti_suppression`'s blank/failed guard.
/// Before the fix, a capped turn could be recorded as an intentional
/// benign stop instead of the truncated failure it actually is.
#[tokio::test]
async fn a_capped_turn_with_verify_is_never_recorded_as_a_benign_halt() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-capped-verify-")
        .tempdir()
        .expect("tempdir");
    let base_url = crate::workflows::gated_tool_turn_test::spawn_script(vec![
        crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"halt_benign\"}"),
    ])
    .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(CappedVerifiedWorkflowTurn);
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990v"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990v"));
    let halted = RunHaltedNodes::default();
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990v".to_string(),
        "run-1990v".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_halted(halted.clone());

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "loop_step",
                "prompt": "keep going",
                "verify": { "criteria": "must finish the report" }
            }),
        )
        .await;
    assert!(
        result.is_err(),
        "a truncated turn must never be accepted as semantically sufficient"
    );
    assert!(
        halted.take().is_empty(),
        "a capped/truncated turn must never be recorded as an intentional benign halt, \
         regardless of what the judge answers"
    );
}

/// Codex review on #1990 (issue #1866): `RunHaltedNodes` is shared across
/// every attempt `HarnessAgentRunner` makes for a run, and tinyflows
/// re-runs a node's whole turn when `retry.max_attempts > 1`. This drives
/// the exact sequence: attempt 1's judge answers `halt_benign` (pushing the
/// node id), attempt 2 (the retry) succeeds outright. Without retracting
/// the stale entry, `reclassify_halted_nodes` would relabel attempt 2's
/// genuinely successful row `Declined` using a marker left over from the
/// attempt that failed.
#[tokio::test]
async fn a_later_successful_attempt_is_not_shadowed_by_an_earlier_benign_halt() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-retry-halt-")
        .tempdir()
        .expect("tempdir");
    let base_url = crate::workflows::gated_tool_turn_test::spawn_script(vec![
        crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"halt_benign\"}"),
        crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
    ])
    .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RecordingWorkflowTurn::new());
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990h"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990h"));
    let halted = RunHaltedNodes::default();
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990h".to_string(),
        "run-1990h".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_halted(halted.clone());
    let node = json!({
        "node_id": "flaky",
        "prompt": "go",
        "verify": { "criteria": "must finish" }
    });

    let first = runner.run_turn("researcher", node.clone()).await;
    assert!(
        first.is_err(),
        "attempt 1's halt_benign verdict must gate the node"
    );
    assert!(
        halted.contains("flaky"),
        "attempt 1 must record the benign halt while it is the node's only outcome"
    );

    let second = runner.run_turn("researcher", node).await;
    assert!(
        second.is_ok(),
        "attempt 2's continue verdict must let the node through"
    );
    assert!(
        !halted.contains("flaky"),
        "attempt 2 succeeded outright; attempt 1's stale benign-halt marker must not survive \
         to shadow it"
    );
}

/// Codex review on #1990 (issue #1866): the agent's turn is composed from
/// the node's static instruction AND the operator's run-specific request
/// (`compose_turn_message`, issue #154) — but the judge was handed only the
/// static instruction. A reusable node's `verify.criteria` can only be
/// checked against what was actually asked this run; passing the judge the
/// pre-compose instruction meant it evaluated a different, narrower prompt
/// than the one the agent answered.
#[tokio::test]
async fn the_judge_sees_the_operators_run_request_not_just_the_static_instruction() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-run-request-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RecordingWorkflowTurn::new());
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990r"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990r"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990r".to_string(),
        "run-1990r".to_string(),
        Some("check tuesday's numbers".to_string()),
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "reusable",
                "prompt": "Summarize the report.",
                "verify": { "criteria": "must call out tuesday's numbers specifically" }
            }),
        )
        .await
        .expect("a `continue` verdict must not gate the node");

    let seen = script.seen.lock().expect("seen");
    assert_eq!(
        seen.len(),
        1,
        "only the judge calls the scripted model here"
    );
    let sent = seen[0].to_string();
    assert!(
        sent.contains("check tuesday's numbers"),
        "the judge's prompt must carry the operator's run-specific request, not just the \
         node's static instruction: {sent}"
    );
    assert!(
        sent.contains("Request for this run:"),
        "the judge's prompt must use the same composed shape the agent's own turn ran on: {sent}"
    );
}

/// A turn double that always answers with a fixed refusal reply — a node
/// whose agent could not complete the ask, the shape a `recover` verdict is
/// meant to rescue.
struct RefusalWorkflowTurn;

#[async_trait]
impl RunTurn for RefusalWorkflowTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat_id: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_background_workflow(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        _workflow_run_id: &str,
        _node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(crate::harness::TurnOutcome {
            reply: "I cannot draft the email without the customer's name.".to_string(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
}

/// A [`FactStore`] that always answers `list` with one fixed fact,
/// regardless of the query — standing in for a real match so `ask_around`
/// always has evidence to offer.
struct OneFactStore;

#[async_trait]
impl crate::ports::FactStore for OneFactStore {
    async fn list(
        &self,
        _company: &CompanyId,
        _query: Option<&str>,
        _kind: Option<crate::ports::FactKind>,
    ) -> crate::Result<Vec<crate::ports::FactRecord>> {
        Ok(vec![crate::ports::FactRecord {
            id: "f1".to_string(),
            kind: crate::ports::FactKind::Fact,
            title: "Company context".to_string(),
            body: "irrelevant background, not the customer's name".to_string(),
            source: "test".to_string(),
            updated_at_millis: 0,
        }])
    }

    async fn upsert(
        &self,
        _company: &CompanyId,
        _fact: &crate::ports::FactRecord,
    ) -> crate::Result<()> {
        unreachable!("not exercised by this test")
    }

    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unreachable!("not exercised by this test")
    }
}

/// Codex review on #1990 (issue #1866, #3903874673): found evidence is not
/// itself proof the gap closed. Before the fix, ANY evidence — however
/// unrelated — was appended to a refusal's own reply and the node settled
/// `Succeeded` without ever re-checking whether the augmented text now
/// actually answers the ask. Here the "recovered" fact is deliberately
/// irrelevant to the missing customer name, so a correct re-verify must
/// still refuse to accept the node.
#[tokio::test]
async fn recovered_evidence_that_does_not_close_the_gap_is_not_accepted() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-recover-reverify-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"retry\"}"),
        ])
        .await;
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    deps.facts = Some(Arc::new(OneFactStore));
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RefusalWorkflowTurn);
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990g"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990g"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990g".to_string(),
        "run-1990g".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft",
                "prompt": "Draft the customer email.",
                "verify": { "criteria": "must include the customer's name" }
            }),
        )
        .await;

    assert!(
        result.is_err(),
        "irrelevant recovered evidence must not turn a refusal into a success"
    );
    let seen = script.seen.lock().expect("seen");
    assert_eq!(
        seen.len(),
        2,
        "the judge must be asked again about the augmented output, not just once up front"
    );
    let reverify_prompt = seen[1].to_string();
    assert!(
        reverify_prompt.contains("Recovered company context"),
        "the second judge call must see the augmented output, not the original refusal alone: \
         {reverify_prompt}"
    );
}

/// The text a node ships after a successful recovery must be the exact
/// text the re-verification judge was shown. `augment_with_recovery`
/// bounds the reply so the evidence survives the judge's output window;
/// a separately composed, unbounded string would let an oversized reply
/// ship content the judge never read.
#[tokio::test]
async fn a_recovered_reply_ships_the_exact_text_the_judge_certified() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-recover-certified-text-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    deps.facts = Some(Arc::new(OneFactStore));
    let record = crate::workflows::gated_tool_turn_test::record();
    let oversized = "R".repeat(25_000);
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: oversized.clone(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990h"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990h"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990h".to_string(),
        "run-1990h".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (_value, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft",
                "prompt": "Draft the customer email.",
                "verify": { "criteria": "must include the customer's name" }
            }),
        )
        .await
        .expect("a `continue` re-verdict accepts the recovered reply");

    let judged = script.seen.lock().expect("seen")[1].to_string();
    let escaped = serde_json::to_string(&outcome.reply).expect("reply serializes");
    assert!(
        judged.contains(escaped.trim_matches('"')),
        "the reply stored on the outcome must be the same text the re-verification judge \
         read, but the judge never saw it ({} stored chars)",
        outcome.reply.chars().count()
    );
    assert!(
        outcome.reply.chars().count() < oversized.chars().count(),
        "the fixture must be large enough that recovery augmentation has to bound it, \
         otherwise this test cannot observe the divergence"
    );
}

/// A node declaring both a postcondition and a verify criteria emits one
/// output with two views of it: `value["text"]` and the JSON fields
/// merged into `value`. Recovery rewrites the reply, so both views must
/// describe the rewritten reply — a merge carrying the pre-recovery parse
/// would let a downstream `=item.json.<field>` binding read fields that
/// the shipped text no longer backs.
#[tokio::test]
async fn a_recovered_reply_does_not_emit_its_pre_recovery_json_parse() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-recover-stale-parse-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    deps.facts = Some(Arc::new(OneFactStore));
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"draft\": \"no customer name yet\"}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990i"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990i"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990i".to_string(),
        "run-1990i".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, _outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft",
                "prompt": "Draft the customer email.",
                "postcondition": { "require": "non_empty" },
                "verify": { "criteria": "must include the customer's name" }
            }),
        )
        .await
        .expect("a `continue` re-verdict accepts the recovered reply");

    assert!(
        value["text"]
            .as_str()
            .expect("text is a string")
            .contains("Recovered company context"),
        "the emitted text must be the recovered reply: {}",
        value["text"]
    );
    assert!(
        value.get("draft").is_none(),
        "the pre-recovery parse must not ship alongside a reply that no longer carries it: \
         {value}"
    );
}

/// `augment_with_recovery` always appends a `Recovered company context:`
/// prose block to the reply, so a reply that satisfied a declared
/// `field_present` postcondition before recovery (its JSON parsed and
/// carried the field) stops satisfying it after (the augmented text no
/// longer parses as JSON at all). A node must not settle `Succeeded`
/// carrying an output that no longer satisfies the postcondition its own
/// gate certified — the recovered reply is re-checked against the same
/// postcondition, and a node whose recovery breaks it fails instead of
/// silently shipping the field as absent.
#[tokio::test]
async fn a_recovered_reply_that_fails_its_postcondition_does_not_settle_succeeded() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-recover-postcondition-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    deps.facts = Some(Arc::new(OneFactStore));
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"draft\": \"no customer name yet\"}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990j"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990j"));
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990j".to_string(),
        "run-1990j".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft",
                "prompt": "Draft the customer email.",
                "postcondition": { "require": "field_present", "field": "json.draft" },
                "verify": { "criteria": "must include the customer's name" }
            }),
        )
        .await;

    assert!(
        result.is_err(),
        "a recovered reply that no longer satisfies its declared postcondition must not \
         settle Succeeded"
    );

    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1990j".to_string()),
        )
        .await
        .expect("list attempts");
    let statuses: Vec<_> = attempts.iter().map(|a| a.status).collect();
    assert_eq!(
        statuses,
        vec![crate::ports::RunStatus::Failed],
        "the recovered-but-noncompliant node must settle Failed, not Succeeded: {statuses:?}"
    );
}

/// A provider that always returns the `recover` verdict, driving the
/// judge's recovery-then-park branch.
#[derive(Default)]
struct RecoverJudgeProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl tinyinference::model::ChatModel<()> for RecoverJudgeProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: tinyinference::model::ModelRequest,
    ) -> tinyinference::Result<tinyinference::model::ModelResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(tinyinference::model::ModelResponse::assistant(
            "{\"verdict\":\"recover\"}".to_string(),
        ))
    }
}

impl crate::harness::provider::HarnessModel for RecoverJudgeProvider {
    fn telemetry_provider_id(&self) -> String {
        "recover-judge".to_string()
    }
}

/// tinysweeper on #1990 (#3905096415) read the recover branch as settling
/// `Blocked` and then having an outer handler overwrite it with `Failed`.
/// `run_turn` has no such handler — every settle is followed by an
/// immediate `return Err`, and the trailing settle is the fall-through
/// success path — so the parked row stays `Blocked`. Pinned here so a
/// future outer error handler cannot silently introduce the overwrite.
#[tokio::test]
async fn a_recovery_park_leaves_the_attempt_row_blocked() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-recover-park-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    deps.provider = Arc::new(RecoverJudgeProvider::default());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "I cannot draft this without the customer's renewal date".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-recover"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-recover"));
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-recover".to_string(),
        "run-recover".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft_step",
                "prompt": "draft the renewal email",
                "verify": { "criteria": "the email must name the renewal date" },
            }),
        )
        .await;
    assert!(result.is_err(), "a parked node halts its branch");

    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-recover".to_string()),
        )
        .await
        .expect("list attempts");
    let statuses: Vec<_> = attempts.iter().map(|a| a.status).collect();
    assert_eq!(
        statuses,
        vec![crate::ports::RunStatus::Blocked],
        "the parked node must be recorded Blocked, not overwritten with Failed"
    );
}

/// A provider that counts every `invoke` and always escalates, so a judge
/// call is both detectable and destructive to the caller's diagnosis.
#[derive(Default)]
struct EscalatingJudgeProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl tinyinference::model::ChatModel<()> for EscalatingJudgeProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: tinyinference::model::ModelRequest,
    ) -> tinyinference::Result<tinyinference::model::ModelResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(tinyinference::model::ModelResponse::assistant(
            "{\"verdict\":\"escalate\",\"gap\":\"information\"}".to_string(),
        ))
    }
}

impl crate::harness::provider::HarnessModel for EscalatingJudgeProvider {
    fn telemetry_provider_id(&self) -> String {
        "escalating-judge".to_string()
    }
}

/// Codex review on #1990 (#3905537805): a turn refused by its per-agent
/// spend cap is rejected by the `LimitStop` path regardless, so paying for
/// a judge on the pause notice buys nothing — and an `escalate` verdict
/// returns early with a generic blocker, replacing the budget-pause
/// diagnosis the operator needs with "needs information intervention".
#[tokio::test]
async fn a_budget_paused_turn_skips_the_sufficiency_judge() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-budget-paused-judge-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let provider = Arc::new(EscalatingJudgeProvider::default());
    deps.provider = provider.clone();
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "paused — out of budget".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: Some(crate::harness::BudgetPause {
            agent: "researcher".to_string(),
            summary: "acme is out of inference credits".to_string(),
        }),
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1990"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990".to_string(),
        "run-1990".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "spend_step",
                "prompt": "keep going",
                "verify": { "criteria": "the report must be sent" },
            }),
        )
        .await;

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a budget-paused turn must not pay for a sufficiency judge"
    );
    let (_, outcome) = result.expect(
        "the budget-pause diagnosis must survive: the judge must not turn this into a \
         generic information blocker",
    );
    assert!(outcome.budget_paused.is_some());
}

/// PR #1883 review (Codex #3874941288): the sibling of
/// `a_capped_turn_settles_failed_and_feeds_run_capped_nodes` for the OTHER
/// signal that settles this attempt row `Failed` — `outcome.budget_paused`.
/// Before this fix, only `hit_iteration_cap` fed `RunCappedNodes`, so
/// `reclassify_capped_nodes` never saw a budget-paused node's id and its
/// row stayed `Ok` even though the attempt was `Failed` — the exact
/// disagreement #1865 exists to close, just via the other cap.
#[tokio::test]
async fn a_budget_paused_turn_settles_failed_and_feeds_run_capped_nodes() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1883-budget-paused-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "paused — out of budget".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: Some(crate::harness::BudgetPause {
            agent: "researcher".to_string(),
            summary: "acme is out of inference credits".to_string(),
        }),
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1883"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1883"));
    let capped = RunCappedNodes::default();
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1883".to_string(),
        "run-1883".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        capped.clone(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let (_, outcome) = runner
        .run_turn(
            "researcher",
            json!({ "node_id": "spend_step", "prompt": "keep going" }),
        )
        .await
        .expect("a budget-paused turn is still Ok — the reply is a real, partial checkpoint");
    assert!(outcome.budget_paused.is_some());

    // Half 1: the sideways channel `reclassify_capped_nodes` reads. This
    // is the assertion that failed before the fix — `capped.take()` came
    // back empty because only `hit_iteration_cap` pushed to it.
    assert_eq!(
        capped.take(),
        vec!["spend_step".to_string()],
        "the budget-paused node's id must reach the channel the runner reconciles \
         against, the same as a capped node's"
    );

    // Half 2: the attempt row this run's Observatory/task-detail surfaces
    // read, pinned here so it cannot drift from the #1865 signal above.
    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1883".to_string()),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1, "one attempt for one node turn");
    assert_eq!(attempts[0].status, crate::ports::RunStatus::Failed);
    assert_eq!(
        attempts[0].error.as_deref(),
        Some(
            "agent paused for lack of inference budget/credits: acme is out of inference credits"
        )
    );
}

#[tokio::test]
async fn a_spend_halted_turn_skips_the_judge_and_settles_failed() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-spend-halted-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let provider = Arc::new(EscalatingJudgeProvider::default());
    deps.provider = provider.clone();
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "here is what I found so far".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: Some(crate::harness::SpendHalt {
            agent: "researcher".to_string(),
            spent_usd: 5.25,
            cap_usd: 5.0,
        }),
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1990-spend"));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run("run-1990-spend"),
    );
    let capped = RunCappedNodes::default();
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1990-spend".to_string(),
        "run-1990-spend".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        capped.clone(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let (_, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "spend_step",
                "prompt": "keep going",
                "verify": { "criteria": "the report must be sent" },
            }),
        )
        .await
        .expect("a spend-halted turn is still Ok — the reply is a real, partial checkpoint");

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a spend-halted turn must not pay for a sufficiency judge"
    );
    assert!(outcome.halted_for_spend.is_some());

    assert_eq!(
        capped.take(),
        vec!["spend_step".to_string()],
        "the spend-halted node's id must reach the channel the runner reconciles against"
    );

    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1990-spend".to_string()),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1, "one attempt for one node turn");
    assert_eq!(attempts[0].status, crate::ports::RunStatus::Failed);
    assert!(
        attempts[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("spend cap partway through")),
        "the spend-halt diagnosis must survive to the attempt row, got {:?}",
        attempts[0].error
    );
}

/// A [`RunTurn`] that always answers with a scripted outcome — standing in
/// for an ACP-backed harness whose turn stopped abnormally, without
/// needing a real ACP subprocess to produce one.
struct ScriptedTurn(crate::harness::TurnOutcome);

#[async_trait]
impl RunTurn for ScriptedTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(self.0.clone())
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(self.0.clone())
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Ok(self.0.clone())
    }
}

/// PR #1880 review: "Propagate abnormal ACP stops beyond step notes." The
/// gap was that `HarnessAgentRunner::run_turn` read only
/// `hit_iteration_cap`, which stays `false` on an ACP `refusal`,
/// `cancelled`, or unrecognized `stopReason` — so the node settled
/// `Succeeded` here and `run` (the `AgentRunner` impl below) reported
/// `StopReason::Finished`, indistinguishable from the agent having
/// actually answered.
///
/// Asserted on the **outcome**, not on whether a `Note` step exists —
/// `harness::acp::run_turn::fold` already put a note on the timeline
/// before this fix, and the finding was explicitly that the note alone
/// does not stop the workflow graph from advancing as if the turn
/// succeeded. This is that stronger claim: the node call itself must
/// fail.
#[tokio::test]
async fn an_abnormal_acp_stop_fails_the_workflow_node() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1880-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "I can't help with that.".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: Some("[stopped: the agent declined to continue]".to_string()),
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1880"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1880"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1".to_string(),
        "run-1880".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let result = runner
        .run_turn("responder", json!({ "prompt": "do the thing" }))
        .await;

    let err = result.expect_err(
        "a refused/cancelled/unrecognized ACP stop must fail the node, \
         not settle it Succeeded/Finished",
    );
    let message = err.to_string();
    assert!(
        message.contains("the agent declined to continue"),
        "the error must carry the abnormal-stop reason, not a generic failure: {message}"
    );
}

/// Issue #1866 (deterministic tier) — the RED-on-old proof. A capped
/// turn's partial reply already settles the attempt row `Failed` (issue
/// #1865, pinned above), but on the pre-#1866 `run_turn` it still returns
/// `Ok` and flows the truncated text downstream via `=items` — nothing
/// stops it. Declaring a `postcondition` this same output fails must
/// ALSO turn the return into `Err`, so nothing downstream ever binds it.
///
/// Reuses [`CappedWorkflowTurn`] — its `{ "text": "partial answer, still
/// going", "agent_ref": ... }` envelope has no `items` field, so
/// `field_present` on `items` is exactly the gap this node's truncated
/// output represents. On the code as it stood before this issue, this
/// assertion fails: `run_turn` returns `Ok` here (see the sibling test
/// above, which asserts `.expect(...)` on the identical outcome).
#[tokio::test]
async fn a_node_whose_postcondition_fails_halts_before_returning_ok() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-postcondition-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(CappedWorkflowTurn);
    let board_claim = Arc::new(deps.delegations.claim_board("run-1866"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1866"));
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1866".to_string(),
        "run-1866".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    )
    .with_runs(Some(runs.clone()), None, RunAttempts::default());

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "loop_step",
                "prompt": "keep going",
                "postcondition": { "require": "field_present", "field": "items" }
            }),
        )
        .await;

    let err = result.expect_err(
        "a truncated reply that also fails its declared postcondition must halt — \
         this is the RED-on-old assertion: pre-#1866 code returns Ok here",
    );
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("items"),
        "the halting message should name what the output was missing: {message}"
    );

    // The ordinary failure bucket, not `WaitingApproval` — nobody has to
    // approve a bad output the way they approve a gated tool call.
    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1866".to_string()),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1, "one attempt for one node turn");
    assert_eq!(attempts[0].status, crate::ports::RunStatus::Failed);
}

/// Companion GREEN: a node with no `postcondition` declared is completely
/// unaffected — the exact back-compat contract every other first-class
/// field on this call site keeps (`on_error`, `retry`,
/// `requires_approval`). Reuses the ordinary `RecordingWorkflowTurn` /
/// `ok_outcome` fixture the #1702 dispatch test above already trusts.
#[tokio::test]
async fn a_node_with_no_postcondition_is_unaffected() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-no-postcondition-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RecordingWorkflowTurn::new());
    let board_claim = Arc::new(deps.delegations.claim_board("run-1866b"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1866b"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1866b".to_string(),
        "run-1866b".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, outcome) = runner
        .run_turn("researcher", json!({ "node_id": "plain", "prompt": "go" }))
        .await
        .expect("a node with no postcondition must not be gated at all");
    assert_eq!(outcome.reply, "ok");
    assert_eq!(value["text"], "ok");
}

/// Companion GREEN: an output that DOES satisfy its declared
/// postcondition returns `Ok` exactly as an ungated node would — the gate
/// only ever removes a path, never adds one for output that clears it.
#[tokio::test]
async fn a_satisfying_output_still_returns_ok() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-satisfying-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(RecordingWorkflowTurn::new());
    let board_claim = Arc::new(deps.delegations.claim_board("run-1866c"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1866c"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1866c".to_string(),
        "run-1866c".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    // `RecordingWorkflowTurn::ok_outcome` replies "ok" — non-empty, so
    // `non_empty` is satisfied and the turn proceeds exactly as if no
    // postcondition were declared at all.
    let (value, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "plain",
                "prompt": "go",
                "postcondition": { "require": "non_empty" }
            }),
        )
        .await
        .expect("an output that satisfies its postcondition must not be halted");
    assert_eq!(outcome.reply, "ok");
    assert_eq!(value["text"], "ok");
}

/// Codex review on #1937 (issue #1866) — the RED-on-old proof for
/// `non_empty_list`. The postcondition envelope this call site built was
/// always `{ "text": <reply>, "agent_ref": <ref> }`: an object, never a
/// `Value::Array`, so a `require = "non_empty_list"` declaration with no
/// `field` could never be satisfied by ANY agent reply — including a
/// reply that is itself the literal JSON text of a non-empty list, which
/// is exactly what this test sends. On the code as it stood before this
/// fix, this assertion fails: `run_turn` returns `Err` here because the
/// envelope's `json` never carried the agent's parsed reply.
///
/// Updated for Codex #3893541856 (bare-array emission): the emitted
/// `value` is now the array itself, not an object with a `text` key — see
/// `a_bare_array_reply_replaces_the_emitted_value_wholesale` below for the
/// dedicated coverage of that shape. `outcome.reply` (a separate field,
/// untouched by any of this) still carries the raw string regardless.
#[tokio::test]
async fn a_reply_that_is_a_json_list_satisfies_non_empty_list_with_no_field() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-postcondition-list-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "[\"x\", \"y\"]".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937".to_string(),
        "run-1937".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "list two things",
                "postcondition": { "require": "non_empty_list" }
            }),
        )
        .await
        .expect(
            "a reply that IS the JSON text of a non-empty list must satisfy \
             `non_empty_list` with no `field` — this is the RED-on-old assertion: \
             pre-fix code always built a `{text, agent_ref}` envelope that could \
             never be seen as a `Value::Array`",
        );
    assert_eq!(outcome.reply, "[\"x\", \"y\"]");
    assert_eq!(value, json!(["x", "y"]));
}

/// Companion: a plain-prose reply (the common case — agent nodes are not
/// asked for structured output by default) still fails `non_empty_list`
/// honestly, rather than the fix silently passing everything through
/// once a `json` key exists on the envelope.
#[tokio::test]
async fn a_prose_reply_still_fails_non_empty_list_with_no_field() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-postcondition-prose-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "here is a summary, not a list".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937b"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937b"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937b".to_string(),
        "run-1937b".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "list two things",
                "postcondition": { "require": "non_empty_list" }
            }),
        )
        .await;

    let err = result.expect_err(
        "a plain-prose reply must still fail `non_empty_list` — the fix must not \
         silently pass every reply once the envelope carries a `json` key",
    );
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("not a list"),
        "the halting message should say the shape did not match: {message}"
    );
}

/// CodeRabbit review on #1937 (issue #1866) — confirms the fix covers
/// `field_present` with the documented `json.items` dotted path, not just
/// `non_empty_list`'s no-field form (the two are fixed by the same
/// envelope change: the reply is best-effort JSON-parsed into a `json`
/// key, and `field_present`'s existing dotted-path resolution reaches it
/// like any other nested object). On the code as it stood before the fix,
/// this assertion fails: the envelope carried no `json` key at all, so
/// `json.items` could never resolve.
#[tokio::test]
async fn a_reply_that_is_json_satisfies_field_present_on_a_json_dotted_path() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-postcondition-field-present-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"items\": [1, 2, 3]}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937c"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937c"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937c".to_string(),
        "run-1937c".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "reply with a JSON object naming items",
                "postcondition": { "require": "field_present", "field": "json.items" }
            }),
        )
        .await
        .expect(
            "a reply that IS a JSON object carrying `items` must satisfy \
             `field_present` on the documented `json.items` path",
        );
    assert_eq!(outcome.reply, "{\"items\": [1, 2, 3]}");
    assert_eq!(value["text"], "{\"items\": [1, 2, 3]}");
}

/// Codex review on #1937 (issue #1866) — the emitted-value companion to
/// the test above: `value` (the tuple's first element) is exactly what
/// `run`'s `AgentRunOutcome.json` becomes (`json: value.clone()`, a few
/// lines below this call site), which tinyflows' `finish_agent_run`
/// (`nodes/integration/agent.rs`) then lands unchanged at the item
/// envelope's `json` whenever it is an `Object`/`Array` — i.e. `value`
/// literally IS what a downstream `=item.json.<field>` binding reads.
/// Before merging `parsed_reply`'s fields into `value` (Codex
/// #3893330383), this was `{"text": ..., "agent_ref": ...}` regardless of
/// what the reply parsed to, so the gate above could pass while
/// `value["items"]` (and therefore `item.json.items` downstream) stayed
/// absent. See `a_structured_agent_reply_is_readable_by_a_downstream_json_binding`
/// in `workflows::runner` for the same claim proven through a real
/// two-node graph with an actual `=item.json.items` expression, not just
/// this unit-level inspection of the returned tuple.
#[tokio::test]
async fn the_parsed_reply_lands_in_the_emitted_value_a_downstream_binding_reads() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-postcondition-emitted-value-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"items\": [1, 2, 3]}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937d"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937d"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937d".to_string(),
        "run-1937d".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, _outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "reply with a JSON object naming items",
                "postcondition": { "require": "field_present", "field": "json.items" }
            }),
        )
        .await
        .expect("the postcondition is satisfied, so the turn must succeed");

    // `text`/`agent_ref` must survive the merge unchanged — delivery.rs's
    // report_text reads a delivered report's body via `item.json.text`
    // and must keep finding the raw reply string here, not the parsed
    // object's own (absent, in this reply) `text` key.
    assert_eq!(value["text"], "{\"items\": [1, 2, 3]}");
    assert_eq!(value["agent_ref"], "researcher");
    // The actual finding: the SAME value `field = "json.items"` certified
    // above must also be readable off the emitted value a downstream
    // binding sees.
    assert_eq!(value["items"], json!([1, 2, 3]));
}

/// CodeRabbit #3893565788 on #1937 — the "blast radius" proof. A node
/// with NO declared postcondition, whose reply happens to be valid JSON,
/// must emit the exact `{text, agent_ref}` shape it always has — the
/// merge must never run for a node that did not opt into structured
/// output evaluation. On the code as it stood right after the
/// #3893330383 fix (before this scoping), this assertion fails:
/// `revenue` would appear as a top-level key in `value`, changing the
/// output contract for every agent node in every existing workflow that
/// happens to reply with a JSON object, whether or not it ever declared
/// a postcondition.
#[tokio::test]
async fn a_reply_that_parses_as_json_is_not_merged_without_a_declared_postcondition() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-no-postcondition-json-reply-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"revenue\": 12000, \"text\": \"ignored\"}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937e"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937e"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937e".to_string(),
        "run-1937e".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    // No `postcondition` key at all — the ordinary, overwhelmingly common
    // case: an agent node nobody ever asked to declare a run-safety gate.
    let (value, outcome) = runner
        .run_turn(
            "researcher",
            json!({ "node_id": "analyst", "prompt": "give me the numbers" }),
        )
        .await
        .expect("a node with no postcondition must not be gated at all");

    assert_eq!(outcome.reply, "{\"revenue\": 12000, \"text\": \"ignored\"}");
    assert_eq!(
        value,
        json!({
            "text": "{\"revenue\": 12000, \"text\": \"ignored\"}",
            "agent_ref": "researcher",
        }),
        "a node with no declared postcondition must emit exactly {{text, agent_ref}} \
         regardless of what the reply parses as — no `revenue` key, and `text` must \
         stay the raw reply string, not the parsed object's own `text` value: {value}"
    );
}

/// Codex #3893541856 on #1937 — the bare-array companion to the object
/// merge above. A node whose declared `non_empty_list` (no `field`)
/// passes against a bare JSON-array reply must emit that array itself as
/// `value`, not the `{text, agent_ref}` wrapper the gate never validated
/// — otherwise a downstream `=item.json` binding (reading the whole
/// value, not a dotted field into it) resolves to the wrapper instead of
/// the array the gate certified, reproducing the exact defect
/// #3893330383 fixed for the object case.
#[tokio::test]
async fn a_bare_array_reply_replaces_the_emitted_value_wholesale() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-bare-array-emission-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "[\"x\", \"y\"]".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937f"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937f"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937f".to_string(),
        "run-1937f".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let (value, outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "list two things",
                "postcondition": { "require": "non_empty_list" }
            }),
        )
        .await
        .expect("a reply that IS a non-empty JSON array must satisfy non_empty_list");

    // The gate certified the array; the emitted value must literally BE
    // that array — not an object wrapping it, and not the old
    // `{text, agent_ref}` shape.
    assert_eq!(value, json!(["x", "y"]));
    // The raw reply string is still available independently: `outcome`
    // (a distinct field from `value`) and `AgentRunOutcome.text` (built
    // from `outcome.reply` directly, not from `value`) both still carry
    // it — nothing that reads the prose loses it.
    assert_eq!(outcome.reply, "[\"x\", \"y\"]");
}

/// Codex #3894162757 on #1937 — supersedes a prior round's
/// `a_bare_scalar_reply_replaces_the_emitted_value_wholesale`, which
/// asserted `run_turn`'s OWN return value and never noticed that
/// tinyflows nulls a bare scalar one layer further out (see the doc
/// comment on the removed `Value::Bool(_) | Value::Number(_) |
/// Value::String(_)` emission arm, and
/// `workflows::runner::tests::a_scalar_reply_cannot_satisfy_field_present_on_the_bare_json_root`
/// for the full-graph proof of the delivery gap that test missed).
/// `field_present` on the bare `field = "json"` root can now never
/// pass for a scalar reply — the gate refuses to certify a shape it
/// knows cannot reach a downstream `=item.json` binding.
#[tokio::test]
async fn a_bare_scalar_reply_fails_field_present_on_the_bare_json_root() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-bare-scalar-rejected-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "42".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937g"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937g"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937g".to_string(),
        "run-1937g".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let result = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "scorer",
                "prompt": "reply with a single confidence score",
                "postcondition": { "require": "field_present", "field": "json" }
            }),
        )
        .await;

    let err = result.expect_err(
        "a bare scalar reply (`42`) must NOT satisfy field_present on the bare \
         `json` root — tinyflows can never deliver a scalar through \
         `=item.json` (it normalizes anything but Object/Array to null), so \
         certifying it would pass a gate whose value the workflow can never \
         actually read",
    );
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("json") && message.contains("scalar"),
        "the halting message should say why a scalar under `json` cannot \
         satisfy this gate: {message}"
    );
}

/// Codex #3894038816 on #1937 — the silent-disable finding, traced
/// end-to-end rather than inferred. `postcondition` rides inside the
/// engine-resolved node config (`translate_node` writes it as an
/// ordinary config key, same as `on_error`/`retry` — see the module doc
/// above the function), so `tinyflows::expr::resolve` — the SAME
/// resolution `nodes::execution::resolve_config_traced` runs on the
/// whole node config before an agent node's turn — walks straight into
/// it. An authored `field = "=item.missing"` is an ordinary
/// `=`-expression as far as that resolver is concerned; it does not know
/// or care that this particular leaf is a safety policy rather than
/// ordinary data.
///
/// Step 1 below proves `translate()` carries the expression through
/// UNRESOLVED (translation is not where resolution happens). Step 2
/// proves the mechanism concretely: running the real
/// `tinyflows::expr::resolve` against a scope whose `item` genuinely
/// lacks `missing` (the ordinary case the author meant to catch) turns
/// `postcondition.field` into a plain `Value::Null` — indistinguishable,
/// at that point, from no `field` having been authored at all. Step 3
/// feeds exactly that resolved shape to `run_turn`.
///
/// `field = "=item.missing"` cannot reach this point through
/// `parse_workflow` today — `workflow_file::validate`'s bare-structured-
/// root check (`postcondition_field_with_a_bare_structured_root_is_rejected`)
/// rejects it as a byproduct, since no `=`-expression's first dotted
/// segment can ever equal `json`/`text`/`agent_ref`. This test builds the
/// node directly instead (the same technique
/// `agent_ref_survives_a_spoofing_config` in `workflows::translate` uses)
/// to isolate the SECOND, independent layer: `evaluate_postcondition`
/// must not silently pass just because *something upstream* — this
/// resolution step today, a future one tomorrow — turned a validated
/// `field` into null before `run_turn` ever saw it.
///
/// RED on the code as it stood before the `evaluate_postcondition` fix:
/// `run_turn` returned `Ok`, for a reply ("just prose, no items here")
/// that plainly satisfies nothing — the gate silently did not run.
#[tokio::test]
async fn a_field_resolved_away_by_an_authored_expression_fails_closed_at_run_turn() {
    use crate::company::{
        WorkflowFile, WorkflowNodeDef, WorkflowNodeKind, WorkflowPostconditionDef,
    };
    use crate::workflows::translate::translate;

    // Step 1 — author `field = "=item.missing"` directly on the model
    // (bypassing `parse_workflow`/`validate`, per the doc comment above),
    // and confirm `translate()` carries it through as the literal
    // expression string — translation does not resolve expressions.
    let file = WorkflowFile {
        global: false,
        id: "wf".into(),
        name: "WF".into(),
        description: None,
        owner_desk: None,
        nodes: vec![WorkflowNodeDef {
            id: "worker".into(),
            kind: WorkflowNodeKind::Agent,
            name: "Worker".into(),
            summary: None,
            agent: Some("researcher".into()),
            schedule: None,
            config: None,
            on_error: None,
            retry: None,
            requires_approval: None,
            repeatable: None,
            destination: None,
            postcondition: Some(WorkflowPostconditionDef {
                require: "field_present".to_string(),
                field: Some("=item.missing".to_string()),
            }),
            verify: None,
        }],
        edges: Vec::new(),
    };
    let graph = translate(&file);
    let node_config = graph.nodes[0].config.clone();
    assert_eq!(
        node_config["postcondition"]["field"], "=item.missing",
        "translate() must carry the authored expression through UNRESOLVED —              it is config resolution, not translate(), that evaluates it"
    );

    // Step 2 — run the SAME resolution the engine runs
    // (`tinyflows::nodes::execution::resolve_config_traced` calls
    // `tinyflows::expr::resolve` on the whole config tree) against a
    // scope whose `item` genuinely has no `missing` key — the ordinary
    // case `=item.missing` exists to catch.
    let scope = json!({ "item": { "other_field": "present, but not the missing key" } });
    let resolved_config = tinyflows::expr::resolve(&node_config, &scope);
    assert_eq!(
        resolved_config["postcondition"]["field"],
        Value::Null,
        "traced: config resolution turns the authored `=item.missing` into a              plain JSON null before run_turn ever sees it"
    );

    // Step 3 — feed exactly that resolved postcondition to `run_turn`,
    // with a reply that plainly does not satisfy any real check.
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-expression-field-resolved-away-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "just prose, no items here".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937h"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937h"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937h".to_string(),
        "run-1937h".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let request = json!({
        "node_id": "worker",
        "prompt": "say something",
        "postcondition": resolved_config["postcondition"].clone(),
    });

    let result = runner.run_turn("researcher", request).await;

    let err = result.expect_err(
        "a postcondition whose `field` resolved away to null must halt the node —              the gate silently not running is worse than the gate certifying the wrong              value",
    );
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("field_present"),
        "the halting message should name the predicate that could not be              evaluated: {message}"
    );
}

/// Codex #3893619015 on #1937 — traces the underlying mechanism this
/// finding names, at the layer `evaluate_postcondition`/`run_turn`
/// operates on. `postcondition_field_into_reserved_json_key_is_rejected`
/// in `company::workflow_file::tests` is the actual fix: `validate()`
/// refuses `field: "json.text"`/`"json.agent_ref"` at author time, so no
/// graph that ever reaches `run_turn` in production can carry one. This
/// test constructs the request `run_turn` would see if that guarantee
/// were ever bypassed, to pin — and make visible — exactly why the
/// validation-time rejection is the right layer for the fix rather than
/// something patchable here: `text`/`agent_ref` are inserted into `value`
/// FIRST and merged with `or_insert` (base wins), on purpose, so
/// `delivery.rs::report_text` keeps finding the raw reply string for the
/// overwhelming majority of nodes whose reply is plain prose — the same
/// base-wins rule that protects that majority is exactly what makes a
/// `field` colliding with one of those two reserved keys validate a
/// value the emitted output can never actually hold.
#[tokio::test]
async fn a_colliding_field_would_diverge_between_gate_and_emitted_value() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1937-colliding-field-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let record = crate::workflows::gated_tool_turn_test::record();
    let turn = Arc::new(ScriptedTurn(crate::harness::TurnOutcome {
        reply: "{\"text\": [\"a\", \"b\"], \"agent_ref\": 123}".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    }));
    let board_claim = Arc::new(deps.delegations.claim_board("run-1937g"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1937g"));
    let runner = HarnessAgentRunner::new(
        turn,
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1937g".to_string(),
        "run-1937g".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    // `field = "json.text"`: the parsed reply's OWN `text` key is an
    // array. `field_present` only asks "is this present and non-null" —
    // it passes, having validated an ARRAY.
    let (value, _outcome) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "reply with structured data",
                "postcondition": { "require": "field_present", "field": "json.text" }
            }),
        )
        .await
        .expect("field_present on json.text finds the parsed reply's own text key, an array");

    // But the emitted `value["text"]` — what a downstream `=item.json.text`
    // binding actually reads — is the RAW REPLY STRING (`or_insert`, base
    // wins), a completely different type from the array the gate just
    // validated. Gate green; downstream gets a string where the author
    // was told to expect (and validated) a non-empty array.
    assert!(
        value["text"].is_string(),
        "value[\"text\"] must still be the raw reply string (the report_text              guarantee), not the array the gate validated: {value}"
    );
    assert_ne!(
        value["text"],
        json!(["a", "b"]),
        "the gate validated json.text as an array, but the emitted value's              text key is a different value entirely: {value}"
    );

    // Same divergence on `agent_ref`: the parsed reply's own `agent_ref`
    // is the number 123; the gate's `field_present` on `json.agent_ref`
    // passes on that number, but the emitted `value["agent_ref"]` is the
    // real roster id string, not 123.
    let (value2, _outcome2) = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "lister",
                "prompt": "reply with structured data",
                "postcondition": { "require": "field_present", "field": "json.agent_ref" }
            }),
        )
        .await
        .expect("field_present on json.agent_ref finds the parsed reply's own agent_ref key");
    assert_eq!(
        value2["agent_ref"], "researcher",
        "the emitted agent_ref must stay the real roster id (not the model-supplied              123 the gate validated): {value2}"
    );
}

/// Issue #638: a node that gates more calls than the cap allows leaves the
/// operator a **notice**, not only a log line.
///
/// Asserted on `RunNotices` — the value that becomes `WorkflowRun::notices`
/// and then the journaled outcome the history panel reads — rather than on
/// a log, which is what the issue asks for and what the chat path already
/// had via #561.
#[tokio::test]
async fn an_overflowing_node_leaves_the_operator_a_notice() {
    let over = MAX_APPROVAL_REQUESTS_PER_TURN + 3;
    let (notices, queue) = overflowing_runner_notices(over, true).await;

    assert_eq!(notices.len(), 1, "one notice for one overflow: {notices:?}");
    let notice = &notices[0];
    assert!(
        notice.contains(&format!("at most {MAX_APPROVAL_REQUESTS_PER_TURN}")),
        "it must quote the cap that did the discarding: {notice}"
    );
    assert!(notice.contains('3'), "…and how many went past it: {notice}");
    assert_eq!(
        queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN).requests.len(),
        0,
        "the drain already emptied this run's scope"
    );
}

/// The ordering fix that rides with it. The `parking`-is-`None` guard
/// `return`s, and it used to sit **above** the overflow branch — so on a
/// runtime with no approvals gate the discard was not even reaching the
/// log, let alone the operator.
///
/// That is the worst case, not a corner: the survivors could not be parked
/// either, so the notice is the *only* thing the operator can be told.
#[tokio::test]
async fn the_notice_survives_a_runtime_with_no_approvals_gate() {
    let over = MAX_APPROVAL_REQUESTS_PER_TURN + 2;
    let (notices, _) = overflowing_runner_notices(over, false).await;
    assert_eq!(
        notices.len(),
        1,
        "no gate to park into is exactly when the operator most needs telling: {notices:?}"
    );
}

/// A node that stayed under the cap says nothing — the notice must be the
/// exception, not a line on every run.
#[tokio::test]
async fn a_node_within_the_cap_raises_no_notice() {
    let (notices, _) = overflowing_runner_notices(MAX_APPROVAL_REQUESTS_PER_TURN, true).await;
    assert!(notices.is_empty(), "nothing was discarded: {notices:?}");
}

/// Issue #1825 (P1, found by chatgpt-codex-connector): `park_gated_calls`
/// must arm a blocked node's in-memory continuation stash itself, before
/// it parks a single call — not leave that to the runner's block-settle
/// pass (`stash_blocked_agent_nodes` in `super::super::runner`), which
/// only runs after the agent has returned, the engine has settled, and —
/// on the halt path — the run's output has already been persisted.
///
/// # The race this closes
///
/// `park_and_journal` (inside the loop this test drives) is what makes a
/// blocked node's approval card durable and clickable. Before this fix,
/// nothing armed `BlockedNodeQueue` until well after that — an operator
/// who approved the card in that window found `continue_turn` consuming
/// their decision against an empty stash: the turn retired with nothing
/// to release, and the later block-settle pass then stashed facts for a
/// decision that had already been spent, permanently stranding the run
/// (exactly the loss `stashed_turns()`'s reconciliation retires as
/// "unapproved"). `HarnessAgentRunner` carrying no trigger input was why
/// the arm could not happen here before — see `RunContext::trigger_input`
/// and this struct's own `trigger_input` field.
///
/// # Why this drives `park_gated_calls` directly
///
/// No `stash_blocked_agent_nodes` block-settle pass runs anywhere in this
/// test — the queue is inspected immediately after the parking call
/// returns, the same way the resolve path's `peek` would find it if an
/// approval landed at that instant. Pre-fix this assertion fails: nothing
/// in `park_gated_calls` armed the queue, so the peek is `None`. Post-fix
/// it holds this run's own trigger input, proving the card cannot outrun
/// the stash that redeems it.
/// What the node's diagnosis promises has to match what deciding the card
/// actually does (CodeRabbit review on #1905).
///
/// A gated tool call and an agent's blocker ride the same `approval_ids`
/// and settle the node identically, but only the first resumes on approval:
/// its park carries the node's turn key, while a blocker is parked
/// `Unlinked` with `agent: None` and no continuation — deliberately, since
/// answering a question is not authorising a call. The diagnosis said
/// "Approving the card continues this run automatically" for both, which
/// for a blocker is an operator approving a card and then watching a run
/// that never moves.
///
/// Issue #2005 moved the truthful line rather than removing the rule: a
/// blocker's answer now DOES re-enter the step, but not by approving — the
/// four verdicts differ, and one of them stops the run. The card must
/// describe that, not borrow the gated call's sentence.
#[test]
fn the_diagnosis_only_promises_a_resume_it_can_keep() {
    let gated = ParkedCalls {
        tools: vec!["publish_artifact".to_string()],
        approval_ids: vec!["appr-1".to_string()],
        unparkable: 0,
        blockers: 0,
    };
    let text = blocked_diagnosis(Some("work"), "writer", &gated);
    assert!(
        text.contains("continues this run automatically"),
        "a gated call really does resume on approval: {text}"
    );

    let blocker = ParkedCalls {
        tools: vec!["escalate_to_human".to_string()],
        approval_ids: vec!["appr-1".to_string()],
        unparkable: 0,
        blockers: 1,
    };
    let text = blocked_diagnosis(Some("work"), "writer", &blocker);
    assert!(
        !text.contains("continues this run automatically"),
        "a blocker is not decided by approving it: {text}"
    );
    assert!(
        text.contains("re-enters this step"),
        "an answered blocker does re-enter the step it stopped: {text}"
    );
    assert!(
        text.contains(crate::ports::blockers::BLOCKER_VERDICT_CHOICES),
        "all four verdicts are reachable, so all four have to be named: {text}"
    );

    let mixed = ParkedCalls {
        tools: vec![
            "publish_artifact".to_string(),
            "escalate_to_human".to_string(),
        ],
        approval_ids: vec!["appr-1".to_string(), "appr-2".to_string()],
        unparkable: 0,
        blockers: 1,
    };
    let text = blocked_diagnosis(Some("work"), "writer", &mixed);
    assert!(
        text.contains("continue this run when approved")
            && text.contains("re-enter the step they stopped"),
        "a mixed node has to describe both, since neither sentence is true of all of it: \
         {text}"
    );
    assert!(
        text.contains(crate::ports::blockers::BLOCKER_VERDICT_CHOICES),
        "a mixed node's blocker cards offer the same four verdicts, worded from the same \
         fragment as the blocker-only branch above: {text}"
    );

    // Nothing was parked at all — every call failed to park — so there is
    // no card to promise anything about.
    let none_parked = ParkedCalls {
        tools: vec!["publish_artifact".to_string()],
        approval_ids: Vec::new(),
        unparkable: 1,
        blockers: 0,
    };
    let text = blocked_diagnosis(Some("work"), "writer", &none_parked);
    assert!(!text.contains("Approving the card"), "{text}");
    assert!(!text.contains("re-enters this step"), "{text}");
}

#[tokio::test]
async fn park_gated_calls_arms_the_stash_before_any_block_settle_pass_runs() {
    use crate::harness::policy::{ApprovalRequest, ApprovalScope};
    use crate::ports::types::{Effect, EffectGroup};

    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p1-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let parking = deps
        .delivery
        .clone()
        .expect("gated_tool_turn_test::deps wires delivery")
        .parking
        .clone()
        .expect("gated_tool_turn_test::deps wires parking");
    let queue = deps.approval_requests.clone();
    let trigger_input = json!({ "request": "quarterly numbers" });
    let board_claim = Arc::new(deps.delegations.claim_board("run-1825-p1"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1825-p1"));
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1825-p1".to_string(),
        "run-1825-p1".to_string(),
        None,
        trigger_input.clone(),
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key(&runner.run_id, "work");

    // Pushed inside the run's own scope, exactly as its turn would.
    let claim = queue.claim(ApprovalScope::Run("run-1825-p1".to_string()));
    claim
        .scoped(async {
            queue.push(ApprovalRequest {
                tool: "shell".to_string(),
                reason: "gated".to_string(),
                effect: Effect {
                    kind: "shell".to_string(),
                    group: EffectGroup::Other,
                    amount_usd: None,
                    established_thread: false,
                    first_time_counterparty: false,
                    payload: json!({ "cmd": "rm -rf /" }),
                    agent: Some("ceo".to_string()),
                    run_id: None,
                },
            });
        })
        .await;

    // The real call a turn's tool loop makes. No block-settle pass runs
    // anywhere in this test.
    claim
        .scoped(runner.park_gated_calls(Some("work"), "work", &node_turn))
        .await;

    let stashed = parking.blocked_nodes.peek(&node_turn).expect(
        "the stash must be armed by park_gated_calls itself, before any block-settle \
         pass runs — an operator approving this node's just-parked card must always find \
         something to release",
    );
    assert_eq!(stashed.workflow_id, "wf-1825-p1");
    assert_eq!(stashed.input, trigger_input);
}

/// Issue #1825 (P1, second follow-up — found by chatgpt-codex-connector):
/// `park_gated_calls` must durably stash a blocked node's continuation
/// facts itself, before it parks a single call, not leave the durable
/// mirror to `stash_blocked_agent_nodes`'s block-settle pass alone.
///
/// # The race this closes
///
/// The test above proves the *in-memory* arm can no longer be outrun by
/// an operator acting on a just-published card. But `park_and_journal`
/// (inside the loop this test also drives) is what makes that card
/// **host-durable** and clickable across a restart — and until this fix,
/// nothing durable backed the in-memory arm until
/// `stash_blocked_agent_nodes` ran, which is strictly later: only once
/// the agent has returned and the engine has settled. A process that
/// died in that window left a restart with a recoverable card
/// (`ApprovalParked` is `Durability::Host` for a workflow-scoped effect)
/// and no matching `BlockedNodeStashed` record for `BlockedNodeQueue`'s
/// own `rearm` to rebuild a stash from — approving the recovered card
/// then consumed it against nothing, the identical shape the in-memory
/// race above closes, one durability tier up.
///
/// # Why this drives `park_gated_calls` directly, and reads the journal
///
/// Exactly like the test above: no `stash_blocked_agent_nodes` block-
/// settle pass runs anywhere here, so a durable stash observed right
/// after `park_gated_calls` returns can only have come from the park-time
/// write this fix adds. Pre-fix this assertion fails: `blocked_stashes()`
/// is empty, because nothing durable is written until settle. Post-fix it
/// holds this run's own trigger input, proving the durable record cannot
/// outrun the card that redeems it either.
#[tokio::test]
async fn park_gated_calls_durably_stashes_before_any_block_settle_pass_runs() {
    use crate::harness::policy::{ApprovalRequest, ApprovalScope};
    use crate::ports::types::{Effect, EffectGroup};

    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p1b-")
        .tempdir()
        .expect("tempdir");
    let (deps, journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let queue = deps.approval_requests.clone();
    let trigger_input = json!({ "request": "quarterly numbers" });
    let board_claim = Arc::new(deps.delegations.claim_board("run-1825-p1b"));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run("run-1825-p1b"),
    );
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1825-p1b".to_string(),
        "run-1825-p1b".to_string(),
        None,
        trigger_input.clone(),
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key(&runner.run_id, "work");

    let claim = queue.claim(ApprovalScope::Run("run-1825-p1b".to_string()));
    claim
        .scoped(async {
            queue.push(ApprovalRequest {
                tool: "shell".to_string(),
                reason: "gated".to_string(),
                effect: Effect {
                    kind: "shell".to_string(),
                    group: EffectGroup::Other,
                    amount_usd: None,
                    established_thread: false,
                    first_time_counterparty: false,
                    payload: json!({ "cmd": "rm -rf /" }),
                    agent: Some("ceo".to_string()),
                    run_id: None,
                },
            });
        })
        .await;

    // The real call a turn's tool loop makes. No block-settle pass runs
    // anywhere in this test.
    claim
        .scoped(runner.park_gated_calls(Some("work"), "work", &node_turn))
        .await;

    let stashed = journal
        .blocked_stashes()
        .into_iter()
        .find(|(turn, ..)| turn == &node_turn)
        .expect(
            "the durable stash must be written by park_gated_calls itself, before any \
             block-settle pass runs — a restart landing after this node's card goes \
             durable must always find a matching stash to rebuild from",
        );
    assert_eq!(stashed.1, "wf-1825-p1b");
    assert_eq!(stashed.2, trigger_input);
}

/// A node whose turn parks neither a gated call nor a blocker must never
/// touch the blocked-node stash at all — `park_gated_calls` runs on every
/// ordinary node, and most never block. The release-on-total-failure
/// cleanup this queues must only fire for a turn this very call armed.
#[tokio::test]
async fn park_gated_calls_leaves_an_unstashed_turn_untouched() {
    let dir = tempfile::Builder::new()
        .prefix("oc-2005-release-guard-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let trigger_input = json!({ "topic": "quarterly numbers" });
    let board_claim = Arc::new(deps.delegations.claim_board("run-2005-guard"));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run("run-2005-guard"),
    );
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "reporting".to_string(),
        "run-2005-guard".to_string(),
        None,
        trigger_input,
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key(&runner.run_id, "work");

    // Nothing was ever queued for this node's turn — no blocker, no gated
    // call — so this call never armed a stash for it.
    let summary = runner
        .park_gated_calls(Some("work"), "work", &node_turn)
        .await;
    assert_eq!(summary.approval_ids.len(), 0);

    // A turn that never touches the journal never creates the file.
    let raw = tokio::fs::read_to_string(dir.path().join("journal.jsonl"))
        .await
        .unwrap_or_default();
    assert!(
        !raw.contains("BlockedNodeReleased"),
        "a turn this call never stashed must not durably record a release for it: {raw}"
    );
}

/// Issue #1825 (P2, third follow-up — found by chatgpt-codex-connector): a
/// node whose every gated call fails to park must not leave a stash behind
/// with nothing that can ever redeem it.
///
/// The arm and the durable stash run unconditionally, before the request
/// loop attempts a single park — required, since that ordering is what
/// closes the P1 race. But when every request in the batch then fails
/// (journal outage), `summary.approval_ids` comes back empty: no approval
/// id was ever minted for this turn, so nothing will ever call
/// `continue_turn` for it, and the stash this call armed and durably wrote
/// would otherwise sit forever — one workflow id and trigger payload
/// retained in memory for the process's life, and durably on every replay.
///
/// Forces every park to fail by pointing `parking.journal` at a path whose
/// parent directory does not exist, so `record_parked` inside
/// `park_and_journal` fails for each request — the gate's own `park` stays
/// in-memory and always succeeds, so this isolates the journal failure
/// without needing a custom `ApprovalGate` double.
#[tokio::test]
async fn a_node_with_no_successfully_parked_call_leaves_no_stash_behind() {
    use crate::harness::policy::{ApprovalRequest, ApprovalScope};
    use crate::ports::types::{Effect, EffectGroup};

    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p2c-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    // `FsJournalStore::append_journal` calls `create_dir_all` on the
    // parent, so a merely-missing directory would not fail the write — it
    // would just get created. A regular file standing where the journal's
    // parent directory needs to be does: `create_dir_all` cannot turn a
    // file into a directory, so every append genuinely fails.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("write blocker file");
    let broken_journal = Arc::new(crate::runtime::journal::RuntimeJournal::new(
        blocker.join("journal.jsonl"),
    ));
    let delivery = deps
        .delivery
        .as_mut()
        .expect("gated_tool_turn_test::deps wires delivery");
    let parking = delivery
        .parking
        .as_mut()
        .expect("gated_tool_turn_test::deps wires parking");
    parking.journal = broken_journal.clone();
    let queue = deps.approval_requests.clone();
    let trigger_input = json!({ "request": "quarterly numbers" });
    let board_claim = Arc::new(deps.delegations.claim_board("run-1825-p2c"));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run("run-1825-p2c"),
    );
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1825-p2c".to_string(),
        "run-1825-p2c".to_string(),
        None,
        trigger_input.clone(),
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key(&runner.run_id, "work");

    let claim = queue.claim(ApprovalScope::Run("run-1825-p2c".to_string()));
    claim
        .scoped(async {
            queue.push(ApprovalRequest {
                tool: "shell".to_string(),
                reason: "gated".to_string(),
                effect: Effect {
                    kind: "shell".to_string(),
                    group: EffectGroup::Other,
                    amount_usd: None,
                    established_thread: false,
                    first_time_counterparty: false,
                    payload: json!({ "cmd": "rm -rf /" }),
                    agent: Some("ceo".to_string()),
                    run_id: None,
                },
            });
        })
        .await;

    let summary = claim
        .scoped(runner.park_gated_calls(Some("work"), "work", &node_turn))
        .await;

    assert!(
        summary.approval_ids.is_empty(),
        "precondition: the broken journal must fail every park attempt"
    );
    assert_eq!(summary.unparkable, 1);
    assert!(
        !runner
            .deps
            .delivery
            .as_ref()
            .expect("delivery wired")
            .parking
            .as_ref()
            .expect("parking wired")
            .blocked_nodes
            .is_armed(&node_turn),
        "a node with zero successfully parked calls must not leave an unredeemable \
         in-memory stash behind"
    );
    assert!(
        broken_journal
            .blocked_stashes()
            .into_iter()
            .all(|(turn, ..)| turn != node_turn),
        "a node with zero successfully parked calls must not leave a durable stash \
         behind either"
    );
}

/// Issue #1825 (P1, fourth follow-up — found by chatgpt-codex-connector):
/// approving the first card a multi-call node parks must not complete its
/// continuation batch before the rest of the node's calls have even been
/// attempted.
///
/// # The race this closes
///
/// `park_gated_calls` parks a node's gated calls one at a time in a loop,
/// and each successful `park_and_journal` arms `ContinuationQueue` for the
/// node's turn — issue #469/#978's original per-call mechanism, unchanged.
/// With no hold, `outstanding` right after the FIRST call parks is exactly
/// 1: a decision on that lone card zeroes it out and
/// `ContinuationQueue::decide` hands back a "complete" batch, even though
/// the loop has not attempted the node's second call yet.
/// `blocked_nodes.arm` (the P1 first follow-up, above) already makes the
/// workflow id and trigger input available the instant the first card
/// exists, so a premature zero here finds a real stash rather than an
/// empty one — pre-fix, that reaches `resume_blocked_agent_node` and
/// re-dispatches the run while this node is still parking its remaining
/// calls.
///
/// # How this is reproduced deterministically
///
/// A real timing race needs two concurrent tasks; this test gets the same
/// interleaving without one. `RaceGate` wraps the approval gate
/// `park_gated_calls` parks through, and its second `park` call — the
/// second gated call's — first decides the FIRST card via the SAME
/// `ContinuationQueue` handle `park_and_journal` arms, synchronously,
/// before that second park even returns. That is exactly where a fast
/// operator's decision would land relative to the loop below, reproduced
/// on ordering rather than wall-clock luck.
#[tokio::test]
async fn approving_the_first_card_of_a_multi_call_node_does_not_complete_the_batch_early() {
    use crate::harness::policy::{ApprovalRequest, ApprovalScope};
    use crate::ports::approvals::ApprovalGate;
    use crate::ports::types::{
        Actor, ActorKind, ApprovalId, CompanyEvent, Effect, EffectGroup, PolicyDecision,
        Verdict,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as AsyncMutex;

    /// Delegates every call to `inner`, except that the SECOND `park` it
    /// sees first decides the FIRST approval it minted, via the same
    /// `ContinuationQueue` the real park path arms — simulating an
    /// operator racing ahead of `park_gated_calls`'s own loop.
    struct RaceGate {
        inner: Arc<dyn ApprovalGate>,
        continuations: crate::runtime::continuation::ContinuationQueue,
        node_turn: String,
        calls: AtomicUsize,
        first_approval: AsyncMutex<Option<ApprovalId>>,
        /// What `ContinuationQueue::decide` returned for the interleaved
        /// decision on the first card — the assertion this test exists
        /// for. Outer `Option`: whether the interleave actually ran.
        early_decide_result: AsyncMutex<Option<Option<Vec<CompanyEvent>>>>,
    }

    #[async_trait::async_trait]
    impl ApprovalGate for RaceGate {
        async fn evaluate(
            &self,
            company: &CompanyId,
            effect: &Effect,
        ) -> crate::Result<PolicyDecision> {
            self.inner.evaluate(company, effect).await
        }

        async fn park(&self, company: &CompanyId, effect: Effect) -> crate::Result<ApprovalId> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let id = self.inner.park(company, effect).await?;
            if call == 0 {
                *self.first_approval.lock().await = Some(id.clone());
            } else if call == 1 {
                let first = self
                    .first_approval
                    .lock()
                    .await
                    .clone()
                    .expect("the first card must have parked before the second");
                let event = CompanyEvent::ApprovalResolved {
                    approval_id: first,
                    verdict: Verdict::Approve,
                    by: Actor {
                        kind: ActorKind::Operator,
                        id: "operator".to_string(),
                    },
                };
                let result = self.continuations.decide(&self.node_turn, Some(event));
                *self.early_decide_result.lock().await = Some(result);
            }
            Ok(id)
        }

        async fn resolve(
            &self,
            id: &ApprovalId,
            verdict: Verdict,
            by: Actor,
        ) -> crate::Result<Option<Effect>> {
            self.inner.resolve(id, verdict, by).await
        }
    }

    let dir = tempfile::Builder::new()
        .prefix("oc-1825-p1-4-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());

    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key("run-1825-p1-4", "work");

    let delivery = deps
        .delivery
        .as_mut()
        .expect("gated_tool_turn_test::deps wires delivery");
    let parking = delivery
        .parking
        .as_mut()
        .expect("gated_tool_turn_test::deps wires parking");
    let race_gate = Arc::new(RaceGate {
        inner: parking.approvals.clone(),
        continuations: parking.continuations.clone(),
        node_turn: node_turn.clone(),
        calls: AtomicUsize::new(0),
        first_approval: AsyncMutex::new(None),
        early_decide_result: AsyncMutex::new(None),
    });
    parking.approvals = race_gate.clone();

    let queue = deps.approval_requests.clone();
    let trigger_input = json!({ "request": "quarterly numbers" });
    let board_claim = Arc::new(deps.delegations.claim_board("run-1825-p1-4"));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run("run-1825-p1-4"),
    );
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1825-p1-4".to_string(),
        "run-1825-p1-4".to_string(),
        None,
        trigger_input.clone(),
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    let claim = queue.claim(ApprovalScope::Run("run-1825-p1-4".to_string()));
    claim
        .scoped(async {
            for tool in ["shell", "http"] {
                queue.push(ApprovalRequest {
                    tool: tool.to_string(),
                    reason: "gated".to_string(),
                    effect: Effect {
                        kind: tool.to_string(),
                        group: EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: json!({ "call": tool }),
                        agent: Some("ceo".to_string()),
                        run_id: None,
                    },
                });
            }
        })
        .await;

    let summary = claim
        .scoped(runner.park_gated_calls(Some("work"), "work", &node_turn))
        .await;

    assert_eq!(summary.approval_ids.len(), 2, "both calls must have parked");

    let early_result = race_gate.early_decide_result.lock().await.clone();
    assert_eq!(
        early_result,
        Some(None),
        "deciding the first card while the loop was still parking the second must NOT \
         complete the batch — ContinuationQueue::decide must report 'still waiting' \
         (None), not hand back a batch the run has not finished parking yet"
    );
}

/// Queues `count` gated calls in a run's scope, drains them through
/// `park_gated_calls`, and returns whatever the run was told.
///
/// `with_gate` selects whether a `parking` sink is wired, which is the axis
/// the guard-order test needs.
async fn overflowing_runner_notices(
    count: usize,
    with_gate: bool,
) -> (Vec<String>, crate::harness::policy::ApprovalRequestQueue) {
    use crate::harness::policy::{ApprovalRequest, ApprovalScope};
    use crate::ports::types::{Effect, EffectGroup};

    let dir = tempfile::Builder::new()
        .prefix("oc-638-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    if !with_gate {
        deps.delivery = None;
    }
    let queue = deps.approval_requests.clone();
    let notices = RunNotices::default();
    let board_claim = Arc::new(deps.delegations.claim_board("run-1"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1"));
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1".to_string(),
        "run-1".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        notices.clone(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );

    // Pushed inside the run's own scope, exactly as its turn would.
    let claim = queue.claim(ApprovalScope::Run("run-1".to_string()));
    claim
        .scoped(async {
            for i in 0..count {
                queue.push(ApprovalRequest {
                    tool: "shell".to_string(),
                    reason: "gated".to_string(),
                    effect: Effect {
                        kind: "shell".to_string(),
                        group: EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: json!({ "n": i }),
                        agent: Some("ceo".to_string()),
                        run_id: None,
                    },
                });
            }
        })
        .await;
    let node_turn =
        crate::runtime::workflow_resume::workflow_node_turn_key(&runner.run_id, "work");
    claim
        .scoped(runner.park_gated_calls(Some("work"), "work", &node_turn))
        .await;
    (notices.take(), queue)
}

/// PR #1775 review: a publish the tool refused mid-turn, but which the
/// post-turn workspace capture materialized anyway, must not be silently
/// dropped from the run's notices. The node's own turn reply already told
/// the operator delivery failed (the tool's response, at call time); going
/// silent here would leave that unreconciled against a run inspector that
/// shows the file delivered.
#[tokio::test]
async fn a_captured_publish_reconciles_its_earlier_refusal_notice() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1775-")
        .tempdir()
        .expect("tempdir");
    let (deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let pending_publishes = deps.pending_publishes.clone();
    let notices = RunNotices::default();
    let board_claim = Arc::new(deps.delegations.claim_board("run-1775"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1775"));
    let runner = HarnessAgentRunner::new(
        single_turn(&deps),
        deps,
        crate::workflows::gated_tool_turn_test::record(),
        CompanyId::new("acme"),
        "wf-1".to_string(),
        "run-1775".to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        notices.clone(),
        RunBoard::default(),
        RunBlocks::default(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim.clone(),
    );

    // The tool's refusal, staged inside this run's scope exactly as the
    // live `publish_artifact` call would have.
    publish_refusal_claim
        .scoped(async { pending_publishes.push_refusal("specs/plan.md".to_string()) })
        .await;

    // The post-turn drain, told that the workspace scan captured that
    // same file anyway.
    publish_refusal_claim
        .scoped(async {
            runner.drain_publish_refusals(&["specs/plan.md".to_string()]);
        })
        .await;

    let recorded = notices.take();
    assert_eq!(
        recorded.len(),
        1,
        "the refusal must be reconciled with a notice, not silenced: {recorded:?}"
    );
    assert!(
        recorded[0].contains("specs/plan.md"),
        "the notice must name the file: {recorded:?}"
    );
    assert!(
        recorded[0].contains("captured"),
        "the notice must say the file landed anyway, not just that it was refused: \
         {recorded:?}"
    );
}

#[test]
fn message_prefers_prompt_then_input_then_message() {
    assert_eq!(
        message_from_request(&json!({ "prompt": "P", "input": "I" })),
        "P"
    );
    assert_eq!(message_from_request(&json!({ "input": "I" })), "I");
    assert_eq!(message_from_request(&json!({ "message": "M" })), "M");
}

// ── Issue #154: the operator's run request reaches the agent ──

#[test]
fn run_request_is_appended_under_a_labelled_heading() {
    let out = compose_turn_message("Draft the launch post.", Some("dark mode for iOS"));
    // The node's standing instruction still leads.
    assert!(out.starts_with("Draft the launch post."), "{out}");
    // …and this run's subject is distinguishable from it.
    assert!(out.contains("Request for this run:"), "{out}");
    assert!(out.contains("dark mode for iOS"), "{out}");
}

#[test]
fn a_run_with_no_request_is_byte_identical_to_the_old_message() {
    // The guarantee that makes this safe to land: runs that supply no topic
    // must behave exactly as they did before.
    for empty in [None, Some(""), Some("   "), Some("\n\t ")] {
        assert_eq!(
            compose_turn_message("Draft the launch post.", empty),
            "Draft the launch post.",
            "empty request {empty:?} must not alter the message"
        );
    }
}

#[test]
fn a_request_with_no_instruction_stands_on_its_own() {
    // No dangling heading when the node carries no usable instruction.
    assert_eq!(
        compose_turn_message("", Some("ship dark mode")),
        "ship dark mode"
    );
    assert_eq!(
        compose_turn_message("   ", Some("ship dark mode")),
        "ship dark mode"
    );
}

#[test]
fn run_request_text_reads_the_console_payload_and_a_bare_string() {
    assert_eq!(
        run_request_text(&json!({ "request": "dark mode" })).as_deref(),
        Some("dark mode")
    );
    assert_eq!(
        run_request_text(&json!("dark mode")).as_deref(),
        Some("dark mode")
    );
    // Tolerated spellings from a hand-written call or an older client.
    for key in ["input", "topic", "message", "text"] {
        let mut payload = serde_json::Map::new();
        payload.insert(key.to_string(), json!("dark mode"));
        assert_eq!(
            run_request_text(&Value::Object(payload)).as_deref(),
            Some("dark mode"),
            "key {key} should be accepted"
        );
    }
    // Trimmed.
    assert_eq!(
        run_request_text(&json!({ "request": "  dark mode  " })).as_deref(),
        Some("dark mode")
    );
}

#[test]
fn run_request_text_is_none_for_payloads_that_carry_no_topic() {
    // These are the shapes an existing caller already sends — none may start
    // injecting a topic into agent messages.
    for payload in [
        json!({}),
        json!(null),
        json!(42),
        json!({ "request": "" }),
        json!({ "request": "   " }),
        json!({ "unrelated": "value" }),
        json!({ "request": 7 }),
        json!(["dark mode"]),
    ] {
        assert_eq!(
            run_request_text(&payload),
            None,
            "payload {payload} must carry no topic"
        );
    }
}

#[test]
fn message_falls_back_to_serialized_request() {
    // No known string key: fall back to the serialized object.
    let out = message_from_request(&json!({ "agent_ref": "x" }));
    assert!(out.contains("agent_ref"));
}

// ── Issue #782: the upstream node's output reaches the next agent's turn ──

/// [`append_upstream_input`] under the shipped budget, keeping the #782
/// tests reading about *what reaches the turn* rather than about the #849
/// budget they are all far below. The truncation report those calls discard
/// has its own tests below.
fn folded(request: &Value) -> String {
    append_upstream_input(
        &message_from_request(request),
        request,
        upstream::DEFAULT_UPSTREAM_BUDGET_CHARS,
    )
    .0
}

/// The headline. An `agent -> agent` pipeline's second teammate must receive
/// the first's output. `translate` binds `input = "=items"`, the engine
/// resolves it to the predecessor envelope, and this proves the runner folds
/// that envelope's prose into the turn — under a heading, AFTER the node's
/// own instruction, so both survive.
#[test]
fn upstream_output_is_folded_into_the_turn() {
    // The shape the engine hands us: `prompt` (the node's static job) plus
    // `input` (the resolved `=items`) — one predecessor agent envelope.
    let request = json!({
        "prompt": "Write the launch post.",
        "input": [{ "json": {}, "text": "The analyst found a 20% MoM jump.", "raw": {} }],
    });
    let message = folded(&request);
    // The node's own instruction still leads.
    assert!(message.starts_with("Write the launch post."), "{message}");
    // …the upstream output is present, under its heading…
    assert!(message.contains(UPSTREAM_INPUT_HEADING), "{message}");
    assert!(
        message.contains("The analyst found a 20% MoM jump."),
        "the previous step's output must reach the turn: {message}"
    );
}

/// Fan-in: a `merge -> agent` (or several edges into one agent) resolves
/// `=items` to EVERY predecessor, and all of them must be delivered — the
/// "loses all but the first" failure is exactly what `=items` (not `=item`)
/// guards against.
#[test]
fn fan_in_delivers_every_predecessor() {
    let request = json!({
        "prompt": "Combine the research.",
        "input": [
            { "json": {}, "text": "Predecessor A: market is up.", "raw": {} },
            { "json": {}, "text": "Predecessor B: sentiment is positive.", "raw": {} },
        ],
    });
    let message = folded(&request);
    assert!(
        message.contains("Predecessor A: market is up."),
        "first predecessor missing: {message}"
    );
    assert!(
        message.contains("Predecessor B: sentiment is positive."),
        "second predecessor missing — a fan-in must not lose all but the first: {message}"
    );
}

/// A non-agent predecessor (a `tool_call` / `transform` / `output` node) has
/// no prose `text`, so its structured output is rendered as JSON rather than
/// dropped.
#[test]
fn a_structured_predecessor_is_rendered_as_json() {
    let request = json!({
        "prompt": "Summarise the fetch.",
        "input": [{ "json": { "rows": 3 }, "text": null, "raw": { "rows": 3 } }],
    });
    let message = folded(&request);
    assert!(message.contains(UPSTREAM_INPUT_HEADING), "{message}");
    assert!(
        message.contains("\"rows\""),
        "structured output rendered: {message}"
    );
}

/// The byte-identical guarantee. A single-agent workflow with no predecessor
/// (no `input`, or an empty / all-null / empty-container `input`) composes
/// exactly the pre-#782 message — never a dangling empty heading.
#[test]
fn no_upstream_output_is_byte_identical() {
    let base = "Draft the launch post.";
    for input in [
        None,
        Some(json!(null)),
        Some(json!([])),
        Some(json!([null])),
        Some(json!([{}])),
        Some(json!([{ "json": {}, "text": "   ", "raw": {} }])),
    ] {
        let mut request = serde_json::Map::new();
        request.insert("prompt".to_string(), json!(base));
        if let Some(input) = input.clone() {
            request.insert("input".to_string(), input);
        }
        let request = Value::Object(request);
        assert_eq!(
            folded(&request),
            base,
            "input {input:?} must not alter the message or add an empty heading"
        );
        // And the whole composition (including the #154 run topic) is
        // unchanged from what `compose_turn_message` alone would produce.
        let instruction = folded(&request);
        assert_eq!(
            compose_turn_message(&instruction, Some("ship dark mode")),
            compose_turn_message(base, Some("ship dark mode")),
            "the no-upstream path must leave the run-topic composition untouched"
        );
    }
}

/// Upstream output and the #154 run topic coexist: the node's instruction
/// leads, the previous step's output follows under its heading, and the run's
/// subject follows under its own — all three reach the teammate.
#[test]
fn upstream_output_and_run_topic_coexist() {
    let request = json!({
        "prompt": "Write the post.",
        "input": [{ "json": {}, "text": "ANALYST_SAID_THIS", "raw": {} }],
    });
    let instruction = folded(&request);
    let message = compose_turn_message(&instruction, Some("dark mode launch"));
    assert!(message.starts_with("Write the post."), "{message}");
    assert!(message.contains(UPSTREAM_INPUT_HEADING), "{message}");
    assert!(message.contains("ANALYST_SAID_THIS"), "{message}");
    assert!(message.contains("Request for this run:"), "{message}");
    assert!(message.contains("dark mode launch"), "{message}");
}

// ── Issue #849: nothing may hand an agent node an unbounded payload ──
//
// Driven by synthetic oversized payloads, never by a live page: the reported
// failure is intermittent *because* it depends on how much text a sports
// section happened to return that minute, so a test that reproduced it that
// way would be a coin flip too.

/// One predecessor envelope carrying `chars` characters of page-like text —
/// the shape a `web_fetch` `tool_call` node emits (its non-JSON output is
/// wrapped as `{"text": …}`, which the tinyflows envelope lifts to `text`).
fn source_envelope(marker: &str, chars: usize) -> Value {
    let body = format!("{marker}{}", "x".repeat(chars.saturating_sub(marker.len())));
    json!({ "json": { "text": body.clone() }, "text": body, "raw": { "text": body } })
}

/// How much slack above the budget the markers, the heading and the section
/// rules are allowed to add. They sit **outside** the budget deliberately —
/// the budget exists to bound upstream *text*, and letting our own accounting
/// compete for room would mean the truncation marker could itself be the
/// thing squeezed out (the reasoning `memory_loop`'s skipped-hit marker
/// arrived at first).
const MARKER_SLACK: usize = 2_000;

/// The reported shape: three fetched sources fan in to one ranking agent.
/// Every source must still be represented, the turn must be bounded, and
/// every cut must be visible.
#[test]
fn a_three_way_fan_in_is_bounded_and_no_source_is_lost() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let request = json!({
        "prompt": "Rank today's stories.",
        "input": [
            source_envelope("SOURCE_ONE", 200_000),
            source_envelope("SOURCE_TWO", 200_000),
            source_envelope("SOURCE_THREE", 200_000),
        ],
    });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    assert!(
        message.chars().count() <= budget + MARKER_SLACK,
        "600k characters of upstream input must not reach a turn: {} characters",
        message.chars().count()
    );
    // …and every source is still *there*, which is what separates a bound
    // from "drop everything after the first".
    for marker in ["SOURCE_ONE", "SOURCE_TWO", "SOURCE_THREE"] {
        assert!(message.contains(marker), "{marker} was lost entirely");
    }
    // Each cut is visible to the agent.
    assert_eq!(
        message.matches("TRUNCATED BY OPENCOMPANY").count(),
        3,
        "every truncated source carries its own marker: {message}"
    );
    assert!(message.contains("source 3 of 3"), "{message}");

    // …and to the operator.
    assert_eq!(report.sources.len(), 3);
    assert!(report.truncated_any());
    let notice = report.notice().expect("the operator is told");
    assert!(notice.contains("3 sources"), "{notice}");
    assert!(notice.contains("3 of them were truncated"), "{notice}");
}

/// The "is it only a fan-in?" question, answered: it is not. A **single**
/// enormous `web_fetch` into one agent runs the same unbounded path, and the
/// bound at the join covers it with no second rule.
#[test]
fn a_single_enormous_source_is_bounded_too() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let request = json!({
        "prompt": "Summarise this page.",
        "input": [source_envelope("ONLY_SOURCE", 500_000)],
    });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    assert!(
        message.chars().count() <= budget + MARKER_SLACK,
        "a single 500k-character page must not reach a turn whole: {} characters",
        message.chars().count()
    );
    assert!(message.contains("ONLY_SOURCE"), "the source still arrives");
    assert!(message.contains("source 1 of 1"), "{message}");
    assert_eq!(report.sources.len(), 1);
    assert!(report.truncated_any());
}

/// A large sibling must not starve a small one — the fan-in failure mode a
/// flat per-source cap would not fix and a running total would make
/// order-dependent.
#[test]
fn a_short_source_survives_whole_beside_an_enormous_one() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let short = "SHORT_SOURCE: the wire service filed three lines today.";
    let request = json!({
        "prompt": "Rank today's stories.",
        "input": [
            source_envelope("HUGE_SOURCE", 400_000),
            json!({ "json": {}, "text": short, "raw": {} }),
        ],
    });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    assert!(
        message.contains(short),
        "the short source must arrive intact, not be crowded out: {message}"
    );
    assert_eq!(
        message.matches("TRUNCATED BY OPENCOMPANY").count(),
        1,
        "only the enormous source is cut: {message}"
    );
    assert_eq!(report.sources[1].produced, report.sources[1].kept);
    assert!(report.sources[0].kept < report.sources[0].produced);
}

/// The overwhelmingly common run: everything fits, so the fold is exactly
/// what #782 produced and the operator is told nothing new.
#[test]
fn an_ordinary_fan_in_is_untouched_and_says_nothing() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let request = json!({
        "prompt": "Combine the research.",
        "input": [
            { "json": {}, "text": "Predecessor A: market is up.", "raw": {} },
            { "json": {}, "text": "Predecessor B: sentiment is positive.", "raw": {} },
        ],
    });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    assert!(!message.contains("TRUNCATED"), "{message}");
    assert!(!report.truncated_any());
    assert_eq!(report.notice(), None);
    assert!(
        message.contains("Predecessor A: market is up."),
        "{message}"
    );
    assert!(
        message.contains("Predecessor B: sentiment is positive."),
        "{message}"
    );
}

/// The bound survives composition: the marker is still in the message the
/// teammate is actually sent, alongside the node's instruction and the #154
/// run topic.
#[test]
fn the_truncation_marker_survives_into_the_composed_turn() {
    let request = json!({
        "prompt": "Rank today's stories.",
        "input": [source_envelope("BIG_SOURCE", 200_000)],
    });
    let (instruction, _) = append_upstream_input(
        &message_from_request(&request),
        &request,
        upstream::DEFAULT_UPSTREAM_BUDGET_CHARS,
    );
    let message = compose_turn_message(&instruction, Some("today's sport"));
    assert!(message.starts_with("Rank today's stories."), "{message}");
    assert!(message.contains("TRUNCATED BY OPENCOMPANY"), "{message}");
    assert!(message.contains("Request for this run:"), "{message}");
    assert!(message.contains("today's sport"), "{message}");
}

/// A thousand-way fan-in — a `split_out` over a large array is all it takes —
/// must not smuggle a thousand truncation markers past the budget. This is
/// the fold-level twin of `upstream`'s
/// `a_thousand_oversized_sources_stay_inside_the_budget`, driven through the
/// real envelope shape rather than pre-rendered strings, because that is the
/// path a graph actually takes.
#[test]
fn a_thousand_way_fan_in_cannot_smuggle_its_markers_past_the_budget() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let inputs: Vec<Value> = (0..1_000)
        .map(|n| source_envelope(&format!("SOURCE_{n}"), 5_000))
        .collect();
    let request = json!({ "prompt": "Rank today's stories.", "input": inputs });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    // The section itself is bounded by `budget`; the message adds only the
    // node's own instruction and the heading, which are not upstream text.
    assert!(
        message.chars().count() <= budget + MARKER_SLACK,
        "5,000,000 characters of upstream input across 1,000 sources produced a {}-character \
         turn",
        message.chars().count()
    );
    assert_eq!(report.sources.len(), 1_000, "every input is accounted for");
    let notice = report.notice().expect("the operator is told");
    assert!(notice.contains("1000 sources"), "{notice}");
}

/// A source rendered as JSON (a `transform` / structured `tool_call` output,
/// which has no prose `text`) is bounded on the same path — the bound is on
/// what the turn carries, not on which node kind produced it.
#[test]
fn a_structured_source_is_bounded_on_the_same_path() {
    let budget = upstream::DEFAULT_UPSTREAM_BUDGET_CHARS;
    let rows: Vec<Value> = (0..20_000)
        .map(|n| json!({ "headline": format!("story {n}"), "score": n }))
        .collect();
    let request = json!({
        "prompt": "Rank these.",
        "input": [{ "json": { "rows": rows }, "text": null, "raw": {} }],
    });
    let (message, report) =
        append_upstream_input(&message_from_request(&request), &request, budget);

    assert!(
        message.chars().count() <= budget + MARKER_SLACK,
        "a structured payload is bounded too: {} characters",
        message.chars().count()
    );
    assert!(message.contains("TRUNCATED BY OPENCOMPANY"), "{message}");
    assert!(report.truncated_any());
}

#[test]
fn workflow_workspace_is_unique_per_run_and_traversal_safe() {
    let root = std::path::Path::new("/tmp/workspaces");
    let company = CompanyId::new("acme");
    let first = workflow_workspace(root, &company, "../billing", "run:1");
    let second = workflow_workspace(root, &company, "../billing", "run:2");

    assert_ne!(first, second);
    assert!(first.starts_with(root.join("acme").join("_workflow")));
    assert!(!first.to_string_lossy().contains("../billing"));
    assert_eq!(
        first.file_name().and_then(|part| part.to_str()),
        Some("workspace")
    );
}

/// Issue #499. tinyflows 0.6 added `Capabilities::memory`, and this pins the
/// answer we gave it.
///
/// `None` is a decision, not an omission — see the comment at the field. A
/// `MemoryProvider` here would let a workflow read and *write* agent memory
/// (`remember`/`forget` are on the trait), and which scopes a workflow may
/// touch is a policy question this repo has not answered. Until it is,
/// unwired is the honest state: a `memory` node fails with a capability
/// error rather than quietly writing somewhere nobody authorised.
///
/// So this test is here to make wiring it a *deliberate* act. Whoever
/// changes it has to change this line too, which is where they will find the
/// question they need to answer first.
#[tokio::test]
async fn the_memory_capability_is_left_unwired_on_purpose() {
    let dir = tempfile::tempdir().expect("tempdir");
    // No endpoint is spawned: `build_capabilities` assembles a struct of
    // handles and never calls the provider, so a base URL that answers
    // nothing is sufficient and keeps this off the network.
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
        "http://127.0.0.1:1/unused".to_string(),
        dir.path(),
    );
    let record = crate::workflows::gated_tool_turn_test::record();

    let caps = build_capabilities(
        single_turn(&deps),
        deps,
        &record,
        RunContext {
            workflow_id: "wf",
            run_id: "run:1",
            checkpoint_thread_id: "run:1",
            workflow_fingerprint: "fp:1",
            run_request: None,
            trigger_input: &Value::Null,
            started_by: crate::ports::types::StartedBy::Operator,
            dry_run: false,
            notices: RunNotices::default(),
            board: RunBoard::default(),
            blocks: Default::default(),
            capped: Default::default(),
            halted: Default::default(),
            approvals: Default::default(),
            artifacts: Default::default(),
            runs: None,
            deep: None,
            attempts: Default::default(),
            child_gates: Default::default(),
        },
    )
    .await
    .expect("build_capabilities");

    assert!(
        caps.memory.is_none(),
        "wiring `Capabilities::memory` gives workflows read AND write access \
         to agent memory — settle which scopes a workflow may touch before \
         changing this, and say so at the field"
    );
    // The neighbouring optional capability IS wired, so this is a statement
    // about `memory` specifically rather than about the bundle being empty.
    assert!(
        caps.agent.is_some(),
        "agent capability should still be wired"
    );
}

/// Issue #542 — T9: a dry bundle wires the effect STUBS (agent / tools / http
/// all echo with the `dry_run` marker) and the inert `NoopState`, while the
/// read-only resolver stays real. Pinned behaviourally through the marker, so
/// a future refactor that quietly wired a real effect into a dry bundle fails
/// here.
#[tokio::test]
async fn a_dry_bundle_wires_stubs_and_noop_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
        "http://127.0.0.1:1/unused".to_string(),
        dir.path(),
    );
    let record = crate::workflows::gated_tool_turn_test::record();

    let caps = build_capabilities(
        single_turn(&deps),
        deps,
        &record,
        RunContext {
            workflow_id: "wf",
            run_id: "run:1",
            checkpoint_thread_id: "run:1",
            workflow_fingerprint: "fp:1",
            run_request: None,
            trigger_input: &Value::Null,
            started_by: crate::ports::types::StartedBy::Operator,
            dry_run: true,
            notices: RunNotices::default(),
            board: RunBoard::default(),
            blocks: Default::default(),
            capped: Default::default(),
            halted: Default::default(),
            approvals: Default::default(),
            artifacts: Default::default(),
            runs: None,
            deep: None,
            attempts: Default::default(),
            child_gates: Default::default(),
        },
    )
    .await
    .expect("build_capabilities");

    // http: the stub reports without sending, carrying the marker.
    //
    // A *public* URL, deliberately. This case used to use `127.0.0.1`, which
    // the real guard refuses — so it asserted that the dry slot answers `ok`
    // for a target no real run can reach, pinning issue #1048's false green
    // in place. The slot being the stub is what this test is about; whether a
    // given target is refused is `dry_run`'s own suite.
    let http_out = caps
        .http
        .request(json!({ "url": "https://example.com/hook" }), None)
        .await
        .expect("an allowed target is not refused by the dry stub");
    assert_eq!(
        http_out["dry_run"],
        json!(true),
        "http slot should be the dry stub"
    );

    // agent: the stub echoes with no pool routing.
    let agent = caps.agent.as_ref().expect("agent stub is wired");
    let agent_out = agent
        .run_agent("ceo", json!({ "prompt": "hi" }), None)
        .await
        .expect("dry agent never fails");
    assert_eq!(
        agent_out["dry_run"],
        json!(true),
        "agent slot should be the dry stub"
    );

    // state: NoopState — a load reads None and a store is dropped.
    assert_eq!(caps.state.load("k").await.expect("noop load"), None);
    caps.state.store("k", json!(1)).await.expect("noop store");
    assert_eq!(
        caps.state.load("k").await.expect("noop load"),
        None,
        "dry state must be the inert NoopState, never durable"
    );
}

// ── Issue #661 (M4): the unwired `llm` stub reports the RIGHT failure ──

/// T1 — the engine's output_parser auto-fix request (it calls `llm` to repair
/// a schema mismatch) surfaces the SCHEMA errors, not the generic bare-LLM
/// lead that used to mask them.
#[tokio::test]
async fn unwired_llm_surfaces_schema_errors_on_auto_fix_request() {
    let request = json!({
        "task": "coerce_to_schema",
        "schema": { "type": "object", "required": ["name", "age"] },
        "value": { "other": 1 },
        "errors": [
            "$: missing required property `name`",
            "$: missing required property `age`",
        ],
    });
    let EngineError::Capability(msg) = UnwiredLlm
        .complete(request, None)
        .await
        .expect_err("an unwired llm must error")
    else {
        panic!("expected a capability error");
    };
    // The real cause is present…
    assert!(
        msg.contains("failed schema validation"),
        "should carry the schema-validation lead: {msg}"
    );
    assert!(
        msg.contains("missing required property `name`")
            && msg.contains("missing required property `age`"),
        "should carry the specific schema failures: {msg}"
    );
    // …and it does NOT lead with the generic bare-LLM message that hid them.
    assert!(
        !msg.starts_with("workflow agent node has no roster agent"),
        "the schema failure must not be masked by the generic lead: {msg}"
    );
}

/// T2 — any other request (here an agent node with no `agent_ref`, whose
/// request is the node config) keeps the generic message byte-identical.
#[tokio::test]
async fn unwired_llm_keeps_generic_message_for_non_auto_fix_request() {
    let EngineError::Capability(msg) = UnwiredLlm
        .complete(json!({ "prompt": "hi" }), None)
        .await
        .expect_err("an unwired llm must error")
    else {
        panic!("expected a capability error");
    };
    assert_eq!(
        msg, BARE_LLM_UNWIRED_MESSAGE,
        "a non-auto-fix request must get the byte-identical generic message"
    );
}

/// T4 — a `coerce_to_schema` request whose `errors` is empty or missing (or
/// not an array of strings) falls back to the generic message rather than
/// emitting an empty schema-error string or panicking.
#[tokio::test]
async fn unwired_llm_falls_back_when_auto_fix_carries_no_errors() {
    for request in [
        json!({ "task": "coerce_to_schema" }),
        json!({ "task": "coerce_to_schema", "errors": [] }),
        json!({ "task": "coerce_to_schema", "errors": "oops" }),
        json!({ "task": "coerce_to_schema", "errors": [1, 2] }),
    ] {
        let EngineError::Capability(msg) = UnwiredLlm
            .complete(request.clone(), None)
            .await
            .expect_err("an unwired llm must error")
        else {
            panic!("expected a capability error for {request}");
        };
        assert_eq!(
            msg, BARE_LLM_UNWIRED_MESSAGE,
            "a coerce_to_schema request with no usable errors must fall back \
             to the generic message: {request}"
        );
    }
}

// ── Issue #661 (L2): a workspace mkdir failure aborts the live build ──

/// T5 — live mode with an impossible `workspace_root` (a path rooted under a
/// regular file) fails the build with a `Harness` error naming the path and
/// the underlying I/O cause, instead of warning past it and handing back a
/// bundle whose effects are rooted at a directory that does not exist.
#[tokio::test]
async fn build_capabilities_live_errors_when_workspace_cannot_be_created() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A regular file where a directory would need to be: `create_dir_all`
    // under it fails with ENOTDIR.
    let not_a_dir = dir.path().join("not-a-dir");
    std::fs::write(&not_a_dir, b"x").expect("write file");

    let (mut deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
        "http://127.0.0.1:1/unused".to_string(),
        dir.path(),
    );
    deps.workspace_root = not_a_dir.clone();
    let record = crate::workflows::gated_tool_turn_test::record();

    // `Capabilities` is not `Debug`, so match rather than `expect_err`.
    let err = match build_capabilities(
        single_turn(&deps),
        deps,
        &record,
        RunContext {
            workflow_id: "wf",
            run_id: "run:1",
            checkpoint_thread_id: "run:1",
            workflow_fingerprint: "fp:1",
            run_request: None,
            trigger_input: &Value::Null,
            started_by: crate::ports::types::StartedBy::Operator,
            dry_run: false, // live: the workspace mkdir runs
            notices: RunNotices::default(),
            board: RunBoard::default(),
            blocks: Default::default(),
            capped: Default::default(),
            halted: Default::default(),
            approvals: Default::default(),
            artifacts: Default::default(),
            runs: None,
            deep: None,
            attempts: Default::default(),
            child_gates: Default::default(),
        },
    )
    .await
    {
        Ok(_) => panic!("an uncreatable workspace must fail the build"),
        Err(err) => err,
    };

    let crate::error::OpenCompanyError::Harness(msg) = &err else {
        panic!("expected a Harness error, got {err:?}");
    };
    assert!(
        msg.contains("could not create its workspace directory"),
        "message should name the failure: {msg}"
    );
    assert!(
        msg.contains("not-a-dir"),
        "message should name the offending path: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("not a directory"),
        "message should carry the underlying I/O cause: {msg}"
    );
}

/// T6 — the same impossible root is harmless for a dry run: it builds no
/// workspace, so the bundle assembles fine.
#[tokio::test]
async fn build_capabilities_dry_ignores_an_impossible_workspace_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_dir = dir.path().join("not-a-dir");
    std::fs::write(&not_a_dir, b"x").expect("write file");

    let (mut deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
        "http://127.0.0.1:1/unused".to_string(),
        dir.path(),
    );
    deps.workspace_root = not_a_dir;
    let record = crate::workflows::gated_tool_turn_test::record();

    build_capabilities(
        single_turn(&deps),
        deps,
        &record,
        RunContext {
            workflow_id: "wf",
            run_id: "run:1",
            checkpoint_thread_id: "run:1",
            workflow_fingerprint: "fp:1",
            run_request: None,
            trigger_input: &Value::Null,
            started_by: crate::ports::types::StartedBy::Operator,
            dry_run: true, // dry: no workspace mkdir at all
            notices: RunNotices::default(),
            board: RunBoard::default(),
            blocks: Default::default(),
            capped: Default::default(),
            halted: Default::default(),
            approvals: Default::default(),
            artifacts: Default::default(),
            runs: None,
            deep: None,
            attempts: Default::default(),
            child_gates: Default::default(),
        },
    )
    .await
    .expect("a dry build never touches the workspace");
}

// ---- the transcript fold (the record a workflow node now leaves) -------

mod transcript_fold {
    use super::super::transcript_from_steps;
    use crate::ports::types::{TurnStep, TurnStepFailure, TurnStepKind, TurnStepStatus};

    fn step(kind: TurnStepKind, status: TurnStepStatus, label: &str) -> TurnStep {
        TurnStep {
            kind,
            status,
            label: label.to_string(),
            ..TurnStep::default()
        }
    }

    #[test]
    fn a_tool_less_turn_folds_to_nothing() {
        // The zero-steps tell: a memory-served answer genuinely did nothing
        // worth recording, and an empty transcript says exactly that.
        assert!(transcript_from_steps(&[]).is_empty());
    }

    #[test]
    fn each_step_kind_maps_to_an_engine_word() {
        let steps = vec![
            step(TurnStepKind::Thinking, TurnStepStatus::Ok, "Thinking"),
            step(TurnStepKind::Note, TurnStepStatus::Ok, "note"),
            step(TurnStepKind::ToolCall, TurnStepStatus::Ok, "shell"),
            step(TurnStepKind::ToolCall, TurnStepStatus::Error, "shell"),
            step(TurnStepKind::ToolCall, TurnStepStatus::Running, "shell"),
            step(
                TurnStepKind::ToolCall,
                TurnStepStatus::AwaitingApproval,
                "shell",
            ),
        ];
        assert_eq!(
            transcript_from_steps(&steps)
                .iter()
                .map(|e| e.kind.clone())
                .collect::<Vec<_>>(),
            [
                "agent_thinking",
                "agent_message",
                "tool_result",
                "error",
                "tool_call",
                "tool_awaiting_approval",
            ]
        );
    }

    #[test]
    fn a_parked_call_is_not_folded_as_a_failure() {
        // The #411 distinction, preserved through the fold: the one step an
        // operator can act on must not read as a crash.
        let parked = transcript_from_steps(&[step(
            TurnStepKind::ToolCall,
            TurnStepStatus::AwaitingApproval,
            "shell",
        )]);
        assert_eq!(parked[0].kind, "tool_awaiting_approval");
        assert_ne!(parked[0].kind, "error");
    }

    #[test]
    fn the_line_carries_what_the_step_knows() {
        let entry = transcript_from_steps(&[TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "shell".to_string(),
            detail: Some("python3 solve.py".to_string()),
            result: Some("3 lines".to_string()),
            truncated: true,
            elapsed_ms: Some(1200),
            failure: None,
        }]);
        assert_eq!(
            entry[0].text,
            "shell: python3 solve.py → 3 lines [truncated] (1200ms)"
        );
    }

    #[test]
    fn a_failure_class_rides_the_line_in_snake_case() {
        let entry = transcript_from_steps(&[TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Error,
            label: "github.merge".to_string(),
            failure: Some(TurnStepFailure::BlockedByPolicy),
            ..TurnStep::default()
        }]);
        assert!(
            entry[0].text.contains("[blocked_by_policy]"),
            "got {:?}",
            entry[0].text
        );
    }

    #[test]
    fn empty_detail_and_result_add_no_punctuation() {
        // A bare label must not fold to "label: " or "label → ".
        let entry = transcript_from_steps(&[TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "workspace_read".to_string(),
            detail: Some(String::new()),
            result: Some(String::new()),
            ..TurnStep::default()
        }]);
        assert_eq!(entry[0].text, "workspace_read");
    }

    #[test]
    fn one_long_step_cannot_eat_the_records_budget() {
        // `TranscriptEntry::bounded` is the crate's own ceiling; the fold
        // must go through it rather than around it.
        let entry = transcript_from_steps(&[TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "shell".to_string(),
            result: Some("x".repeat(64 * 1024)),
            ..TurnStep::default()
        }]);
        assert!(
            entry[0].text.len() < 8 * 1024,
            "entry was {} bytes — bounded() was bypassed",
            entry[0].text.len()
        );
        assert!(entry[0].text.ends_with("…[truncated]"));
    }

    #[test]
    fn order_is_preserved() {
        // A transcript read out of order is not a transcript.
        let steps: Vec<TurnStep> = (0..5)
            .map(|i| {
                step(
                    TurnStepKind::ToolCall,
                    TurnStepStatus::Ok,
                    &format!("step{i}"),
                )
            })
            .collect();
        assert_eq!(
            transcript_from_steps(&steps)
                .iter()
                .map(|e| e.text.clone())
                .collect::<Vec<_>>(),
            ["step0", "step1", "step2", "step3", "step4"]
        );
    }

    #[test]
    fn every_failure_class_has_a_stable_snake_case_wire_word() {
        for (failure, expected) in [
            (TurnStepFailure::Declined, "declined"),
            (TurnStepFailure::BlockedByPolicy, "blocked_by_policy"),
            (TurnStepFailure::Unauthorized, "unauthorized"),
            (TurnStepFailure::MissingPermission, "missing_permission"),
            (TurnStepFailure::MissingApp, "missing_app"),
            (TurnStepFailure::NotFound, "not_found"),
            (TurnStepFailure::Timeout, "timeout"),
            (TurnStepFailure::Unavailable, "unavailable"),
            (TurnStepFailure::Failed, "failed"),
        ] {
            assert_eq!(failure.wire_word(), expected);
        }
    }
}

// ---- the attempt row a workflow node now opens ------------------------

mod attempt {
    use super::*;
    use crate::ports::{NewRun, RunFilter, RunStatus, RunStore};

    fn store() -> Arc<dyn RunStore> {
        let dir = tempfile::Builder::new()
            .prefix("oc-attempt-")
            .tempdir()
            .expect("tempdir");
        let path = dir.path().to_path_buf();
        // The tempdir must outlive the store; leak it, this is a test.
        std::mem::forget(dir);
        Arc::new(crate::store::fs_ops::FsOps::new(&path))
    }

    #[tokio::test]
    async fn a_node_run_is_addressable_by_its_workflow_run() {
        // The join, end to end at the port: this is the query that had no
        // answer before, because a node's attempt had neither a card nor a
        // conversation to be found by.
        let runs = store();
        let company = CompanyId::new("acme");
        for (id, node) in [("a", "solve"), ("b", "check")] {
            let row = runs
                .create_run(
                    &company,
                    NewRun::for_workflow_node(id, "run-1", node, "programmer"),
                )
                .await
                .expect("create");
            runs.begin_run_untriggered(&company, &row.id)
                .await
                .expect("begin");
        }

        let found = runs
            .list_runs(&company, &RunFilter::for_workflow_run("run-1"))
            .await
            .expect("list");
        assert_eq!(found.len(), 2);
        assert!(
            found.iter().all(|r| r.status == RunStatus::Running),
            "an untriggered begin still moves the row to Running"
        );
        assert!(
            found.iter().all(|r| r.trigger_event_seq.is_none()),
            "a workflow node has no driving journal event, and says so"
        );
        let mut nodes: Vec<&str> = found.iter().filter_map(|r| r.node_id.as_deref()).collect();
        nodes.sort_unstable();
        assert_eq!(nodes, ["check", "solve"]);
    }

    #[tokio::test]
    async fn an_untriggered_begin_refuses_an_illegal_transition() {
        // The transition legality that lives on the port must not be
        // bypassed by the sibling entry point.
        let runs = store();
        let company = CompanyId::new("acme");
        let row = runs
            .create_run(
                &company,
                NewRun::for_workflow_node("a", "run-1", "solve", "p"),
            )
            .await
            .expect("create");
        runs.begin_run_untriggered(&company, &row.id)
            .await
            .expect("first begin");
        assert!(
            runs.begin_run_untriggered(&company, &row.id).await.is_err(),
            "Running -> Running is not a legal transition"
        );
    }
}

// ── Issue #1861: a node blocked on something a person can answer ────────

/// A turn double that fails with an arbitrary message, so the classifier
/// sees a real error chain rather than a hand-built string.
struct FailingTurn(String);

#[async_trait]
impl RunTurn for FailingTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        Err(crate::error::OpenCompanyError::Harness(self.0.clone()))
    }

    async fn run_steered(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        _control: &crate::company::steer::SteerControl,
        chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.run(company, agent_id, message, chat).await
    }

    async fn run_steered_background(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.run(
            company,
            agent_id,
            message,
            crate::runtime::delegation::ChatTarget::channel(None),
        )
        .await
    }
}

async fn run_failing_node(
    dir: &std::path::Path,
    error: &str,
) -> (RunBlocks, Arc<crate::runtime::journal::RuntimeJournal>) {
    let (deps, journal) = crate::workflows::gated_tool_turn_test::deps(String::new(), dir);
    let record = crate::workflows::gated_tool_turn_test::record();
    let board_claim = Arc::new(deps.delegations.claim_board("run-1861"));
    let publish_refusal_claim =
        Arc::new(deps.pending_publishes.claim_refusals_for_run("run-1861"));
    let blocks = RunBlocks::default();
    let runner = HarnessAgentRunner::new(
        Arc::new(FailingTurn(error.to_string())),
        deps,
        record,
        CompanyId::new("acme"),
        "wf-1".to_string(),
        "run-1861".to_string(),
        None,
        json!({}),
        crate::ports::types::StartedBy::Operator,
        RunNotices::default(),
        RunBoard::default(),
        blocks.clone(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        RunArtifacts::default(),
        board_claim,
        publish_refusal_claim,
    );
    let outcome = runner
        .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
        .await;
    assert!(outcome.is_err(), "a failed node must not advance the graph");
    (blocks, journal)
}

/// The workflow half of #1861. A node that died on a model id the provider
/// rejects is answerable, so it reaches the operator as a parked question
/// and the node holds open — through the same #881 machinery an agent's own
/// blocked tool call already uses, which is what makes the two arrive as
/// one shape.
#[tokio::test]
async fn a_node_that_fails_on_a_rejected_model_parks_a_blocker() {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let dir = tempfile::Builder::new()
        .prefix("oc-1861-")
        .tempdir()
        .expect("tempdir");
    let (blocks, journal) = run_failing_node(
        dir.path(),
        "the model `gpt-nonexistent` does not exist or you do not have access to it",
    )
    .await;

    let blocked = blocks.take();
    assert_eq!(blocked.len(), 1, "the node is held open, not failed");
    assert_eq!(blocked[0].node_id, "gather");
    assert!(
        blocked[0].tools.is_empty(),
        "nothing the agent called was gated; the node itself stopped"
    );
    assert_eq!(
        blocked[0].approval_ids.len(),
        1,
        "the block must name the approval it is decidable through"
    );

    let parked = journal
        .pending()
        .into_iter()
        .find(|p| p.effect.kind.starts_with("blocker."))
        .expect("a blocker is parked");
    assert_eq!(parked.effect.kind, "blocker.infrastructure");
    assert_eq!(parked.effect.run_id.as_deref(), Some("run-1861"));

    let payload: BlockerPayload =
        serde_json::from_value(parked.effect.payload.clone()).expect("payload round-trips");
    assert_eq!(payload.kind, BlockerKind::Infrastructure);
    assert_eq!(payload.source, BlockerSource::Provider);
    assert_eq!(
        payload.step,
        Some(BlockerStep::Node {
            run_id: "run-1861".to_string(),
            node_id: "gather".to_string()
        }),
        "a run has no card to name instead, and #1864 restarts the node"
    );
}

/// The conservative default holds here too: an error the classifier does
/// not recognise fails the node exactly as it did before, and holds nothing
/// open on a question nobody was asked.
#[tokio::test]
async fn an_unrecognised_node_failure_still_fails_and_parks_nothing() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1861b-")
        .tempdir()
        .expect("tempdir");
    let (blocks, journal) = run_failing_node(dir.path(), "index out of bounds").await;

    assert!(
        blocks.take().is_empty(),
        "an unrecognised failure is a failure, and the node must settle as one"
    );
    assert!(
        journal
            .pending()
            .into_iter()
            .all(|p| !p.effect.kind.starts_with("blocker.")),
        "nothing was parked"
    );
}

/// A transient stop is recognised precisely so it does **not** hold the run
/// open: a rate limit resolves itself and asking about it wastes the ask.
#[tokio::test]
async fn a_transient_node_failure_does_not_hold_the_run_open() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1861c-")
        .tempdir()
        .expect("tempdir");
    let (blocks, _journal) = run_failing_node(
        dir.path(),
        "hosted inference returned 429: rate limit exceeded",
    )
    .await;
    assert!(blocks.take().is_empty());
}

/// Issue #2005: the engine-side trigger reader — what an answered blocker
/// riding the continuation's trigger input actually does to the node it
/// names.
mod node_blocker_answer {
    use super::*;
    use crate::ports::blockers::{BlockerKind, BlockerSource, BlockerVerdict};
    use crate::runtime::workflow_resume::{
        BlockerAnswer, CONTINUATION_BLOCKER_KEY, with_blocker_answer, workflow_node_turn_key,
    };

    const RUN_ID: &str = "run-2005";

    /// A turn double that records the message it was handed, so an amend's
    /// injection is provable and a skip's non-execution is too.
    struct MessageRecordingTurn {
        messages: std::sync::Mutex<Vec<String>>,
    }

    impl MessageRecordingTurn {
        fn new() -> Self {
            Self {
                messages: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn messages(&self) -> Vec<String> {
            self.messages.lock().expect("messages").clone()
        }
    }

    #[async_trait]
    impl RunTurn for MessageRecordingTurn {
        async fn run(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            message: &str,
            _chat_id: crate::runtime::delegation::ChatTarget<'_>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.messages
                .lock()
                .expect("messages")
                .push(message.to_string());
            Ok(ok_outcome())
        }

        async fn run_steered(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            message: &str,
            _control: &crate::company::steer::SteerControl,
            _chat_id: crate::runtime::delegation::ChatTarget<'_>,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.messages
                .lock()
                .expect("messages")
                .push(message.to_string());
            Ok(ok_outcome())
        }

        async fn run_steered_background(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            message: &str,
            _control: &crate::company::steer::SteerControl,
            _chat: crate::runtime::delegation::ChatTarget<'_>,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.messages
                .lock()
                .expect("messages")
                .push(message.to_string());
            Ok(ok_outcome())
        }

        async fn run_background_workflow(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            message: &str,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
            _workflow_run_id: &str,
            _node_id: &str,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.messages
                .lock()
                .expect("messages")
                .push(message.to_string());
            Ok(ok_outcome())
        }
    }

    fn answered(node: &str, verdict: BlockerVerdict, answer: &str) -> Value {
        with_blocker_answer(
            json!({ "topic": "quarterly numbers" }),
            &BlockerAnswer {
                node: node.to_string(),
                verdict,
                answer: answer.to_string(),
            },
        )
    }

    async fn runner_with(
        dir: &std::path::Path,
        turn: Arc<MessageRecordingTurn>,
        trigger_input: Value,
    ) -> HarnessAgentRunner {
        let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(String::new(), dir);
        let board_claim = Arc::new(deps.delegations.claim_board(RUN_ID));
        let publish_refusal_claim =
            Arc::new(deps.pending_publishes.claim_refusals_for_run(RUN_ID));
        HarnessAgentRunner::new(
            turn,
            deps,
            crate::workflows::gated_tool_turn_test::record(),
            CompanyId::new("acme"),
            "reporting".to_string(),
            RUN_ID.to_string(),
            None,
            trigger_input,
            crate::ports::types::StartedBy::Operator,
            RunNotices::default(),
            RunBoard::default(),
            RunBlocks::default(),
            RunCappedNodes::default(),
            RunApprovals::default(),
            RunArtifacts::default(),
            board_claim,
            publish_refusal_claim,
        )
    }

    fn tmp(prefix: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("tempdir")
    }

    /// A skip proceeds past the node without spending a turn on the
    /// question the operator just waived.
    #[tokio::test]
    async fn a_skipped_node_does_not_run_its_turn() {
        let dir = tmp("oc-2005-skip-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            answered("gather", BlockerVerdict::Skip, ""),
        )
        .await;

        let (value, outcome) = runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await
            .expect("a skipped node still produces an output the branch can bind");

        assert!(
            turn.messages().is_empty(),
            "a waived node must not spend a turn: {:?}",
            turn.messages()
        );
        assert!(outcome.reply.contains("skipped"), "{}", outcome.reply);
        assert_eq!(value["agent_ref"], "researcher");
    }

    /// An amend re-runs the node carrying the operator's correction — the
    /// workflow twin of the card path's note append.
    #[tokio::test]
    async fn an_amended_node_re_runs_carrying_the_operators_words() {
        let dir = tmp("oc-2005-amend-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            answered("gather", BlockerVerdict::Amend, "use gpt-4o-mini instead"),
        )
        .await;

        runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await
            .expect("an amended node runs");

        let messages = turn.messages();
        assert_eq!(messages.len(), 1, "the node runs exactly once");
        assert!(
            messages[0].contains("use gpt-4o-mini instead"),
            "the correction has to reach the turn, or the re-run repeats the failure: {}",
            messages[0]
        );
    }

    /// A retry runs the step again as it was — no correction to inject.
    #[tokio::test]
    async fn a_retried_node_runs_again_as_it_was() {
        let dir = tmp("oc-2005-retry-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            answered("gather", BlockerVerdict::Retry, ""),
        )
        .await;

        runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await
            .expect("a retried node runs");

        let messages = turn.messages();
        assert_eq!(messages.len(), 1);
        assert!(
            !messages[0].contains("Answer from the operator"),
            "a bare retry carries no words: {}",
            messages[0]
        );
    }

    /// One node's answer is not the graph's: every other node runs as it
    /// always did.
    #[tokio::test]
    async fn an_answer_for_another_node_leaves_this_one_alone() {
        let dir = tmp("oc-2005-other-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            answered("review", BlockerVerdict::Skip, ""),
        )
        .await;

        runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await
            .expect("an unanswered node runs");

        assert_eq!(turn.messages().len(), 1);
    }

    /// An unreadable answer fails the node rather than degrading to
    /// "nobody answered" — the degrade would spend a turn on the identical
    /// failure with the operator's decision gone.
    #[tokio::test]
    async fn an_unreadable_answer_fails_the_node_rather_than_running_it() {
        let dir = tmp("oc-2005-garbled-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            json!({
                CONTINUATION_BLOCKER_KEY: [{ "node": "gather", "verdict": "shrug" }]
            }),
        )
        .await;

        let outcome = runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await;

        assert!(outcome.is_err(), "a garbled answer must stop the node");
        assert!(turn.messages().is_empty(), "and must not spend a turn");
    }

    /// A cancel starts no run at all, so a node reached carrying one is a
    /// host bug — and stops loudly rather than carrying on as if the
    /// operator had said yes.
    #[tokio::test]
    async fn a_cancelled_answer_stops_the_node() {
        let dir = tmp("oc-2005-cancel-");
        let turn = Arc::new(MessageRecordingTurn::new());
        let runner = runner_with(
            dir.path(),
            turn.clone(),
            json!({
                CONTINUATION_BLOCKER_KEY: [{ "node": "gather", "verdict": "cancel" }]
            }),
        )
        .await;

        let outcome = runner
            .run_turn("researcher", json!({ "node_id": "gather", "prompt": "go" }))
            .await;

        assert!(outcome.is_err());
        assert!(turn.messages().is_empty());
    }

    /// The other half of the thread: a blocker's park has to stash what the
    /// answer's re-entry will need. The gated-call arm cannot cover it — a
    /// turn that parked no gated call returns before reaching that arm, and
    /// the runner's settle-time pass refuses to arm a turn that is not
    /// already armed — so without this the answer reaches a resume with no
    /// run to continue.
    #[tokio::test]
    async fn parking_a_node_blocker_stashes_the_run_its_answer_re_enters() {
        let dir = tmp("oc-2005-stash-");
        let (deps, _journal) =
            crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
        let parking = deps
            .delivery
            .clone()
            .expect("delivery")
            .parking
            .clone()
            .expect("parking");
        let trigger_input = json!({ "topic": "quarterly numbers" });
        let board_claim = Arc::new(deps.delegations.claim_board(RUN_ID));
        let publish_refusal_claim =
            Arc::new(deps.pending_publishes.claim_refusals_for_run(RUN_ID));
        let runner = HarnessAgentRunner::new(
            single_turn(&deps),
            deps,
            crate::workflows::gated_tool_turn_test::record(),
            CompanyId::new("acme"),
            "reporting".to_string(),
            RUN_ID.to_string(),
            None,
            trigger_input.clone(),
            crate::ports::types::StartedBy::Operator,
            RunNotices::default(),
            RunBoard::default(),
            RunBlocks::default(),
            RunCappedNodes::default(),
            RunApprovals::default(),
            RunArtifacts::default(),
            board_claim,
            publish_refusal_claim,
        );

        let parked = runner
            .park_node_blocker_as(
                "gather",
                "the model id `gpt-nope` was rejected",
                BlockerKind::Infrastructure,
                BlockerSource::Provider,
                "a model id this provider serves",
            )
            .await;
        assert!(parked.is_some(), "the blocker parks");

        let stashed = parking
            .blocked_nodes
            .peek(&workflow_node_turn_key(RUN_ID, "gather"))
            .expect("a parked blocker must stash the run its answer re-enters");
        assert_eq!(stashed.workflow_id, "reporting");
        assert_eq!(stashed.input, trigger_input);
    }

    /// A resolver racing in against a live blocker park must always find
    /// the stash already armed. This spies on the approval gate's own
    /// `park` call and captures whether the stash is armed at that exact
    /// point — deterministic, no wall-clock race needed, on the same
    /// principle as
    /// `park_and_journal_arms_the_continuation_slot_before_the_card_is_parkable`
    /// in `workflows::delivery`.
    #[tokio::test]
    async fn park_node_blocker_as_arms_the_stash_before_the_card_is_parkable() {
        use crate::ports::ApprovalGate;
        use crate::ports::types::{Actor, ApprovalId, Effect, PolicyDecision, Verdict};

        struct Spy {
            inner: Arc<dyn ApprovalGate>,
            blocked_nodes: crate::runtime::blocked_nodes::BlockedNodeQueue,
            turn: String,
            armed_at_park: std::sync::Mutex<Option<bool>>,
        }

        #[async_trait]
        impl ApprovalGate for Spy {
            async fn evaluate(
                &self,
                company: &CompanyId,
                effect: &Effect,
            ) -> crate::Result<PolicyDecision> {
                self.inner.evaluate(company, effect).await
            }

            async fn park(
                &self,
                company: &CompanyId,
                effect: Effect,
            ) -> crate::Result<ApprovalId> {
                *self.armed_at_park.lock().expect("spy lock") =
                    Some(self.blocked_nodes.is_armed(&self.turn));
                self.inner.park(company, effect).await
            }

            async fn resolve(
                &self,
                id: &ApprovalId,
                verdict: Verdict,
                by: Actor,
            ) -> crate::Result<Option<Effect>> {
                self.inner.resolve(id, verdict, by).await
            }
        }

        let dir = tmp("oc-2005-race-a-");
        let (mut deps, _journal) =
            crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
        let parking = deps
            .delivery
            .clone()
            .expect("delivery")
            .parking
            .clone()
            .expect("parking");

        let run_id = "run-2005-race-a".to_string();
        let turn = workflow_node_turn_key(&run_id, "gather");
        let spy = Arc::new(Spy {
            inner: parking.approvals.clone(),
            blocked_nodes: parking.blocked_nodes.clone(),
            turn: turn.clone(),
            armed_at_park: std::sync::Mutex::new(None),
        });
        let mut spied_parking = parking.clone();
        spied_parking.approvals = spy.clone();
        deps.delivery.as_mut().expect("delivery").parking = Some(spied_parking);

        let trigger_input = json!({ "topic": "quarterly numbers" });
        let board_claim = Arc::new(deps.delegations.claim_board(&run_id));
        let publish_refusal_claim =
            Arc::new(deps.pending_publishes.claim_refusals_for_run(&run_id));
        let runner = HarnessAgentRunner::new(
            single_turn(&deps),
            deps,
            crate::workflows::gated_tool_turn_test::record(),
            CompanyId::new("acme"),
            "reporting".to_string(),
            run_id,
            None,
            trigger_input,
            crate::ports::types::StartedBy::Operator,
            RunNotices::default(),
            RunBoard::default(),
            RunBlocks::default(),
            RunCappedNodes::default(),
            RunApprovals::default(),
            RunArtifacts::default(),
            board_claim,
            publish_refusal_claim,
        );

        let parked = runner
            .park_node_blocker_as(
                "gather",
                "the model id `gpt-nope` was rejected",
                BlockerKind::Infrastructure,
                BlockerSource::Provider,
                "a model id this provider serves",
            )
            .await;
        assert!(parked.is_some(), "the blocker parks");

        let captured = spy
            .armed_at_park
            .lock()
            .expect("spy lock")
            .expect("park was called");
        assert!(
            captured,
            "the blocked-node stash must already be armed by the time the approval gate's \
             park() runs, before the card becomes resolvable to a concurrent operator"
        );
    }

    /// The same proof as above, for the sibling site: a blocker card
    /// extracted from a node's gated-call batch inside `park_gated_calls`.
    #[tokio::test]
    async fn park_gated_calls_blocker_extraction_arms_the_stash_before_the_first_card_is_parkable()
     {
        use crate::harness::policy::{ApprovalRequest, ApprovalScope};
        use crate::ports::ApprovalGate;
        use crate::ports::blockers::BlockerPayload;
        use crate::ports::types::{
            Actor, ApprovalId, Effect, EffectGroup, PolicyDecision, Verdict,
        };

        struct Spy {
            inner: Arc<dyn ApprovalGate>,
            blocked_nodes: crate::runtime::blocked_nodes::BlockedNodeQueue,
            turn: String,
            armed_at_park: std::sync::Mutex<Option<bool>>,
        }

        #[async_trait]
        impl ApprovalGate for Spy {
            async fn evaluate(
                &self,
                company: &CompanyId,
                effect: &Effect,
            ) -> crate::Result<PolicyDecision> {
                self.inner.evaluate(company, effect).await
            }

            async fn park(
                &self,
                company: &CompanyId,
                effect: Effect,
            ) -> crate::Result<ApprovalId> {
                *self.armed_at_park.lock().expect("spy lock") =
                    Some(self.blocked_nodes.is_armed(&self.turn));
                self.inner.park(company, effect).await
            }

            async fn resolve(
                &self,
                id: &ApprovalId,
                verdict: Verdict,
                by: Actor,
            ) -> crate::Result<Option<Effect>> {
                self.inner.resolve(id, verdict, by).await
            }
        }

        let dir = tmp("oc-2005-race-b-");
        let (mut deps, _journal) =
            crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
        let parking = deps
            .delivery
            .clone()
            .expect("delivery")
            .parking
            .clone()
            .expect("parking");

        let run_id = "run-2005-race-b".to_string();
        let turn = workflow_node_turn_key(&run_id, "work");
        let spy = Arc::new(Spy {
            inner: parking.approvals.clone(),
            blocked_nodes: parking.blocked_nodes.clone(),
            turn: turn.clone(),
            armed_at_park: std::sync::Mutex::new(None),
        });
        let mut spied_parking = parking.clone();
        spied_parking.approvals = spy.clone();
        deps.delivery.as_mut().expect("delivery").parking = Some(spied_parking);

        let queue = deps.approval_requests.clone();
        let trigger_input = json!({ "topic": "quarterly numbers" });
        let board_claim = Arc::new(deps.delegations.claim_board(&run_id));
        let publish_refusal_claim =
            Arc::new(deps.pending_publishes.claim_refusals_for_run(&run_id));
        let runner = HarnessAgentRunner::new(
            single_turn(&deps),
            deps,
            crate::workflows::gated_tool_turn_test::record(),
            CompanyId::new("acme"),
            "reporting".to_string(),
            run_id.clone(),
            None,
            trigger_input,
            crate::ports::types::StartedBy::Operator,
            RunNotices::default(),
            RunBoard::default(),
            RunBlocks::default(),
            RunCappedNodes::default(),
            RunApprovals::default(),
            RunArtifacts::default(),
            board_claim,
            publish_refusal_claim,
        );

        let payload = BlockerPayload {
            kind: BlockerKind::Information,
            source: BlockerSource::AgentQuestion,
            step: None,
            reason: "which quarter should I report?".to_string(),
            needed: "the quarter to report on".to_string(),
            group_key: None,
        };
        let effect_kind = payload.effect_kind();
        let reason = payload.reason.clone();
        let payload_value = serde_json::to_value(&payload).expect("payload serializes");

        let claim = queue.claim(ApprovalScope::Run(run_id.clone()));
        claim
            .scoped(async {
                queue.push(ApprovalRequest {
                    tool: "escalate_to_human".to_string(),
                    reason,
                    effect: Effect {
                        kind: effect_kind,
                        group: EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: payload_value,
                        agent: None,
                        run_id: None,
                    },
                });
            })
            .await;

        claim
            .scoped(runner.park_gated_calls(Some("work"), "work", &turn))
            .await;

        let captured = spy
            .armed_at_park
            .lock()
            .expect("spy lock")
            .expect("park was called");
        assert!(
            captured,
            "a blocker card extracted from a node's gated-call batch must find the stash \
             already armed by the time the approval gate's park() runs"
        );
    }
}

// ── the recovery ladder's peer rung ──────────────────────────────────────

/// The fixture roster plus one teammate whose role and description match a
/// question about a customer's renewal, so [`pick_peer`] has somebody to
/// choose. The node itself runs as `researcher`, which is deliberately not
/// on the roster.
fn record_with_peer() -> CompanyRecord {
    let mut record = crate::workflows::gated_tool_turn_test::record();
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\n[[agent]]\nid = \"cfo\"\nrole = \"Chief Financial \
         Officer\"\ndescription = \"Owns renewal contracts and customer pricing\"\n",
    )
    .expect("manifest parses");
    record
}

/// A [`RunTurn`] whose node turn refuses for want of a fact, and whose peer
/// consultation answers with `peer_reply` — optionally staging board work
/// on the way, standing in for a consulted teammate whose tools wrote.
struct ConsultedPeerTurn {
    peer_reply: &'static str,
    consults: std::sync::atomic::AtomicUsize,
    node_turns: std::sync::atomic::AtomicUsize,
    stage: Option<HarnessDeps>,
}

impl ConsultedPeerTurn {
    fn new(peer_reply: &'static str) -> Self {
        Self {
            peer_reply,
            consults: std::sync::atomic::AtomicUsize::new(0),
            node_turns: std::sync::atomic::AtomicUsize::new(0),
            stage: None,
        }
    }

    fn staging(peer_reply: &'static str, deps: HarnessDeps) -> Self {
        Self {
            stage: Some(deps),
            ..Self::new(peer_reply)
        }
    }

    fn consults(&self) -> usize {
        self.consults.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl RunTurn for ConsultedPeerTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("workflow agent nodes route through run_background_workflow")
    }

    async fn run_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.consults
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(deps) = self.stage.as_ref() {
            deps.delegations
                .push(crate::harness::orchestrator::Delegation::SpawnTask {
                    title: "Chase the renewal paperwork".to_string(),
                    note: None,
                    assignee: None,
                });
            deps.pending_publishes
                .push_refusal("consultation-note.md".to_string());
            deps.approval_requests
                .push(crate::harness::policy::ApprovalRequest {
                    tool: "send_email".to_string(),
                    reason: "the consulted peer tried a gated tool".to_string(),
                    effect: crate::ports::types::Effect {
                        kind: "email.send".to_string(),
                        group: crate::ports::types::EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: json!({ "to": "someone@example.com" }),
                        agent: Some("cfo".to_string()),
                        run_id: None,
                    },
                });
        }
        Ok(crate::harness::TurnOutcome {
            reply: self.peer_reply.to_string(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }

    async fn run_background_workflow(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        _workflow_run_id: &str,
        _node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.node_turns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(crate::harness::TurnOutcome {
            reply: "I cannot draft the email without the customer's renewal date.".to_string(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
}

/// The verify node every peer-rung test below drives.
fn verify_node() -> Value {
    json!({
        "node_id": "draft",
        "prompt": "Draft the customer email.",
        "verify": { "criteria": "must include the customer's renewal date" }
    })
}

/// Builds a runner over `turn` with the peer-bearing roster, handing back
/// the run-scoped collectors a test asserts on.
#[allow(clippy::type_complexity)]
fn peer_runner(
    turn: Arc<dyn RunTurn>,
    deps: HarnessDeps,
    run_id: &str,
    runs: Option<Arc<dyn crate::ports::RunStore>>,
) -> (
    HarnessAgentRunner,
    RunBlocks,
    RunBoard,
    RunNotices,
    RunArtifacts,
) {
    let board_claim = Arc::new(deps.delegations.claim_board(run_id.to_string()));
    let publish_refusal_claim = Arc::new(
        deps.pending_publishes
            .claim_refusals_for_run(run_id.to_string()),
    );
    let blocks = RunBlocks::default();
    let board = RunBoard::default();
    let notices = RunNotices::default();
    let artifacts = RunArtifacts::default();
    let mut runner = HarnessAgentRunner::new(
        turn,
        deps,
        record_with_peer(),
        CompanyId::new("acme"),
        format!("wf-{run_id}"),
        run_id.to_string(),
        None,
        Value::Null,
        crate::ports::types::StartedBy::Operator,
        notices.clone(),
        board.clone(),
        blocks.clone(),
        RunCappedNodes::default(),
        RunApprovals::default(),
        artifacts.clone(),
        board_claim,
        publish_refusal_claim,
    );
    if let Some(runs) = runs {
        runner = runner.with_runs(Some(runs), None, RunAttempts::default());
    }
    (runner, blocks, board, notices, artifacts)
}

/// The rung's reason to exist: an information gap the fact store and the
/// workspace cannot close is put to one roster peer, and an answer the
/// re-verification judge accepts ships as the node's output carrying its
/// provenance.
#[tokio::test]
async fn a_peer_answer_the_judge_accepts_ships_the_recovered_context_block() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-accepted-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let turn = Arc::new(ConsultedPeerTurn::new("The renewal date is March 1st."));
    let (runner, blocks, _board, _notices, _artifacts) =
        peer_runner(turn.clone(), deps, "run-1866-peer-ok", None);

    let (_value, outcome) = runner
        .run_turn("researcher", verify_node())
        .await
        .expect("an answered consultation the judge accepts must let the node ship");

    assert_eq!(turn.consults(), 1, "exactly one peer turn is spent");
    assert!(
        outcome
            .reply
            .contains("peer cfo (Chief Financial Officer): The renewal date is March 1st."),
        "the shipped reply must carry the peer's answer and its provenance: {}",
        outcome.reply
    );
    assert!(
        outcome.reply.contains("Recovered company context:"),
        "the shipped reply must be the augmented text: {}",
        outcome.reply
    );
    let seen = script.seen.lock().expect("seen");
    assert_eq!(seen.len(), 2, "one judge call, then one re-verification");
    assert!(
        seen[1].to_string().contains("peer cfo"),
        "the re-verification judge must see the peer's answer, not the bare refusal"
    );
    assert!(
        blocks.take().is_empty(),
        "an accepted recovery blocks nobody"
    );
}

/// A peer answer is not privileged: the second judge still gets to refuse
/// it, and when it does the operator gets an Information blocker whose
/// recovery log names the peer that was asked.
#[tokio::test]
async fn a_peer_answer_the_judge_rejects_parks_an_information_blocker() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-rejected-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"retry\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let turn = Arc::new(ConsultedPeerTurn::new("I do not have that date either."));
    let (runner, blocks, _board, _notices, _artifacts) = peer_runner(
        turn.clone(),
        deps,
        "run-1866-peer-blocked",
        Some(runs.clone()),
    );

    let err = runner
        .run_turn("researcher", verify_node())
        .await
        .expect_err("an unclosed information gap must not advance downstream");
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("peer: cfo answered"),
        "the operator's blocker must say the peer was asked and answered: {message}"
    );
    assert_eq!(turn.consults(), 1);
    assert_eq!(
        blocks.take().len(),
        1,
        "the gap is parked as one blocked node"
    );
    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run("run-1866-peer-blocked".to_string()),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        crate::ports::RunStatus::Blocked,
        "an information gap is blocked on a person, not failed"
    );
}

/// The deterministic check outranks the peer. An answer the judge accepts
/// still has to satisfy the node's declared postcondition, and when it does
/// not the attempt fails rather than shipping.
#[tokio::test]
async fn a_peer_answer_the_judge_accepts_still_fails_its_postcondition() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-postcondition-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let runs: Arc<dyn crate::ports::RunStore> =
        Arc::new(crate::store::FsOps::new(dir.path().to_path_buf()));
    let turn = Arc::new(ConsultedPeerTurn::new("The renewal date is March 1st."));
    let (runner, blocks, _board, _notices, _artifacts) = peer_runner(
        turn.clone(),
        deps,
        "run-1866-peer-postcondition",
        Some(runs.clone()),
    );

    let err = runner
        .run_turn(
            "researcher",
            json!({
                "node_id": "draft",
                "prompt": "Draft the customer email.",
                "verify": { "criteria": "must include the customer's renewal date" },
                "postcondition": { "require": "field_present", "field": "items" }
            }),
        )
        .await
        .expect_err("a recovered reply must still clear the deterministic check");
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("items"),
        "the halt must name what the recovered output was missing: {message}"
    );
    assert!(
        blocks.take().is_empty(),
        "a failed postcondition asks nobody for anything"
    );
    let attempts = runs
        .list_runs(
            &CompanyId::new("acme"),
            &crate::ports::RunFilter::for_workflow_run(
                "run-1866-peer-postcondition".to_string(),
            ),
        )
        .await
        .expect("list attempts");
    assert_eq!(attempts[0].status, crate::ports::RunStatus::Failed);
}

/// A consultation was asked a question, not given authority. Whatever the
/// consulted peer staged on the shared board and publish queues is thrown
/// away, so the *next* node's drain — the path that would otherwise execute
/// it and attribute it to this run — finds nothing.
#[tokio::test]
async fn a_consultation_that_stages_board_work_leaves_nothing_behind() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-no-authority-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"continue\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let queues = deps.clone();
    let turn = Arc::new(ConsultedPeerTurn::staging(
        "The renewal date is March 1st.",
        deps.clone(),
    ));
    let (runner, blocks, board, notices, _artifacts) =
        peer_runner(turn.clone(), deps, "run-1866-peer-authority", None);

    runner
        .run_turn("researcher", verify_node())
        .await
        .expect("the consultation answered, so the node ships");
    assert_eq!(turn.consults(), 1);

    // The node that runs next is what would execute a leaked staging: its
    // own post-turn drain reads the same queues.
    runner
        .run_turn(
            "researcher",
            json!({ "node_id": "next", "prompt": "carry on" }),
        )
        .await
        .expect("a plain node runs");

    assert!(
        board.take().is_empty(),
        "a consultation must open no card on the run's board"
    );
    assert!(
        blocks.take().is_empty(),
        "a consultation settles nothing and blocks nobody"
    );
    assert!(
        notices.take().is_empty(),
        "no operator notice may be raised for work a consultation only staged"
    );
    assert_eq!(
        queues.approval_requests.queued(),
        0,
        "a consultation must not leave an approval card for the operator's next chat cycle \
         to drain as if the operator had asked for it"
    );
}

/// A consulted peer may be the orchestrator, which can run a whole workflow
/// — whose nodes reach this same recover path. The rung must fire once down
/// the whole stack, not once per nested node.
#[tokio::test]
async fn a_consultation_cannot_re_enter_the_recovery_ladder() {
    struct ReentrantPeerTurn {
        consults: std::sync::atomic::AtomicUsize,
        node_turns: std::sync::atomic::AtomicUsize,
        runner: std::sync::OnceLock<Arc<HarnessAgentRunner>>,
    }

    #[async_trait]
    impl RunTurn for ReentrantPeerTurn {
        async fn run(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            _message: &str,
            _chat: crate::runtime::delegation::ChatTarget<'_>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            unreachable!("nodes route through run_background_workflow")
        }

        async fn run_steered(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            _message: &str,
            _control: &crate::company::steer::SteerControl,
            _chat: crate::runtime::delegation::ChatTarget<'_>,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            unreachable!("nodes route through run_background_workflow")
        }

        async fn run_steered_background(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            _message: &str,
            _control: &crate::company::steer::SteerControl,
            _chat: crate::runtime::delegation::ChatTarget<'_>,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            unreachable!("nodes route through run_background_workflow")
        }

        async fn run_background(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            _message: &str,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.consults
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // The consulted peer runs a nested graph node, which reaches
            // the same recover path.
            let runner = self.runner.get().expect("runner wired").clone();
            let _ = Box::pin(runner.run_turn("researcher", verify_node())).await;
            Ok(crate::harness::TurnOutcome {
                reply: String::new(),
                steps: Vec::new(),
                hit_iteration_cap: false,
                abnormal_stop: None,
                halted_for_spend: None,
                budget_paused: None,
            })
        }

        async fn run_background_workflow(
            &self,
            _company: &CompanyId,
            _agent_id: &str,
            _message: &str,
            _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
            _workflow_run_id: &str,
            _node_id: &str,
        ) -> crate::Result<crate::harness::TurnOutcome> {
            self.node_turns
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::harness::TurnOutcome {
                reply: "I cannot draft the email without the customer's renewal date."
                    .to_string(),
                steps: Vec::new(),
                hit_iteration_cap: false,
                abnormal_stop: None,
                halted_for_spend: None,
                budget_paused: None,
            })
        }
    }

    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-reentrant-")
        .tempdir()
        .expect("tempdir");
    let (base_url, _script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
            crate::workflows::gated_tool_turn_test::Turn::Say("{\"verdict\":\"recover\"}"),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let turn = Arc::new(ReentrantPeerTurn {
        consults: std::sync::atomic::AtomicUsize::new(0),
        node_turns: std::sync::atomic::AtomicUsize::new(0),
        runner: std::sync::OnceLock::new(),
    });
    let (runner, _blocks, _board, _notices, _artifacts) =
        peer_runner(turn.clone(), deps, "run-1866-peer-reentrant", None);
    let runner = Arc::new(runner);
    turn.runner.set(runner.clone()).ok().expect("wire runner");

    let _ = runner.run_turn("researcher", verify_node()).await;

    assert_eq!(
        turn.node_turns.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the nested node did run, so the re-entrancy this guards against was reached"
    );
    assert_eq!(
        turn.consults.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the peer rung must fire once down the whole stack, not once per nested node"
    );
}

/// The whole gate is opt-in: a node with no `verify` spends no judge call
/// and asks no peer, exactly as it did before any of this existed.
#[tokio::test]
async fn a_node_with_no_verify_calls_neither_judge_nor_peer() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-optin-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(Vec::new()).await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let turn = Arc::new(ConsultedPeerTurn::new("should never be reached"));
    let (runner, _blocks, _board, _notices, _artifacts) =
        peer_runner(turn.clone(), deps, "run-1866-peer-optin", None);

    runner
        .run_turn("researcher", json!({ "node_id": "plain", "prompt": "go" }))
        .await
        .expect("an unverified node runs as it always did");

    assert!(
        script.seen.lock().expect("seen").is_empty(),
        "no judge call for a node that declared no criteria"
    );
    assert_eq!(turn.consults(), 0, "and therefore no peer turn either");
}

/// `escalate` is a different arm from `recover` and must never touch the
/// ladder: an infrastructure or human gap is not something a teammate can
/// answer, so spending a peer turn on it would be pure cost. Held by
/// construction; pinned so a refactor that folded the arms together fails.
#[tokio::test]
async fn an_escalate_verdict_never_enters_the_recovery_ladder() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-escalate-")
        .tempdir()
        .expect("tempdir");
    let (base_url, script) =
        crate::workflows::gated_tool_turn_test::spawn_script_recording(vec![
            crate::workflows::gated_tool_turn_test::Turn::Say(
                "{\"verdict\":\"escalate\",\"gap\":\"infrastructure\"}",
            ),
        ])
        .await;
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(base_url, dir.path());
    let turn = Arc::new(ConsultedPeerTurn::new("should never be reached"));
    let (runner, _blocks, _board, _notices, _artifacts) =
        peer_runner(turn.clone(), deps, "run-1866-peer-escalate", None);

    let err = runner
        .run_turn("researcher", verify_node())
        .await
        .expect_err("an escalated gap halts the node");
    let EngineError::Capability(message) = err else {
        panic!("expected a capability error");
    };
    assert!(
        message.contains("after semantic verification"),
        "the escalate arm's own message, not the recovery arm's: {message}"
    );
    assert!(
        !message.contains("recovery tried"),
        "an escalated gap must not report a recovery ladder it never ran: {message}"
    );
    assert_eq!(turn.consults(), 0, "no peer turn on the escalate arm");
    assert_eq!(
        script.seen.lock().expect("seen").len(),
        1,
        "one judge call and no re-verification"
    );
}
