//! The opt-in semantic sufficiency gate for workflow agent nodes (issue #1866).

use std::time::Duration;

use serde::Deserialize;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ModelRequest, ModelResponse};

use crate::harness::HarnessDeps;
use crate::harness::build::model_for_tier;
use crate::ports::blockers::BlockerKind;
use crate::ports::types::{CompanyId, TokenUsage};

const JUDGE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_TOKENS: u32 = 256;
const MAX_QUERY_CHARS: usize = 800;
const MAX_RECOVERY_ITEMS: usize = 3;
const MAX_RECOVERY_CHARS: usize = 4_000;
const INSTRUCTION_CAP: usize = 8_000;
const INSTRUCTION_TAIL_RESERVE: usize = 2_000;
const OUTPUT_CAP: usize = 20_000;
const RECOVERED_CONTEXT_HEADING: &str = "\n\nRecovered company context:\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SufficiencyVerdict {
    Continue,
    Retry,
    Recover,
    Escalate { gap: BlockerKind },
    HaltBenign,
}

impl SufficiencyVerdict {
    pub const fn token(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Retry => "retry",
            Self::Recover => "recover",
            Self::Escalate { .. } => "escalate",
            Self::HaltBenign => "halt_benign",
        }
    }

    pub const fn tokens() -> [&'static str; 5] {
        [
            Self::Continue.token(),
            Self::Retry.token(),
            Self::Recover.token(),
            Self::Escalate {
                gap: BlockerKind::Information,
            }
            .token(),
            Self::HaltBenign.token(),
        ]
    }

    fn from_parts(token: &str, gap: Option<&str>) -> Option<Self> {
        let tokens = Self::tokens();
        Some(if token == tokens[0] {
            Self::Continue
        } else if token == tokens[1] {
            Self::Retry
        } else if token == tokens[2] {
            Self::Recover
        } else if token == tokens[3] {
            Self::Escalate {
                gap: BlockerKind::from_wire(gap?)?,
            }
        } else if token == tokens[4] {
            Self::HaltBenign
        } else {
            return None;
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JudgeInput<'a> {
    pub instruction: &'a str,
    pub output: &'a str,
    pub criteria: Option<&'a str>,
    pub execution_failed: bool,
}

#[derive(Debug, Deserialize)]
struct JudgeAnswer {
    verdict: String,
    #[serde(default)]
    gap: Option<String>,
}

fn system_prompt() -> String {
    let [
        continue_token,
        retry_token,
        recover_token,
        escalate_token,
        halt_token,
    ] = SufficiencyVerdict::tokens();
    let gaps = [
        BlockerKind::Information.as_str(),
        BlockerKind::Infrastructure.as_str(),
        BlockerKind::Transient.as_str(),
    ]
    .join("|");
    format!(
        "Judge whether one workflow node's output is semantically sufficient. Return one JSON \
         object only: {{\"verdict\":\"TOKEN\",\"gap\":\"GAP\"}}. TOKEN is exactly one of \
         {continue_token}|{retry_token}|{recover_token}|{escalate_token}|{halt_token}. GAP is \
         required only for {escalate_token} and is exactly one of {gaps}. Choose {continue_token} \
         when the output fulfills the instruction and criteria. Choose {retry_token} for an \
         inadequate attempt likely fixed by rerunning. Choose {recover_token} only when existing \
         company knowledge may fill a missing fact. Choose {escalate_token} when a person or \
         infrastructure change is required. Choose {halt_token} only when the requested work is \
         intentionally unnecessary or already satisfied. Never choose {halt_token} for blank, \
         failed, errored, refused, truncated, or otherwise insufficient output. Do not use tools."
    )
}

fn user_prompt(input: JudgeInput<'_>) -> String {
    format!(
        "INSTRUCTION:\n{}\n\nSUCCESS CRITERIA:\n{}\n\nEXECUTION FAILED: {}\n\nOUTPUT:\n{}",
        cap_keeping_tail(input.instruction, INSTRUCTION_CAP, INSTRUCTION_TAIL_RESERVE),
        input.criteria.unwrap_or("No additional criteria supplied."),
        input.execution_failed,
        cap(input.output, OUTPUT_CAP),
    )
}

/// Composes a reply and the evidence recovery found into the text the
/// re-verification pass judges, keeping the evidence inside [`OUTPUT_CAP`].
pub(crate) fn augment_with_recovery(reply: &str, evidence: &str) -> String {
    let block = format!("{RECOVERED_CONTEXT_HEADING}{evidence}");
    let room = OUTPUT_CAP.saturating_sub(block.chars().count() + 1);
    format!("{}{block}", cap(reply, room))
}

fn parse_verdict(text: &str) -> Option<SufficiencyVerdict> {
    let answer: JudgeAnswer = serde_json::from_str(text.trim()).ok()?;
    SufficiencyVerdict::from_parts(answer.verdict.trim(), answer.gap.as_deref().map(str::trim))
}

fn enforce_anti_suppression(
    verdict: SufficiencyVerdict,
    input: JudgeInput<'_>,
) -> SufficiencyVerdict {
    if verdict == SufficiencyVerdict::HaltBenign
        && (input.execution_failed || input.output.trim().is_empty())
    {
        SufficiencyVerdict::Retry
    } else {
        verdict
    }
}

pub async fn judge_sufficiency(
    deps: &HarnessDeps,
    company: &CompanyId,
    input: JudgeInput<'_>,
) -> SufficiencyVerdict {
    if crate::harness::HarnessPool::total_ceiling_spent(company, deps).await {
        tracing::info!(
            company = %company,
            "[capability-budget] total token ceiling reached; skipping the sufficiency judge \
             (no model call)"
        );
        return SufficiencyVerdict::Retry;
    }

    let model = deps
        .model_override
        .clone()
        .unwrap_or_else(|| model_for_tier(None));
    let request = ModelRequest {
        messages: vec![
            Message::system(system_prompt()),
            Message::user(user_prompt(input)),
        ],
        model: Some(model),
        temperature: Some(0.0),
        max_tokens: Some(MAX_OUTPUT_TOKENS),
        ..ModelRequest::default()
    };
    let response = match tokio::time::timeout(JUDGE_TIMEOUT, deps.provider.invoke(&(), request))
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            tracing::warn!(company = %company, error = %err, "workflow sufficiency judge failed");
            return SufficiencyVerdict::Retry;
        }
        Err(_) => {
            tracing::warn!(company = %company, "workflow sufficiency judge timed out");
            return SufficiencyVerdict::Retry;
        }
    };

    let usage = usage_from(&response);
    crate::metering::record_judge_usage(
        &usage,
        &deps.provider.telemetry_provider_id(),
        deps.provider.telemetry_model(),
        company,
        deps.store.as_ref(),
        deps.meter.as_deref(),
    )
    .await;

    let verdict = parse_verdict(&response.text()).unwrap_or(SufficiencyVerdict::Retry);
    enforce_anti_suppression(verdict, input)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryResult {
    pub evidence: Option<String>,
    pub log: String,
}

const MAX_FOCUS_TERMS: usize = 6;
const MIN_FOCUS_TERM_LEN: usize = 4;

/// Candidate single-word `FactStore` queries pulled out of a natural-language
/// question (issue #1990 review, #3903591432): `FactStore::list`'s query is a
/// case-insensitive substring match over a fact's title + body, so the whole
/// question almost never appears verbatim even when a fact IS the answer — a
/// fact titled "Renewal date" never matches the sentence "The answer must
/// include the renewal date". Lowercased, deduplicated, common short/filler
/// words dropped, capped so the bounded recovery ladder stays bounded.
fn focus_terms(question: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the",
        "and",
        "that",
        "this",
        "with",
        "from",
        "must",
        "have",
        "will",
        "shall",
        "should",
        "answer",
        "include",
        "includes",
        "criteria",
        "success",
        "about",
        "into",
        "onto",
        "than",
        "then",
        "when",
        "what",
        "which",
        "specifically",
        "please",
        "need",
        "needs",
    ];
    let mut terms = Vec::new();
    for word in question.split(|c: char| !c.is_alphanumeric()) {
        let lower = word.to_lowercase();
        if lower.chars().count() < MIN_FOCUS_TERM_LEN {
            continue;
        }
        if STOPWORDS.contains(&lower.as_str()) || terms.contains(&lower) {
            continue;
        }
        terms.push(lower);
        if terms.len() >= MAX_FOCUS_TERMS {
            break;
        }
    }
    terms
}

