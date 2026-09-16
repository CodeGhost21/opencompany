use super::*;

/// An ACP agent's frames say which query they belong to, exactly as the
/// built-in harness's do.
///
/// `chat_ctx` used to take `chat_id` alone and hard-code `message_seq:
/// None`, so every ACP frame fell into the shared per-thread bucket — two
/// of its turns in one chat shared a single row-list, and arming the second
/// erased the first's timeline. That is the failure `messageSeq` exists to
/// stop, and it was left in place for one of the two harnesses (Codex on
/// #2069).
///
/// It now takes the whole `ChatTarget`, which is what keeps the two from
/// drifting again: there is no `chat_id`-only shape left to answer with.
#[test]
fn an_acp_chat_turn_carries_the_query_it_answers() {
    use crate::ports::types::EventSeq;

    let company = CompanyId::new("acp-msg-seq");

    let answering = AcpRunTurn::chat_ctx(
        &company,
        "product_manager",
        ChatTarget::channel(Some("general")).answering(Some(EventSeq::new(45))),
    );
    assert_eq!(answering.message_seq, Some(45));

    // And a turn answering no journaled message still says nothing, so the
    // consumer falls back to the thread exactly as it did before.
    let unaddressed = AcpRunTurn::chat_ctx(
        &company,
        "product_manager",
        ChatTarget::channel(Some("general")),
    );
    assert_eq!(unaddressed.message_seq, None);
}

fn turn(updates: Vec<AcpUpdate>) -> AcpTurn {
    AcpTurn {
        updates,
        stop_reason: "end_turn".to_string(),
    }
}

fn turn_with_stop_reason(updates: Vec<AcpUpdate>, stop_reason: &str) -> AcpTurn {
    AcpTurn {
        updates,
        stop_reason: stop_reason.to_string(),
    }
}

#[test]
fn a_max_turn_requests_stop_is_the_tool_step_cap() {
    // Issue #1853 established that a stop must not fold identically to a
    // clean end_turn — the operator needs a cap signal. PR #1880 review:
    // `max_turn_requests` is ACP's analog of openhuman's tool-iteration
    // cap, and is the only stop reason that may set `hit_iteration_cap`,
    // because `workflows/caps` reports that flag as "stopped at the
    // max_tool_iterations cap".
    let outcome = fold(turn_with_stop_reason(vec![], "max_turn_requests"));
    assert!(
        outcome.hit_iteration_cap,
        "max_turn_requests is the tool-step cap, not a clean finish"
    );
    assert!(
        !outcome.reply.trim().is_empty(),
        "a capped turn must say so, not fold to a blank reply"
    );
}

#[test]
fn a_max_tokens_stop_is_not_the_tool_step_cap() {
    // A token-generation budget on a single response is a different cap
    // than the tool-iteration one (PR #1880 review) — conflating them
    // would make a workflow node's `LimitStop{"max_tool_iterations"}`
    // misreport which cap actually stopped the turn.
    let outcome = fold(turn_with_stop_reason(vec![], "max_tokens"));
    assert!(
        !outcome.hit_iteration_cap,
        "a max_tokens stop is not the tool-iteration cap"
    );
    assert!(
        !outcome.reply.trim().is_empty(),
        "a capped turn must say so, not fold to a blank reply"
    );
}

#[test]
fn acp_results_are_reduced_to_shape_not_remote_text() {
    let secret = "API key: do-not-publish";
    let outcome = fold(turn(vec![
        AcpUpdate::ToolCall {
            id: "t".into(),
            title: "Read".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "t".into(),
            status: "completed".into(),
            result: Some(secret.into()),
        },
    ]));
    assert_eq!(outcome.steps[0].result.as_deref(), Some("23 characters"));
    assert!(!outcome.steps[0].result.as_deref().unwrap().contains(secret));
}
#[test]
fn a_tool_only_turn_gets_a_generic_reply_not_raw_tool_titles() {
    // No MessageChunk at all — the agent's entire turn was tool calls.
    // PR #1880 review: the reply must not copy the tools' raw ACP titles
    // — unlike the built-in harness's step label, a title comes straight
    // off the wire with no host-side bounding, and the timeline (already
    // carrying each ToolCall step's own title) is where that content
    // belongs, not a field meant to read as the agent's own words.
    let outcome = fold(turn(vec![
        AcpUpdate::ToolCall {
            id: "t1".into(),
            title: "Read".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "t1".into(),
            status: "completed".into(),
            result: Some("2.4 kB".into()),
        },
        AcpUpdate::ToolCall {
            id: "t2".into(),
            title: "Write".into(),
        },
    ]));
    assert_eq!(outcome.reply, "[no reply text — see steps]");
    assert_eq!(outcome.steps[0].label, "Read");
    assert_eq!(outcome.steps[1].label, "Write");
    // A clean end_turn needs no stop-reason note on top of the synthesis.
    assert!(!outcome.reply.contains("[stopped"));
}

