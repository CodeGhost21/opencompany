use async_trait::async_trait;

use super::*;

#[test]
fn prompt_is_assembled_from_the_verdict_and_gap_tokens() {
    let prompt = system_prompt();
    for token in SufficiencyVerdict::tokens() {
        assert!(prompt.contains(token), "missing verdict token {token}");
    }
    for gap in [
        BlockerKind::Information,
        BlockerKind::Infrastructure,
        BlockerKind::Transient,
    ] {
        assert!(prompt.contains(gap.as_str()), "missing gap token");
    }
}

#[test]
fn text_outside_the_json_object_is_rejected() {
    let wrapped = "Sure, here you go: {\"verdict\":\"halt_benign\"} — hope that helps!";
    assert_eq!(parse_verdict(wrapped), None);
}

#[test]
fn parses_each_closed_verdict() {
    for expected in [
        SufficiencyVerdict::Continue,
        SufficiencyVerdict::Retry,
        SufficiencyVerdict::Recover,
        SufficiencyVerdict::Escalate {
            gap: BlockerKind::Infrastructure,
        },
        SufficiencyVerdict::HaltBenign,
    ] {
        let gap = match expected {
            SufficiencyVerdict::Escalate { gap } => {
                format!(",\"gap\":\"{}\"", gap.as_str())
            }
            _ => String::new(),
        };
        let answer = format!("{{\"verdict\":\"{}\"{gap}}}", expected.token());
        assert_eq!(parse_verdict(&answer), Some(expected));
    }
}

#[test]
fn insufficient_or_failed_output_is_never_halt_benign() {
    for input in [
        JudgeInput {
            instruction: "write it",
            output: "provider failed",
            criteria: None,
            execution_failed: true,
        },
        JudgeInput {
            instruction: "write it",
            output: "   ",
            criteria: None,
            execution_failed: false,
        },
    ] {
        assert_eq!(
            enforce_anti_suppression(SufficiencyVerdict::HaltBenign, input),
            SufficiencyVerdict::Retry
        );
    }
}

#[test]
fn the_run_request_survives_an_oversized_instruction() {
    let standing = "S".repeat(INSTRUCTION_CAP * 2);
    let instruction = format!("{standing}\n\nRequest for this run:\nship dark mode for iOS");
    let prompt = user_prompt(JudgeInput {
        instruction: &instruction,
        output: "done",
        criteria: None,
        execution_failed: false,
    });
    assert!(
        prompt.contains("ship dark mode for iOS"),
        "the run request must survive the instruction cap"
    );
    assert!(prompt.contains("Request for this run:"));
}

#[test]
fn recovered_evidence_stays_inside_the_judge_output_window() {
    let reply = "R".repeat(OUTPUT_CAP + 5_000);
    let evidence = "the renewal date is 2026-11-04";
    let verified = augment_with_recovery(&reply, evidence);
    let prompt = user_prompt(JudgeInput {
        instruction: "draft it",
        output: &verified,
        criteria: None,
        execution_failed: false,
    });
    assert!(
        prompt.contains(evidence),
        "recovered evidence must reach the judge, not fall past the output cap"
    );
    assert!(verified.chars().count() <= OUTPUT_CAP);
}

#[test]
fn recovery_augmentation_leaves_a_short_reply_whole() {
    let verified = augment_with_recovery("the draft", "a recovered fact");
    assert!(verified.starts_with("the draft"));
    assert!(verified.ends_with("a recovered fact"));
}

#[test]
fn recovery_material_is_bounded_on_unicode_boundaries() {
    let text = "🧠".repeat(MAX_RECOVERY_CHARS + 1);
    let capped = cap(&text, MAX_RECOVERY_CHARS);
    assert_eq!(capped.chars().count(), MAX_RECOVERY_CHARS + 1);
    assert!(capped.ends_with('…'));
}