/// Bounded fact → workspace → one-peer recovery. The peer rung is explicitly
/// unavailable until #1859 lands; it is skipped, never broadcast to the roster.
pub async fn ask_around(deps: &HarnessDeps, company: &CompanyId, question: &str) -> RecoveryResult {
    let query = cap(question, MAX_QUERY_CHARS);
    let mut evidence = Vec::new();
    let mut log = Vec::new();

    if let Some(facts) = deps.facts.as_ref() {
        let mut queries = vec![query.clone()];
        queries.extend(focus_terms(&query));
        let mut seen_ids = std::collections::HashSet::new();
        let mut query_errors = Vec::new();
        'queries: for q in &queries {
            match facts.list(company, Some(q), None).await {
                Ok(rows) => {
                    for row in rows {
                        if evidence.len() >= MAX_RECOVERY_ITEMS {
                            break 'queries;
                        }
                        if seen_ids.insert(row.id.clone()) {
                            evidence.push(format!("fact: {} — {}", row.title, row.body));
                        }
                    }
                }
                Err(err) => query_errors.push(err.to_string()),
            }
        }
        if evidence.is_empty() && !query_errors.is_empty() {
            log.push(format!("facts: unavailable ({})", query_errors.join("; ")));
        } else {
            log.push(format!("facts: {} match(es)", evidence.len()));
        }
    } else {
        log.push("facts: unavailable".to_string());
    }

    if evidence.is_empty() {
        match deps
            .context
            .search(company, &query, MAX_RECOVERY_ITEMS)
            .await
        {
            Ok(hits) => {
                for hit in hits {
                    evidence.push(format!("workspace: {}", hit.snippet));
                }
                log.push(format!("workspace: {} match(es)", evidence.len()));
            }
            Err(err) => log.push(format!("workspace: unavailable ({err})")),
        }
    } else {
        log.push("workspace: skipped after fact match".to_string());
    }

    if evidence.is_empty() {
        log.push("peer: skipped; board read capability #1859 is unavailable".to_string());
    } else {
        log.push("peer: skipped after local match".to_string());
    }

    let joined = cap(&evidence.join("\n"), MAX_RECOVERY_CHARS);
    RecoveryResult {
        evidence: (!joined.is_empty()).then_some(joined),
        log: log.join("; "),
    }
}