#[test]
fn a_refusal_is_surfaced_as_a_note_step_and_the_cap_stays_false() {
    // The agent had prose to say, then declined to continue. The note
    // must land regardless — a refusal is not a clean finish even when
    // there is a reply to read. PR #1880 review: it lands as a `Note`
    // step, not appended onto the agent's own reply text.
    let outcome = fold(turn_with_stop_reason(
        vec![AcpUpdate::MessageChunk("I can't help with that.".into())],
        "refusal",
    ));
    assert_eq!(
        outcome.reply, "I can't help with that.",
        "the agent's own prose is kept verbatim, with nothing appended"
    );
    assert!(
        outcome.steps.iter().any(|s| s.kind == TurnStepKind::Note
            && s.label == "[stopped: the agent declined to continue]"),
        "the refusal must be surfaced as a step, not silently swallowed: {:?}",
        outcome.steps
    );
    assert!(
        !outcome.hit_iteration_cap,
        "a refusal is not an iteration-cap pause"
    );
    // PR #1880 review: `hit_iteration_cap == false` used to be the only
    // signal `HarnessAgentRunner` read, so a refusal settled a workflow
    // node `Succeeded`/`Finished` — indistinguishable from the agent
    // having actually answered. This is the outcome-level fix, not just
    // the note above: see `workflows::caps::mod::test::an_abnormal_acp_stop_fails_the_workflow_node`
    // for the assertion that it actually stops the graph.
    assert_eq!(
        outcome.abnormal_stop.as_deref(),
        Some("[stopped: the agent declined to continue]"),
        "a refusal must carry a distinct abnormal-stop outcome, not just a note"
    );
}

#[test]
fn a_cancelled_turn_also_carries_an_abnormal_stop() {
    // Same shape as refusal, different trigger: an operator-initiated (or
    // upstream) cancel is just as much "not a resumable cap, not a clean
    // finish" as a refusal is.
    let outcome = fold(turn_with_stop_reason(vec![], "cancelled"));
    assert_eq!(
        outcome.abnormal_stop.as_deref(),
        Some("[stopped: cancelled before finishing]")
    );
    assert!(!outcome.hit_iteration_cap);
}

#[test]
fn an_end_turn_reply_is_left_verbatim() {
    // The ordinary case — and the one the pre-existing seam test already
    // pins — must not gain a note or any other alteration just because
    // this fold now reads `stop_reason`.
    let outcome = fold(turn(vec![AcpUpdate::MessageChunk("all done".into())]));
    assert_eq!(outcome.reply, "all done");
    assert!(!outcome.hit_iteration_cap);
    assert_eq!(
        outcome.abnormal_stop, None,
        "a clean end_turn is not an abnormal stop"
    );
}

#[test]
fn a_max_turn_requests_stop_is_a_cap_not_an_abnormal_stop() {
    // The cap path (issue #926 / #1880's `hit_iteration_cap` split) and
    // the abnormal-stop path (this PR's review) are deliberately
    // disjoint: a capped turn has a real, resumable checkpoint, which is
    // exactly what `abnormal_stop` says there is none of.
    let outcome = fold(turn_with_stop_reason(vec![], "max_turn_requests"));
    assert!(outcome.hit_iteration_cap);
    assert_eq!(
        outcome.abnormal_stop, None,
        "the cap flag already covers this stop; abnormal_stop must stay None"
    );
}

#[test]
fn an_unrecognized_stop_reason_is_surfaced_not_swallowed() {
    // A stop_reason this fold has never heard of must not silently pass
    // for a clean end_turn — it is carried into a note step so the
    // operator (and whoever reads the ticket) can see the turn stopped
    // abnormally.
    //
    // PR #1880 review: the raw string itself must NOT appear — an
    // unrecognized `stopReason` is unvalidated, unbounded text straight
    // off the wire from an external ACP agent, and this note step is not
    // a private log line: `workflows/caps::transcript_from_steps` maps a
    // `Note` step to `"agent_message"` in the engine transcript, which
    // can be replayed as prior context for later engine reasoning. The
    // fixed notice below carries the abnormal-stop signal without
    // reopening that channel.
    let raw = "some_new_reason_acp_added_later__with_diagnostic_junk_🔥";
    let outcome = fold(turn_with_stop_reason(
        vec![AcpUpdate::MessageChunk("partial thought".into())],
        raw,
    ));
    assert_eq!(outcome.reply, "partial thought");
    assert!(
        outcome.steps.iter().any(|s| s.kind == TurnStepKind::Note
            && s.label == "[stopped: unrecognized stop reason]"),
        "an unrecognized stop must still be surfaced as a step: {:?}",
        outcome.steps
    );
    assert!(
        outcome.steps.iter().all(|s| !s.label.contains(raw)),
        "the raw wire value must never appear in a persisted step: {:?}",
        outcome.steps
    );
    assert!(!outcome.hit_iteration_cap);
    assert_eq!(
        outcome.abnormal_stop.as_deref(),
        Some("[stopped: unrecognized stop reason]"),
        "an unrecognized stop must carry a distinct abnormal-stop outcome, not just a note"
    );
    assert!(
        !outcome
            .abnormal_stop
            .as_deref()
            .unwrap_or_default()
            .contains(raw),
        "the raw wire value must never appear in the abnormal-stop message either"
    );
}

#[test]
fn classify_stop_reason_maps_the_known_shapes() {
    assert_eq!(classify_stop_reason("end_turn"), StopKind::EndTurn);
    assert_eq!(classify_stop_reason("max_tokens"), StopKind::MaxTokens);
    assert_eq!(
        classify_stop_reason("max_turn_requests"),
        StopKind::MaxTurnRequests
    );
    assert_eq!(classify_stop_reason("refusal"), StopKind::Refusal);
    assert_eq!(classify_stop_reason("cancelled"), StopKind::Cancelled);
    assert_eq!(classify_stop_reason("anything_else"), StopKind::Other);
    assert_eq!(classify_stop_reason(""), StopKind::Other);
}