/// Codex review on #1990 (#3903591432): a whole success-criteria sentence
/// pulls out its content words as focused single-term queries, dropping
/// short/filler words, so a fact titled on ONE of those words is
/// reachable even though it never contained the sentence verbatim.
#[test]
fn focus_terms_extracts_content_words_from_a_criteria_sentence() {
    let terms = focus_terms("The answer must include the renewal date");
    assert!(
        terms.contains(&"renewal".to_string()),
        "expected \"renewal\" among {terms:?}"
    );
    assert!(
        terms.contains(&"date".to_string()),
        "expected \"date\" among {terms:?}"
    );
    assert!(
        !terms.contains(&"the".to_string()) && !terms.contains(&"must".to_string()),
        "filler words must be dropped: {terms:?}"
    );
}

#[test]
fn focus_terms_is_capped_and_deduplicated() {
    let terms =
        focus_terms("alpha alpha bravo charlie delta echo foxtrot golf hotel india juliet");
    assert!(terms.len() <= MAX_FOCUS_TERMS, "{terms:?}");
    let unique: std::collections::HashSet<_> = terms.iter().collect();
    assert_eq!(unique.len(), terms.len(), "no duplicate terms: {terms:?}");
}

/// A [`crate::ports::FactStore`] whose `list` only matches an EXACT query
/// string — standing in for the real substring-match store closely enough
/// to prove whether a whole-sentence query alone can ever reach a fact
/// keyed on one of its content words.
struct ExactMatchFactStore {
    matches: &'static str,
    fact: crate::ports::FactRecord,
}

#[async_trait::async_trait]
impl crate::ports::FactStore for ExactMatchFactStore {
    async fn list(
        &self,
        _company: &CompanyId,
        query: Option<&str>,
        _kind: Option<crate::ports::FactKind>,
    ) -> crate::Result<Vec<crate::ports::FactRecord>> {
        Ok(match query {
            Some(q) if q == self.matches => vec![self.fact.clone()],
            _ => Vec::new(),
        })
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

/// Codex review on #1990 (#3903591432): the whole-sentence query used to
/// be the ONLY query `ask_around` ever sent to the fact store. A fact
/// titled "Renewal date" — reachable by the focused term "renewal" — was
/// unreachable by the full sentence "The answer must include the renewal
/// date" against a case-insensitive substring match.
#[tokio::test]
async fn ask_around_finds_a_fact_reachable_only_by_a_focused_term() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-focused-query-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    deps.facts = Some(std::sync::Arc::new(ExactMatchFactStore {
        matches: "renewal",
        fact: crate::ports::FactRecord {
            id: "f1".to_string(),
            kind: crate::ports::FactKind::Fact,
            title: "Renewal date".to_string(),
            body: "The contract renews on March 1st.".to_string(),
            source: "test".to_string(),
            updated_at_millis: 0,
        },
    }));

    let result = ask_around(
        &deps,
        &CompanyId::new("acme"),
        "The answer must include the renewal date",
        None,
    )
    .await;

    let evidence = result
        .evidence
        .expect("a fact reachable only by a focused term must still be found");
    assert!(
        evidence.contains("Renewal date"),
        "expected the matched fact in the evidence: {evidence}"
    );
}

/// A meter that always reports enough spend to exhaust any total ceiling,
/// regardless of `since_millis` — standing in for a company already past
/// its plan-level token cap.
struct ExhaustedMeter;

#[async_trait]
impl crate::ports::UsageMeter for ExhaustedMeter {
    async fn record(
        &self,
        _company: &CompanyId,
        _sample: &crate::ports::UsageSample,
    ) -> crate::Result<()> {
        Ok(())
    }

    async fn query(
        &self,
        _company: &CompanyId,
        _since_millis: u64,
    ) -> crate::Result<Vec<crate::ports::UsageSample>> {
        Ok(vec![crate::ports::UsageSample {
            at_millis: 0,
            agent: "ceo".to_string(),
            provider: "managed".to_string(),
            input_tokens: 10_000,
            output_tokens: 0,
            cached_input_tokens: 0,
            cost_usd: 0.0,
            kind: crate::ports::SampleKind::Inference,
            run_id: None,
            model: None,
        }])
    }
}

/// A provider that counts every `invoke` rather than ever answering one —
/// the judge must never reach it once the total ceiling is spent.
#[derive(Default)]
struct PanicIfInvokedProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl tinyinference::model::ChatModel<()> for PanicIfInvokedProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ModelResponse::assistant(
            "{\"verdict\":\"continue\"}".to_string(),
        ))
    }
}