/// Caps `text` to `max` characters while keeping its last `tail` characters,
/// which is where [`compose_turn_message`](crate::workflows::caps) leaves the
/// run's own request.
fn cap_keeping_tail(text: &str, max: usize, tail: usize) -> String {
    let trimmed = text.trim();
    let total = trimmed.chars().count();
    if total <= max {
        return trimmed.to_string();
    }
    let tail = tail.min(max);
    let head = max - tail;
    let head_text: String = trimmed.chars().take(head).collect();
    let tail_text: String = trimmed.chars().skip(total - tail).collect();
    format!(
        "{head_text}\n[… {} characters elided …]\n{tail_text}",
        total - max
    )
}

fn cap(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    trimmed.chars().take(max).collect::<String>() + "…"
}

fn usage_from(response: &ModelResponse) -> TokenUsage {
    let tokens = response.usage.unwrap_or_default();
    let cost_usd = response
        .raw
        .as_ref()
        .and_then(|raw| raw.pointer("/openhuman_usage_meta/charged_amount_usd"))
        .and_then(serde_json::Value::as_f64)
        .filter(|cost| cost.is_finite() && *cost > 0.0)
        .unwrap_or(0.0);
    TokenUsage {
        input: tokens.input_tokens,
        output: tokens.output_tokens,
        cached_input: tokens.cache_read_tokens,
        cost_usd,
    }
}

#[cfg(test)]
mod tests {
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
    impl tinyagents::harness::model::ChatModel<()> for PanicIfInvokedProvider {
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyagents::Result<ModelResponse> {
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
}