#[test]
fn message_chunks_concatenate_in_order() {
    // ACP streams a reply in pieces; the outcome carries one string.
    let outcome = fold(turn(vec![
        AcpUpdate::MessageChunk("Hello".into()),
        AcpUpdate::MessageChunk(", ".into()),
        AcpUpdate::MessageChunk("world".into()),
    ]));
    assert_eq!(outcome.reply, "Hello, world");
    assert!(outcome.steps.is_empty(), "text alone produces no steps");
}

#[test]
fn a_run_of_thoughts_becomes_one_step() {
    // A model emits these by the hundred. One step per chunk would bury the
    // tool calls an operator is actually reading the timeline for.
    let outcome = fold(turn(vec![
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ThoughtChunk,
    ]));
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].kind, TurnStepKind::Thinking);
    assert_eq!(outcome.steps[0].label, "Thinking");
}

#[test]
fn thinking_resumes_as_a_new_step_after_a_tool_call() {
    // Two separate bouts of reasoning either side of a call are two steps —
    // coalescing them would put the thinking in the wrong order relative to
    // the work it bracketed.
    let outcome = fold(turn(vec![
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ToolCall {
            id: "t1".into(),
            title: "Read".into(),
        },
        AcpUpdate::ThoughtChunk,
    ]));
    let kinds: Vec<_> = outcome.steps.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TurnStepKind::Thinking,
            TurnStepKind::ToolCall,
            TurnStepKind::Thinking
        ]
    );
}

#[test]
fn a_tool_call_takes_its_final_status_and_result() {
    let outcome = fold(turn(vec![
        AcpUpdate::ToolCall {
            id: "t1".into(),
            title: "Read a file".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "t1".into(),
            status: "completed".into(),
            result: Some("2.4 kB".into()),
        },
    ]));
    assert_eq!(outcome.steps.len(), 1, "the update amends, never appends");
    assert_eq!(outcome.steps[0].label, "Read a file");
    assert_eq!(outcome.steps[0].status, TurnStepStatus::Ok);
    assert_eq!(outcome.steps[0].result.as_deref(), Some("6 characters"));
}

#[test]
fn a_failed_tool_call_is_an_error_step() {
    let outcome = fold(turn(vec![
        AcpUpdate::ToolCall {
            id: "t1".into(),
            title: "Write".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "t1".into(),
            status: "failed".into(),
            result: Some("permission denied".into()),
        },
    ]));
    assert_eq!(outcome.steps[0].status, TurnStepStatus::Error);
    assert!(outcome.steps[0].status.is_failure());
}

#[test]
fn a_tool_call_that_never_completes_stays_running() {
    // Exactly what `Running` means: started, no completion seen by the end
    // of the turn. Marking it `Ok` would report work that never finished as
    // having succeeded.
    let outcome = fold(turn(vec![AcpUpdate::ToolCall {
        id: "t1".into(),
        title: "Long thing".into(),
    }]));
    assert_eq!(outcome.steps[0].status, TurnStepStatus::Running);
}

#[test]
fn several_tool_calls_are_amended_independently() {
    // Interleaved calls are ordinary — an agent starts two and they finish
    // out of order. Each update has to find its own step.
    let outcome = fold(turn(vec![
        AcpUpdate::ToolCall {
            id: "a".into(),
            title: "First".into(),
        },
        AcpUpdate::ToolCall {
            id: "b".into(),
            title: "Second".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "b".into(),
            status: "completed".into(),
            result: None,
        },
        AcpUpdate::ToolCallUpdate {
            id: "a".into(),
            status: "failed".into(),
            result: None,
        },
    ]));
    assert_eq!(outcome.steps.len(), 2);
    assert_eq!(outcome.steps[0].label, "First");
    assert_eq!(outcome.steps[0].status, TurnStepStatus::Error);
    assert_eq!(outcome.steps[1].label, "Second");
    assert_eq!(outcome.steps[1].status, TurnStepStatus::Ok);
}

#[test]
fn an_update_for_an_unknown_call_is_dropped_rather_than_invented() {
    // A step with no label is worse on a timeline than no step at all.
    let outcome = fold(turn(vec![AcpUpdate::ToolCallUpdate {
        id: "ghost".into(),
        status: "completed".into(),
        result: Some("x".into()),
    }]));
    assert!(outcome.steps.is_empty());
}

/// An agent that answers from a script, so the trait impl can be driven.
///
/// `hang` makes `prompt` never resolve (the grace-expiry path) and
/// `cancel_fails` makes `cancel` error (the logged-failure path). `cancels`
/// counts cancel calls so a test can assert the grace path nudged twice.
///
/// `hold_for_cancel` makes `prompt` wait until the first `cancel` arrives —
/// the shape of a turn that is mid-tool-call when the operator steers, which
/// is exactly the window the advisory cancel exists for. Without the gate a
/// prompt that resolves immediately exits the loop before the steer check
/// ever runs, and the cancel path goes unexercised. `cancel_hangs` makes
/// `cancel` never answer (the bounded-RPC path).
struct Scripted {
    turn: AcpTurn,
    /// Milliseconds to hold the turn open before answering — how a test
    /// owns the session's slot for a *bounded* window, so a second turn
    /// genuinely queues and then genuinely gets in.
    holds_ms: u64,
    hang: bool,
    hold_for_cancel: bool,
    cancel_hangs: bool,
    cancel_fails: bool,
    cancels: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    cancel_started: tokio::sync::Notify,
}