impl crate::harness::provider::HarnessModel for PanicIfInvokedProvider {
    fn telemetry_provider_id(&self) -> String {
        "panic-if-invoked".to_string()
    }
}

/// Codex review on #1990 (#3904894255): `HarnessPool::run_inner` refuses
/// dispatch at the plan-level total token ceiling before ever invoking the
/// agent model, but `judge_sufficiency` called `deps.provider.invoke`
/// unconditionally — a `verify`-declared node whose company had already
/// exhausted its ceiling still paid for a judge turn, and because that
/// refusal is not `execution_failed`, retries could repeat the spend.
/// Exhausting the ceiling and asking the judge to verdict must now cost
/// zero inference calls.
#[tokio::test]
async fn a_spent_total_ceiling_skips_the_judge_call_entirely() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1990-judge-ceiling-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let provider = std::sync::Arc::new(PanicIfInvokedProvider::default());
    deps.provider = provider.clone();
    deps.plan = Some(crate::harness::capability_budget::CapabilityPlan {
        period: crate::harness::capability_budget::BudgetPeriod::Daily,
        budgets: Default::default(),
        total_budget: Some(1_000),
    });
    deps.meter = Some(std::sync::Arc::new(ExhaustedMeter));

    let verdict = judge_sufficiency(
        &deps,
        &CompanyId::new("acme"),
        JudgeInput {
            instruction: "send the report",
            output: "the report has been sent",
            criteria: None,
            execution_failed: false,
        },
    )
    .await;

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the judge must not spend inference once the total ceiling is spent"
    );
    assert_eq!(
        verdict,
        SufficiencyVerdict::Retry,
        "a budget-refused verify must not be accepted as sufficient"
    );
}

// ── the recovery ladder's peer rung ──────────────────────────────────────

/// A roster built from `[[agent]]` blocks, on the fixture record so every
/// other field keeps the shape the rest of these tests use.
fn roster(agents: &str) -> CompanyRecord {
    let mut record = crate::workflows::gated_tool_turn_test::record();
    record.manifest =
        toml::from_str(&format!("[company]\nname = \"Acme\"\n\n{agents}\n")).expect("manifest");
    record
}

/// `support` is listed FIRST and scores lower than `cfo`, so a scoring
/// regression that fell back to "the first roster entry that matches at all"
/// is visible rather than accidentally right.
const THREE_DESKS: &str = r#"
[[agent]]
id = "support"
role = "Support Lead"
description = "Handles renewal reminders for customers"

[[agent]]
id = "cfo"
role = "Chief Financial Officer"
description = "Owns pricing and renewal contracts"

[[agent]]
id = "designer"
role = "Brand Designer"
description = "Owns the visual identity"
"#;

#[test]
fn pick_peer_picks_the_role_that_shares_most_terms_with_the_question() {
    let record = roster(THREE_DESKS);
    let peer = pick_peer(
        &record,
        "The answer must include the renewal pricing",
        "researcher",
    )
    .expect("a roster role shares terms with this question");
    assert_eq!(
        peer.id, "cfo",
        "the highest-scoring role must be chosen, not the first that matches at all"
    );
}

#[test]
fn pick_peer_never_picks_the_nodes_own_agent() {
    let record = roster(THREE_DESKS);
    let peer = pick_peer(
        &record,
        "The answer must include the renewal pricing",
        "cfo",
    )
    .expect("another role still matches once the node's own agent is excluded");
    assert_eq!(
        peer.id, "support",
        "the node's own agent must be excluded even when it scores highest"
    );
}

#[test]
fn pick_peer_keeps_roster_order_on_a_tie() {
    let record = roster(
        r#"
[[agent]]
id = "first"
role = "Renewal Desk"

[[agent]]
id = "second"
role = "Renewal Team"
"#,
    );
    for _ in 0..8 {
        let peer = pick_peer(&record, "the renewal date", "researcher").expect("a tie matches");
        assert_eq!(
            peer.id, "first",
            "a tie must resolve to roster order, identically on every call"
        );
    }
}

#[test]
fn pick_peer_returns_none_when_no_role_matches() {
    let record = roster(THREE_DESKS);
    assert!(
        pick_peer(&record, "the warehouse forklift inspection", "researcher").is_none(),
        "a question no role shares a term with must pick nobody rather than pick arbitrarily"
    );
}

#[test]
fn pick_peer_ignores_retired_agents() {
    let mut record = roster(THREE_DESKS);
    record.overlay_retired_agents = vec!["cfo".to_string()];
    let peer = pick_peer(
        &record,
        "The answer must include the renewal pricing",
        "researcher",
    )
    .expect("a live role still matches");
    assert_eq!(
        peer.id, "support",
        "a retired teammate must never be consulted"
    );
}

/// A [`FactStore`] that records every query and matches nothing, so a test
/// can read exactly which queries the fact rung sent.
#[derive(Default)]
struct RecordingFactStore {
    queries: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl crate::ports::FactStore for RecordingFactStore {
    async fn list(
        &self,
        _company: &CompanyId,
        query: Option<&str>,
        _kind: Option<crate::ports::FactKind>,
    ) -> crate::Result<Vec<crate::ports::FactRecord>> {
        self.queries
            .lock()
            .expect("queries")
            .push(query.unwrap_or_default().to_string());
        Ok(Vec::new())
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

/// The two rungs must read the question the same way. If the fact rung's
/// term extraction and the peer rung's scoring vocabulary ever drift apart,
/// a question could reach a fact the peer scoring cannot see (or the
/// reverse) and the ladder would disagree with itself about its own
/// subject. Both sides are asserted against the same `focus_terms` call.
#[tokio::test]
async fn the_fact_rung_and_the_peer_rung_read_the_question_the_same_way() {
    let question = "The answer must include the renewal pricing for the enterprise account";
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-term-parity-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    let facts = std::sync::Arc::new(RecordingFactStore::default());
    deps.facts = Some(facts.clone());

    let _ = ask_around(&deps, &CompanyId::new("acme"), question, None).await;

    let sent = facts.queries.lock().expect("queries").clone();
    let terms = focus_terms(question);
    assert_eq!(
        sent.first().map(String::as_str),
        Some(question),
        "the fact rung still asks the whole sentence first"
    );
    assert_eq!(
        sent[1..],
        terms[..],
        "the fact rung's focused queries must be exactly `focus_terms`"
    );

    for term in &terms {
        let record = roster(&format!(
            "[[agent]]\nid = \"peer\"\nrole = \"Desk\"\ndescription = \"{term}\"\n"
        ));
        assert!(
            pick_peer(&record, question, "researcher").is_some(),
            "a role described by the focused term {term:?} must be reachable by the peer rung"
        );
    }
    for dropped in ["answer", "must", "include", "the", "for"] {
        let record = roster(&format!(
            "[[agent]]\nid = \"peer\"\nrole = \"Desk\"\ndescription = \"{dropped}\"\n"
        ));
        assert!(
            pick_peer(&record, question, "researcher").is_none(),
            "a word the fact rung drops must not select a peer either: {dropped:?}"
        );
    }
}

/// A [`RunTurn`] that answers a peer consultation with a canned reply, and
/// counts the two dispatch seams separately so a regression that routed a
/// consultation through `run_background_workflow` — which tags live frames
/// with a run/node id and renders the consultation as a node — is visible.
struct PeerStub {
    reply: &'static str,
    fail: bool,
    stall: bool,
    background_calls: std::sync::atomic::AtomicUsize,
    workflow_calls: std::sync::atomic::AtomicUsize,
}

impl PeerStub {
    fn answering(reply: &'static str) -> Self {
        Self {
            reply,
            fail: false,
            stall: false,
            background_calls: std::sync::atomic::AtomicUsize::new(0),
            workflow_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::answering("")
        }
    }

    fn stalling() -> Self {
        Self {
            stall: true,
            ..Self::answering("")
        }
    }

    fn background(&self) -> usize {
        self.background_calls
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl RunTurn for PeerStub {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("a consultation routes through run_background")
    }

    async fn run_steered(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<std::sync::Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("a consultation routes through run_background")
    }

    async fn run_steered_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<std::sync::Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        unreachable!("a consultation routes through run_background")
    }

    async fn run_background(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _run_sink: Option<std::sync::Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.background_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.stall {
            tokio::time::sleep(PEER_TIMEOUT * 4).await;
        }
        if self.fail {
            return Err(crate::OpenCompanyError::Store(
                "the peer's harness is down".to_string(),
            ));
        }
        Ok(crate::harness::TurnOutcome {
            reply: self.reply.to_string(),
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
        _run_sink: Option<std::sync::Arc<crate::harness::run_trace::RunTraceSink>>,
        _workflow_run_id: &str,
        _node_id: &str,
    ) -> crate::Result<crate::harness::TurnOutcome> {
        self.workflow_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(crate::harness::TurnOutcome {
            reply: self.reply.to_string(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
}

/// Drives `ask_around`'s peer rung against `stub`, with no fact store and an
/// empty workspace, so the first two rungs are guaranteed to find nothing.
async fn ladder_with_peer(
    dir: &std::path::Path,
    stub: &PeerStub,
    record: &CompanyRecord,
    question: &str,
) -> RecoveryResult {
    let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(String::new(), dir);
    ask_around(
        &deps,
        &CompanyId::new("acme"),
        question,
        Some(PeerConsult {
            turn: stub,
            record,
            exclude_agent: "researcher",
            workflow_id: "wf",
        }),
    )
    .await
}

#[tokio::test]
async fn a_peer_answer_lands_as_provenance_tagged_evidence() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-answer-")
        .tempdir()
        .expect("tempdir");
    let stub = PeerStub::answering("Enterprise renewals are priced at 12% uplift.");
    let record = roster(THREE_DESKS);
    let result = ladder_with_peer(
        dir.path(),
        &stub,
        &record,
        "The answer must include the renewal pricing",
    )
    .await;

    let evidence = result.evidence.expect("the peer answered");
    assert_eq!(
        evidence,
        "peer cfo (Chief Financial Officer): Enterprise renewals are priced at 12% uplift.",
        "the answer must carry its provenance into the judge's input and the blocker card"
    );
    assert!(
        result.log.contains("peer: cfo answered"),
        "the recovery log must say who answered: {}",
        result.log
    );
    assert_eq!(stub.background(), 1, "exactly one peer turn is spent");
    assert_eq!(
        stub.workflow_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a consultation is not a node and must not route through the workflow-tagged seam"
    );
}

#[tokio::test]
async fn a_peer_with_no_answer_leaves_the_ladder_empty() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-empty-")
        .tempdir()
        .expect("tempdir");
    let stub = PeerStub::answering("   ");
    let record = roster(THREE_DESKS);
    let result = ladder_with_peer(
        dir.path(),
        &stub,
        &record,
        "The answer must include the renewal pricing",
    )
    .await;

    assert_eq!(result.evidence, None, "an empty reply is not evidence");
    assert!(
        result.log.contains("peer: cfo had no answer"),
        "the log must distinguish an empty answer from nobody being asked: {}",
        result.log
    );
    assert_eq!(stub.background(), 1);
}

#[tokio::test]
async fn a_failed_peer_turn_leaves_the_ladder_empty() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-fail-")
        .tempdir()
        .expect("tempdir");
    let stub = PeerStub::failing();
    let record = roster(THREE_DESKS);
    let result = ladder_with_peer(
        dir.path(),
        &stub,
        &record,
        "The answer must include the renewal pricing",
    )
    .await;

    assert_eq!(result.evidence, None, "a failed turn is not evidence");
    assert!(
        result.log.contains("peer: cfo could not answer"),
        "the log must name the failure rather than report a clean miss: {}",
        result.log
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_turn_that_overruns_its_bound_times_out() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-timeout-")
        .tempdir()
        .expect("tempdir");
    let stub = PeerStub::stalling();
    let record = roster(THREE_DESKS);
    let result = ladder_with_peer(
        dir.path(),
        &stub,
        &record,
        "The answer must include the renewal pricing",
    )
    .await;

    assert_eq!(result.evidence, None, "a timed-out turn is not evidence");
    assert!(
        result.log.contains("peer: cfo timed out"),
        "an unbounded peer turn is the failure this rung must not have: {}",
        result.log
    );
}

#[tokio::test]
async fn a_question_no_role_matches_asks_nobody() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-nomatch-")
        .tempdir()
        .expect("tempdir");
    let stub = PeerStub::answering("should never be reached");
    let record = roster(THREE_DESKS);
    let result = ladder_with_peer(
        dir.path(),
        &stub,
        &record,
        "the warehouse forklift inspection",
    )
    .await;

    assert_eq!(result.evidence, None);
    assert_eq!(
        stub.background(),
        0,
        "no role matched, so no turn may be spent"
    );
    assert!(
        result.log.contains("peer: skipped; no role matched"),
        "{}",
        result.log
    );
}

#[tokio::test]
async fn a_spent_total_ceiling_never_asks_a_peer() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-ceiling-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    deps.plan = Some(crate::harness::capability_budget::CapabilityPlan {
        period: crate::harness::capability_budget::BudgetPeriod::Daily,
        budgets: Default::default(),
        total_budget: Some(1_000),
    });
    deps.meter = Some(std::sync::Arc::new(ExhaustedMeter));
    let stub = PeerStub::answering("should never be reached");
    let record = roster(THREE_DESKS);

    let result = ask_around(
        &deps,
        &CompanyId::new("acme"),
        "The answer must include the renewal pricing",
        Some(PeerConsult {
            turn: &stub,
            record: &record,
            exclude_agent: "researcher",
            workflow_id: "wf",
        }),
    )
    .await;

    assert_eq!(
        stub.background(),
        0,
        "a company past its token ceiling must not pay for a peer turn"
    );
    assert_eq!(result.evidence, None);
    assert!(
        result.log.contains("peer: skipped; token ceiling reached"),
        "{}",
        result.log
    );
}

#[tokio::test]
async fn a_fact_match_never_asks_a_peer() {
    let dir = tempfile::Builder::new()
        .prefix("oc-1866-peer-last-")
        .tempdir()
        .expect("tempdir");
    let (mut deps, _journal) =
        crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
    deps.facts = Some(std::sync::Arc::new(ExactMatchFactStore {
        matches: "renewal",
        fact: crate::ports::FactRecord {
            id: "f1".to_string(),
            kind: crate::ports::FactKind::Fact,
            title: "Renewal pricing".to_string(),
            body: "Enterprise renewals carry a 12% uplift.".to_string(),
            source: "test".to_string(),
            updated_at_millis: 0,
        },
    }));
    let stub = PeerStub::answering("should never be reached");
    let record = roster(THREE_DESKS);

    let result = ask_around(
        &deps,
        &CompanyId::new("acme"),
        "The answer must include the renewal pricing",
        Some(PeerConsult {
            turn: &stub,
            record: &record,
            exclude_agent: "researcher",
            workflow_id: "wf",
        }),
    )
    .await;

    assert_eq!(
        stub.background(),
        0,
        "the peer rung is last; a local answer must add no always-on turn cost"
    );
    assert!(
        result.log.contains("peer: skipped after local match"),
        "{}",
        result.log
    );
}