impl Scripted {
    fn answering(updates: Vec<AcpUpdate>) -> Self {
        Self {
            turn: AcpTurn {
                updates,
                stop_reason: "end_turn".into(),
            },
            holds_ms: 0,
            hang: false,
            hold_for_cancel: false,
            cancel_hangs: false,
            cancel_fails: false,
            cancels: Default::default(),
            cancel_started: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl AcpAgent for Scripted {
    async fn prompt(
        &self,
        _c: &CompanyId,
        _k: &str,
        _m: &str,
        observer: Option<&AcpObserver>,
    ) -> Result<AcpTurn> {
        // Observed before the hang/hold gates, so a steer test still sees
        // the frames a real transport would have already published by the
        // time the operator reaches for cancel.
        if let Some(observer) = observer {
            for update in &self.turn.updates {
                observer(update);
            }
        }
        if self.holds_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.holds_ms)).await;
        }
        if self.hang {
            std::future::pending::<()>().await;
        }
        if self.hold_for_cancel {
            self.cancel_started.notified().await;
        }
        Ok(self.turn.clone())
    }
    async fn cancel(&self, _c: &CompanyId, _k: &str) -> Result<()> {
        self.cancels
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.cancel_started.notify_waiters();
        if self.cancel_hangs {
            std::future::pending::<()>().await;
        }
        if self.cancel_fails {
            return Err(OpenCompanyError::Harness("cancel rejected".into()));
        }
        Ok(())
    }
}

/// The claim the whole slice rests on: this is usable anywhere the
/// OpenHuman implementation is.
///
/// Driven through `&dyn RunTurn` rather than through the concrete type,
/// because that is how the company cycle holds it (`DelegationRunner` takes
/// `&'a dyn RunTurn`). A type that satisfied the trait but was not
/// object-safe would compile here and fail at the one site that matters.
#[tokio::test]
async fn it_is_usable_through_the_run_turn_seam() {
    let agent = Arc::new(Scripted::answering(vec![
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ToolCall {
            id: "t1".into(),
            title: "Read".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "t1".into(),
            status: "completed".into(),
            result: Some("4 items".into()),
        },
        AcpUpdate::MessageChunk("all done".into()),
    ]));
    let run_turn: &dyn RunTurn = &AcpRunTurn::new(agent);

    let outcome = run_turn
        .run(&CompanyId::new("acme"), "ceo", "go", ChatTarget::default())
        .await
        .expect("a turn runs");

    assert_eq!(outcome.reply, "all done");
    assert_eq!(outcome.steps.len(), 2);
    assert_eq!(outcome.steps[1].status, TurnStepStatus::Ok);
    assert_eq!(outcome.steps[1].result.as_deref(), Some("7 characters"));
}

#[tokio::test]
async fn a_steered_turn_still_returns_an_outcome() {
    // Cancellation in ACP is cooperative: the agent still answers, with
    // `stopReason: "cancelled"`. Abandoning the future on a steer would
    // leave a harness mid-tool-call with nothing reading its output, so the
    // contract is that a steered turn still produces an outcome.
    let agent = Arc::new(Scripted::answering(vec![AcpUpdate::MessageChunk(
        "partial".into(),
    )]));
    let run_turn: &dyn RunTurn = &AcpRunTurn::new(agent);
    let control = crate::company::steer::SteerControl::new();
    control.request(crate::company::steer::SteerAction::Cancel);

    let outcome = run_turn
        .run_steered(
            &CompanyId::new("acme"),
            "ceo",
            "go",
            &control,
            ChatTarget::default(),
            None,
        )
        .await
        .expect("a steered turn still answers");
    assert_eq!(outcome.reply, "partial");
    // The pending action survives for the disposition site to read, which
    // is what decides where the card lands.
    assert!(
        control.pending().is_some(),
        "the steer must not be consumed here"
    );
}

#[tokio::test]
async fn a_failed_cancel_is_logged_and_the_turn_still_drains() {
    // `session/cancel` can fail (the subprocess is mid-shutdown, say), but
    // that must not turn a cancelled turn into a failure of its own: the
    // cancel is advisory, the error is logged, and the turn still answers.
    // The prompt holds until the cancel arrives so the steer check is
    // actually reached — a prompt that resolves first would exit the loop
    // and leave the cancel path unexercised.
    let mut agent = Scripted::answering(vec![AcpUpdate::MessageChunk("done".into())]);
    agent.cancel_fails = true;
    agent.hold_for_cancel = true;
    let cancels = agent.cancels.clone();
    let agent = Arc::new(agent);
    let run_turn: &dyn RunTurn = &AcpRunTurn::new(agent);
    let control = crate::company::steer::SteerControl::new();
    control.request(crate::company::steer::SteerAction::Cancel);

    let outcome = run_turn
        .run_steered(
            &CompanyId::new("acme"),
            "ceo",
            "go",
            &control,
            ChatTarget::default(),
            None,
        )
        .await
        .expect("a failed cancel still ends in a turn");
    assert_eq!(outcome.reply, "done");
    assert_eq!(
        cancels.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the failed cancel was still attempted exactly once"
    );
}

#[tokio::test]
async fn a_hung_cancel_rpc_does_not_block_the_turn() {
    // A cancellation RPC that never answers — a wedged host, a dead
    // subprocess — must not pin the steered turn forever. Both cancel calls
    // are bounded, so the turn still settles on the grace schedule.
    let mut agent = Scripted::answering(vec![AcpUpdate::MessageChunk("done".into())]);
    agent.cancel_hangs = true;
    agent.hold_for_cancel = true;
    let agent = Arc::new(agent);
    let run_turn = AcpRunTurn::new(agent);
    let control = crate::company::steer::SteerControl::new();
    control.request(crate::company::steer::SteerAction::Cancel);

    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        run_turn.steered_with_grace(
            &CompanyId::new("acme"),
            "ceo",
            "go",
            &control,
            None,
            None,
            CancelBounds {
                grace: Duration::from_millis(20),
                rpc: Duration::from_millis(50),
            },
        ),
    )
    .await
    .expect("the turn settles despite a hung cancel RPC")
    .expect("the release of the prompt lets the turn answer");

    assert_eq!(outcome.reply, "done");
}

#[tokio::test]
async fn a_cancelled_turn_that_ignores_the_cancel_is_abandoned() {
    // A harness inside a tool call that never returns is the one case the
    // cooperative wait must not honour: past the grace window the waiter
    // drops the turn with an error, and nudges `cancel` once more on the
    // way out — the only drain lever the port exposes.
    let agent = Arc::new(Scripted {
        turn: AcpTurn {
            updates: vec![],
            stop_reason: "end_turn".into(),
        },
        holds_ms: 0,
        hang: true,
        hold_for_cancel: false,
        cancel_hangs: false,
        cancel_fails: false,
        cancels: Default::default(),
        cancel_started: tokio::sync::Notify::new(),
    });
    let cancels = agent.cancels.clone();
    let run_turn = AcpRunTurn::new(agent);
    let control = crate::company::steer::SteerControl::new();
    control.request(crate::company::steer::SteerAction::Cancel);

    let err = run_turn
        .steered_with_grace(
            &CompanyId::new("acme"),
            "ceo",
            "go",
            &control,
            None,
            None,
            CancelBounds {
                grace: Duration::from_millis(20),
                rpc: Duration::from_millis(50),
            },
        )
        .await
        .expect_err("a hung turn is abandoned, not awaited");
    assert!(
        format!("{err}").contains("abandoning the turn"),
        "the error names the abandonment: {err}"
    );
    assert_eq!(
        cancels.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "one cancel on the steer, one best-effort nudge on the way out"
    );
}

#[tokio::test]
async fn a_turn_cancelled_before_it_starts_never_reaches_the_agent() {
    // The sharp edge serialising turns introduced (PR #1904 review):
    // `session/cancel` names a *session*, not a turn. A queued turn that
    // forwarded its cancel would stop whichever turn currently owns the
    // session — an unrelated turn, still working.
    //
    // Driven by holding the slot with a turn that never finishes, so the
    // second turn is unambiguously still queued when it is cancelled.
    let agent = Arc::new(Scripted {
        turn: AcpTurn {
            updates: vec![],
            stop_reason: "end_turn".into(),
        },
        holds_ms: 0,
        hang: true,
        hold_for_cancel: false,
        cancel_hangs: false,
        cancel_fails: false,
        cancels: Default::default(),
        cancel_started: tokio::sync::Notify::new(),
    });
    let cancels = agent.cancels.clone();
    let run_turn = Arc::new(AcpRunTurn::new(agent));
    let company = CompanyId::new("acme");

    // The lock owner: hangs forever, holding the slot.
    let owner = {
        let run_turn = Arc::clone(&run_turn);
        let company = company.clone();
        tokio::spawn(async move {
            let control = crate::company::steer::SteerControl::new();
            run_turn
                .run_steered(
                    &company,
                    "ceo",
                    "first",
                    &control,
                    ChatTarget::default(),
                    None,
                )
                .await
        })
    };
    // Let it take the slot before the queued turn asks for it.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let queued = crate::company::steer::SteerControl::new();
    queued.request(crate::company::steer::SteerAction::Cancel);
    let err = run_turn
        .run_steered(
            &company,
            "ceo",
            "second",
            &queued,
            ChatTarget::default(),
            None,
        )
        .await
        .expect_err("a turn cancelled while queued does not run");

    assert!(
        format!("{err}").contains("cancelled before it started"),
        "the error says it never started: {err}"
    );
    assert_eq!(
        cancels.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "and no cancel reached the agent, which would have stopped the OTHER turn"
    );

    owner.abort();
}

#[tokio::test]
async fn a_cancel_landing_as_the_slot_frees_still_stops_the_turn() {
    // The race the 250ms poll cannot win alone (PR #1904 review): the
    // cancel arrives while this turn is queued, and the slot frees BEFORE
    // the next tick — so `lock_owned()` wins the select and the control is
    // never consulted. Without the check on acquiring, a cancelled turn
    // would reach the agent.
    //
    // The owner holds for 50ms against a 250ms poll, so the lock branch
    // wins deterministically.
    let mut owner_agent = Scripted::answering(vec![AcpUpdate::MessageChunk("first".into())]);
    owner_agent.holds_ms = 50;
    let agent = Arc::new(owner_agent);
    let cancels = agent.cancels.clone();
    let run_turn = Arc::new(AcpRunTurn::new(agent));
    let company = CompanyId::new("acme");

    let owner = {
        let run_turn = Arc::clone(&run_turn);
        let company = company.clone();
        tokio::spawn(async move {
            let control = crate::company::steer::SteerControl::new();
            run_turn
                .run_steered(
                    &company,
                    "ceo",
                    "first",
                    &control,
                    ChatTarget::default(),
                    None,
                )
                .await
        })
    };
    // Long enough that the owner holds the slot, short enough that it is
    // still holding it when the queued turn asks.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let queued = crate::company::steer::SteerControl::new();
    queued.request(crate::company::steer::SteerAction::Cancel);
    let err = run_turn
        .run_steered(
            &company,
            "ceo",
            "second",
            &queued,
            ChatTarget::default(),
            None,
        )
        .await
        .expect_err("a turn cancelled while queued does not run");

    assert!(
        format!("{err}").contains("cancelled before it started"),
        "the error says it never started: {err}"
    );
    assert_eq!(
        cancels.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "and no cancel reached the agent, which would have stopped the OTHER turn"
    );
    owner.await.expect("owner joins").expect("owner answers");
}

#[tokio::test]
async fn a_pending_cancel_on_a_free_slot_still_runs_and_is_forwarded() {
    // The other side of that boundary, and the reason the refusal is
    // scoped to queued turns only. With no other turn on the session there
    // is nothing a forwarded cancel could stop by mistake, so a pending
    // cancel keeps its long-standing meaning: the turn runs, the cancel
    // goes to the agent, and the agent winds down and reports — an `Ok`
    // outcome the caller settles as cancelled rather than failed.
    let mut agent = Scripted::answering(vec![AcpUpdate::MessageChunk("done".into())]);
    agent.hold_for_cancel = true;
    let agent = Arc::new(agent);
    let cancels = agent.cancels.clone();
    let run_turn = AcpRunTurn::new(agent);

    let control = crate::company::steer::SteerControl::new();
    control.request(crate::company::steer::SteerAction::Cancel);

    let outcome = run_turn
        .run_steered(
            &CompanyId::new("acme"),
            "ceo",
            "go",
            &control,
            ChatTarget::default(),
            None,
        )
        .await
        .expect("an uncontended turn still runs and returns its outcome");

    assert_eq!(outcome.reply, "done");
    assert_eq!(
        cancels.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the cancel was forwarded, because this turn was the one running"
    );
}

/// A runner with no desks declared — the shape every key assertion below
/// except the alias one is about.
fn keyer() -> AcpRunTurn {
    AcpRunTurn::new(Arc::new(Scripted::answering(a_working_turn())))
}

#[test]
fn a_session_key_separates_agents_and_companies() {
    let keyer = keyer();
    let acme = CompanyId::new("acme");
    let globex = CompanyId::new("globex");
    assert_ne!(
        keyer.session_key(&acme, "ceo", None),
        keyer.session_key(&acme, "cto", None)
    );
    assert_ne!(
        keyer.session_key(&acme, "ceo", None),
        keyer.session_key(&globex, "ceo", None)
    );
    // Stable across turns, or the second question in a thread arrives with
    // no memory of the first.
    assert_eq!(
        keyer.session_key(&acme, "ceo", None),
        keyer.session_key(&acme, "ceo", None)
    );
}

/// Issue #1890 H — the property this file's comments already claimed.
///
/// "Two desks do not share a conversation" was written on `session_key`
/// while the key held no channel at all, so every desk, DM and thread an
/// ACP teammate answered in was one durable conversation inside another
/// program.
#[test]
fn a_session_key_separates_conversations() {
    let keyer = keyer();
    let acme = CompanyId::new("acme");
    assert_ne!(
        keyer.session_key(&acme, "ceo", Some("engineering")),
        keyer.session_key(&acme, "ceo", Some("growth")),
        "two desks must not share one session"
    );
    assert_ne!(
        keyer.session_key(&acme, "ceo", Some("engineering")),
        keyer.session_key(&acme, "ceo", Some("dm:designer")),
        "nor a desk and a DM"
    );
    // Stable within a conversation, for the same reason it is stable across
    // turns at all.
    assert_eq!(
        keyer.session_key(&acme, "ceo", Some("engineering")),
        keyer.session_key(&acme, "ceo", Some("engineering"))
    );
}

/// The General desk is **one** conversation however it is spelled, folded
/// through the same rule every other reader of a chat id uses — otherwise
/// its four spellings would mint four sessions in an external process, and
/// an operator's own main line would forget itself depending on which id
/// the caller happened to address.
#[test]
fn every_spelling_of_the_general_desk_is_one_session() {
    let keyer = keyer();
    let acme = CompanyId::new("acme");
    let unaddressed = keyer.session_key(&acme, "ceo", None);
    for spelling in ["", "main", "General", "general", "MAIN"] {
        assert_eq!(
            keyer.session_key(&acme, "ceo", Some(spelling)),
            unaddressed,
            "{spelling:?} is the General desk"
        );
    }
}

/// A named desk's id and its display name are one session.
///
/// The key was `(company, agent)` before #1890 H, where no selector could
/// disagree with itself. Adding the chat introduced the possibility that a
/// client addressing one desk by id and another by name mints two sessions
/// in the external agent — and an ACP session is durable state in another
/// process with no lifecycle across the port, so the earlier one is not
/// merely re-created, its context is gone (codex on #1972).
#[test]
fn a_named_desks_two_spellings_are_one_session() {
    let keyer = keyer().with_desks(vec![("growth_desk".to_string(), "Growth".to_string())]);
    let acme = CompanyId::new("acme");
    assert_eq!(
        keyer.session_key(&acme, "ceo", Some("growth_desk")),
        keyer.session_key(&acme, "ceo", Some("Growth")),
        "one desk, one session, whichever spelling addressed it"
    );
    // …and an undeclared selector still keys on itself, which is what a
    // DM and an ad-hoc thread rely on.
    assert_ne!(
        keyer.session_key(&acme, "ceo", Some("growth_desk")),
        keyer.session_key(&acme, "ceo", Some("dm:designer"))
    );
}

/// The slot that serialises turns is **not** the session key.
///
/// Widening it alongside the session would let one teammate's desks prompt
/// an external process concurrently — a change to how that process is
/// driven, made as a side effect of a change about conversation scope.
#[test]
fn the_turn_slot_stays_per_teammate() {
    let keyer = keyer();
    let acme = CompanyId::new("acme");
    assert_eq!(
        AcpRunTurn::lock_key(&acme, "ceo"),
        AcpRunTurn::lock_key(&acme, "ceo"),
    );
    assert_ne!(
        AcpRunTurn::lock_key(&acme, "ceo"),
        AcpRunTurn::lock_key(&acme, "cto"),
    );
    // The point: two conversations of one teammate share a slot while
    // holding different sessions.
    assert_ne!(
        keyer.session_key(&acme, "ceo", Some("engineering")),
        keyer.session_key(&acme, "ceo", Some("growth")),
    );
}
/// Drains the live frames a turn published, giving up once the bus goes
/// quiet — a turn that published nothing must be provable, not merely
/// unobserved, so this returns an empty vec rather than hanging.
async fn drain_live(
    stream: &mut futures::stream::BoxStream<'static, crate::turn_stream::LiveFrame>,
) -> Vec<crate::turn_stream::TurnStreamEvent> {
    use futures::StreamExt;
    let mut frames = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(Duration::from_millis(50), stream.next()).await
    {
        if let Some(event) = frame.as_turn() {
            frames.push(event.clone());
        }
    }
    frames
}

/// The updates a coding turn produces: a thought, a tool call that runs
/// and then completes, and the answer.
fn a_working_turn() -> Vec<AcpUpdate> {
    vec![
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ThoughtChunk,
        AcpUpdate::ToolCall {
            id: "c1".into(),
            title: "Read src/main.rs".into(),
        },
        AcpUpdate::ToolCallUpdate {
            id: "c1".into(),
            status: "in_progress".into(),
            result: None,
        },
        AcpUpdate::ToolCallUpdate {
            id: "c1".into(),
            status: "completed".into(),
            result: Some("42 lines".into()),
        },
        AcpUpdate::MessageChunk("done".into()),
    ]
}

#[tokio::test]
async fn a_chat_turn_streams_its_execution_state_onto_the_watching_thread() {
    // The gap this closes: an ACP turn used to be observable only once it
    // was over. On a five-minute coding turn that is indistinguishable
    // from a hang, while a `built_in` teammate beside it shows every tool
    // call as it starts.
    let company = CompanyId::new("acme-live-chat");
    let mut bus = crate::turn_stream::subscribe(&company);

    let run_turn = AcpRunTurn::new(Arc::new(Scripted::answering(a_working_turn())));
    let outcome = run_turn
        .run(&company, "ceo", "go", ChatTarget::channel(Some("design")))
        .await
        .expect("the turn answers");

    let frames = drain_live(&mut bus).await;
    let kinds: Vec<&str> = frames.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec!["thinking", "tool_call", "tool_result"],
        "one coalesced thinking row, the call, and its completion"
    );

    // Routed to the thread that asked, and labelled with the desk that
    // answered — a frame on the wrong thread is worse than no frame.
    assert!(frames.iter().all(
        |f| f.chat_id.as_deref() == Some("design") && f.agent_id.as_deref() == Some("ceo")
    ));
    // Ordered and dedupable by the console.
    assert_eq!(
        frames.iter().map(|f| f.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let call = &frames[1];
    assert_eq!(call.tool_call_id.as_deref(), Some("c1"));
    assert_eq!(call.label.as_deref(), Some("Read src/main.rs"));
    assert_eq!(call.status, Some("running"));

    let result = &frames[2];
    assert_eq!(
        result.tool_call_id.as_deref(),
        Some("c1"),
        "the completion pairs back to its row"
    );
    assert_eq!(result.status, Some("ok"));
    assert_eq!(result.result.as_deref(), Some("8 characters"));

    // And the live view did not replace the durable one.
    assert_eq!(outcome.reply, "done");
    assert_eq!(
        outcome
            .steps
            .iter()
            .filter(|s| s.kind == TurnStepKind::ToolCall)
            .count(),
        1,
        "the same updates still fold into the timeline that rides the reply"
    );
}

#[tokio::test]
async fn an_unaddressed_chat_turn_streams_onto_the_default_desk() {
    // Where the durable reply lands is where the live rows must land: an
    // API client that omits `chat` still gets a coherent timeline.
    let company = CompanyId::new("acme-live-default");
    let mut bus = crate::turn_stream::subscribe(&company);

    let run_turn = AcpRunTurn::new(Arc::new(Scripted::answering(vec![AcpUpdate::ToolCall {
        id: "c1".into(),
        title: "Search".into(),
    }])));
    run_turn
        .run(&company, "ceo", "go", ChatTarget::default())
        .await
        .expect("the turn answers");

    let frames = drain_live(&mut bus).await;
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].chat_id.as_deref(),
        Some(crate::server::ops::language::DEFAULT_DESK)
    );
}

#[tokio::test]
async fn a_dispatched_card_turn_streams_nothing_onto_the_console() {
    // A dispatched card shows no chat bubble and its steps are folded into
    // the card's own note. Streaming them would put rows on whatever
    // thread most recently sent — the misattribution `LiveStream::Off`
    // exists to prevent.
    let company = CompanyId::new("acme-live-card");
    let mut bus = crate::turn_stream::subscribe(&company);

    let run_turn = AcpRunTurn::new(Arc::new(Scripted::answering(a_working_turn())));
    let control = crate::company::steer::SteerControl::new();
    run_turn
        .run_steered_background(&company, "ceo", "go", &control, ChatTarget::default(), None)
        .await
        .expect("the turn answers");

    assert!(
        drain_live(&mut bus).await.is_empty(),
        "a background turn publishes nothing"
    );
}

#[tokio::test]
async fn a_workflow_node_streams_onto_its_run_rather_than_a_desk() {
    // The trait default for `run_background_workflow` forwards to `run`
    // with no chat id — which, now that `run` streams, would publish a
    // node's tool calls onto the DEFAULT DESK. This asserts the override
    // that keeps them on the run-trace sheet instead.
    let company = CompanyId::new("acme-live-workflow");
    let mut bus = crate::turn_stream::subscribe(&company);

    let run_turn = AcpRunTurn::new(Arc::new(Scripted::answering(vec![AcpUpdate::ToolCall {
        id: "c1".into(),
        title: "Fetch".into(),
    }])));
    run_turn
        .run_background_workflow(&company, "ceo", "go", None, "run-7", "node-2")
        .await
        .expect("the turn answers");

    let frames = drain_live(&mut bus).await;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].workflow_run_id.as_deref(), Some("run-7"));
    assert_eq!(frames[0].node_id.as_deref(), Some("node-2"));
    assert!(
        frames[0].chat_id.is_none(),
        "a node has no chat thread to attribute to"
    );
}

/// How many rows a console holding these frames ends up showing.
///
/// A tool call is two frames and one row: `tool_call` opens it and
/// `tool_result` flips that same row in place, paired by `toolCallId`
/// (`app-shell.tsx`'s `onTurnEvent`). Counting frames instead of rows
/// would make the live view look like it shows twice the work.
fn rendered_rows(frames: &[TurnStreamEvent]) -> usize {
    let paired: std::collections::HashSet<&str> = frames
        .iter()
        .filter_map(|f| f.tool_call_id.as_deref())
        .collect();
    paired.len() + frames.iter().filter(|f| f.tool_call_id.is_none()).count()
}

#[test]
fn the_live_rows_and_the_folded_steps_stay_in_step() {
    // The two views are the same updates read twice, and the property that
    // matters is that neither invents or drops a row the other has. A
    // non-terminal `tool_call_update` is the one that could: it leaves the
    // folded step `Running` and must publish no second row.
    let updates = a_working_turn();
    let mut state = LiveState::default();
    let frames: Vec<_> = updates
        .iter()
        .filter_map(|u| live_frame_from(u, &mut state))
        .collect();

    let outcome = fold(turn(updates));
    assert_eq!(
        rendered_rows(&frames),
        outcome.steps.len(),
        "one live row per folded step: {frames:?} vs {:?}",
        outcome.steps
    );
    assert_eq!(
        frames.iter().filter(|f| f.kind == "tool_call").count(),
        outcome
            .steps
            .iter()
            .filter(|s| s.kind == TurnStepKind::ToolCall)
            .count()
    );
}

#[tokio::test]
async fn a_completion_for_a_call_nobody_saw_start_publishes_no_row() {
    // `fold` drops these ("a step with no label is worse on a timeline
    // than no step"), so the live view must too — a row that appears while
    // the turn runs and is missing from the finished timeline reads as
    // work that was undone.
    let company = CompanyId::new("acme-live-ghost");
    let mut bus = crate::turn_stream::subscribe(&company);

    let run_turn = AcpRunTurn::new(Arc::new(Scripted::answering(vec![
        AcpUpdate::ToolCallUpdate {
            id: "ghost".into(),
            status: "completed".into(),
            result: Some("x".into()),
        },
    ])));
    let outcome = run_turn
        .run(&company, "ceo", "go", ChatTarget::channel(Some("design")))
        .await
        .expect("the turn answers");

    assert!(drain_live(&mut bus).await.is_empty());
    assert!(
        outcome
            .steps
            .iter()
            .all(|s| s.kind != TurnStepKind::ToolCall)
    );
}

#[test]
fn thinking_around_assistant_text_folds_and_streams_the_same_way() {
    // The divergence PR #1904's review caught: the live mapper closed a
    // thinking run on assistant text and `fold` did not, so this sequence
    // streamed two `Thinking` rows and folded one — the second row
    // vanishing the moment the reply replaced the live timeline.
    let updates = vec![
        AcpUpdate::ThoughtChunk,
        AcpUpdate::MessageChunk("partly there. ".into()),
        AcpUpdate::ThoughtChunk,
        AcpUpdate::MessageChunk("done".into()),
    ];

    let mut state = LiveState::default();
    let live = updates
        .iter()
        .filter_map(|u| live_frame_from(u, &mut state))
        .count();

    let outcome = fold(turn(updates));
    let folded = outcome
        .steps
        .iter()
        .filter(|s| s.kind == TurnStepKind::Thinking)
        .count();

    assert_eq!(folded, 2, "text closes a thinking run, so this is two");
    assert_eq!(live, folded, "and the live view says the same");
    assert_eq!(outcome.reply, "partly there. done");
}

#[test]
fn a_burst_of_thoughts_is_one_row_until_something_else_happens() {
    // A model emits these by the hundred; a timeline of them is noise.
    // Mirrors `fold`'s own coalescing so the live view does not show a
    // different number of thinking rows than the finished one.
    let mut state = LiveState::default();
    let thoughts = vec![AcpUpdate::ThoughtChunk; 5];
    let frames: Vec<_> = thoughts
        .iter()
        .filter_map(|u| live_frame_from(u, &mut state))
        .collect();
    assert_eq!(frames.len(), 1);

    // Text closes the run, so the next thought opens a new row — exactly
    // what `fold` does with its own `thinking` flag.
    assert!(live_frame_from(&AcpUpdate::MessageChunk("hi".into()), &mut state).is_none());
    assert!(live_frame_from(&AcpUpdate::ThoughtChunk, &mut state).is_some());
}
