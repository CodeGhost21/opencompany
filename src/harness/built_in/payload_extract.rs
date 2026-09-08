//! Task-aware extraction of an oversized tool result (issue #6014).
//!
//! # What this replaces
//!
//! A tool result larger than the per-result budget used to be **cut on a byte
//! boundary**. That keeps the first few whole records and discards every one
//! after them, which is the wrong end to lose: a listing of thirty issues
//! became two, and the agent — correctly reporting what it could see — told the
//! operator there were two.
//!
//! Two cheaper transforms were tried first and are worth recording so nobody
//! re-derives them. **Dropping navigation fields**
//! ([`project_records_value`](crate::harness::composio_catalog::project_records_value))
//! measured **1.6x** on a real 237 KB GitHub payload — real, free, and nowhere
//! near enough. **Capping every long string** would have kept all thirty
//! records, but it clips the one body the operator asked about exactly as hard
//! as the twenty-nine they did not, because it has no idea which is which.
//!
//! The size is not the problem. The problem is that **nothing in the pipeline
//! knew what the turn was for**, so every strategy had to guess uniformly.
//!
//! # What it does instead
//!
//! [`PayloadSummarizer::maybe_summarize_in_parent`] is handed the tool name,
//! the raw payload **and the turn's task hint**, so the compression can keep
//! what answers the question and drop what does not. That trait has existed
//! upstream all along; OpenCompany simply never set it
//! (`AgentBuilder::payload_summarizer` defaults to `None`, and the only wiring
//! site is OpenHuman's own factory, which builds a **sub-agent** implementation
//! this crate cannot use — see `toolbelt`'s v1 note on why spawn tools are
//! withheld under multi-tenancy).
//!
//! So this is the same contract with a different engine: **one bounded,
//! tool-less model call**, the shape
//! [`TriageEvaluator`](crate::harness::triage), the card titler and
//! [`TaskPlanner`](crate::harness::planning) already use — built `from_deps`
//! so it shares the company's provider, its BYOK switch and its metering
//! rather than resolving a second credential path.
//!
//! # Failure is never fatal
//!
//! Every failure path returns [`SummarizeOutcome::Unavailable`], never `Err`:
//! a summarizer that times out must leave the turn holding the raw payload
//! (which downstream truncation still bounds), not kill the tool call. The
//! reason rides along so the model is told the result is unsummarized rather
//! than being handed a fragment silently.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use openhuman_core::openhuman as oh;

use oh::agent::tinyagents::payload_summarizer::{
    PayloadSummarizer, SummarizeOutcome, SummarizedPayload, UnavailableReason,
};

use crate::harness::HarnessDeps;
use crate::harness::build::model_for_tier;

/// How long one extraction may take before the turn gives up on it.
///
/// Deliberately short. This call sits **between a tool returning and the model
/// seeing its result**, so every millisecond is latency the operator watches
/// accumulate mid-turn. A slow extraction is worse than none: the raw payload
/// is still bounded downstream, so the fallback is the behaviour that shipped
/// before this existed.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(25);

/// Ceiling on the extraction's own output, so a summarizer cannot answer a
/// budget problem with a second one.
const MAX_SUMMARY_TOKENS: u32 = 1_500;

/// One bounded model call that pulls the answering content out of an oversized
/// tool result. See the module docs.
pub struct PayloadExtractor {
    model: Arc<dyn tinyinference::model::ChatModel<()>>,
    model_name: String,
}

impl PayloadExtractor {
    /// Built from the same deps the roster is, so the extraction spends the
    /// company's own credential and is metered against it — the reason every
    /// other one-shot pass in this crate is constructed this way rather than
    /// resolving its own.
    pub fn from_deps(deps: &HarnessDeps) -> Self {
        let model_name = deps
            .model_override
            .clone()
            .unwrap_or_else(|| model_for_tier(None));
        Self {
            model: deps.provider.clone() as Arc<dyn tinyinference::model::ChatModel<()>>,
            model_name,
        }
    }
}

/// The instruction: OpenHuman's own summarizer archetype, verbatim.
///
/// Not a prompt of this crate's own. The first cut here was, and it was a
/// weaker restatement of a prompt that already existed two directories away —
/// it dropped the structural hints ("if the payload is a list, state how many
/// items it had... what page boundaries exist"), the error-payload rule
/// ("preserve the error message verbatim at the top"), the binary-payload rule,
/// and the `Identifiers preserved` section that gives every kept record a line
/// of its own. That last omission showed up immediately in testing: a task that
/// asked for issue numbers got thirty numbers and no titles, because nothing
/// told the model to keep an identifying line per record regardless of what was
/// asked.
///
/// Referenced through the vendored const rather than copied, so the two cannot
/// drift: an upstream edit to the extraction contract reaches this caller on
/// the next vendor bump instead of leaving OpenCompany on a stale fork of it.
///
/// The archetype is written for a sub-agent invocation, and every line of that
/// framing holds here — "you run exactly once per invocation, with no tools and
/// no follow-up iterations" is precisely what this single tool-less call is.
fn system_prompt() -> &'static str {
    oh::agent::registry::agents::summarizer::prompt::ARCHETYPE
}

#[async_trait]
impl PayloadSummarizer for PayloadExtractor {
    async fn maybe_summarize_in_parent(
        &self,
        _parent_ctx: &tinyagents_harness::context::RunContext<()>,
        tool_name: &str,
        parent_task_hint: Option<&str>,
        raw: &str,
    ) -> anyhow::Result<SummarizeOutcome> {
        let original_bytes = raw.len();
        // The task hint is the whole point of this over a mechanical cut. With
        // none, an extraction has no way to tell an answering record from a
        // filler one, and would be guessing exactly as blindly as the byte cut
        // — so decline and let the bounded raw payload through, which at least
        // says what it dropped.
        // The upstream hint when a caller supplies one (the sub-agent path
        // does), else this crate's own — set by `run_inner` around every turn,
        // because OpenCompany does not take the sub-agent path that populates
        // the parameter.
        let own_hint = crate::runtime::delegation::current_task_hint();
        let Some(task) = parent_task_hint
            .map(str::to_string)
            .or(own_hint)
            .map(|hint| hint.trim().to_string())
            .filter(|hint| !hint.is_empty())
        else {
            tracing::debug!(
                tool = tool_name,
                bytes = original_bytes,
                "[payload-extract] no task hint on this turn; leaving the payload to the \
                 downstream bound"
            );
            return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Disabled));
        };

        let request = tinyinference::model::ModelRequest {
            messages: vec![
                tinyinference::message::Message::system(system_prompt()),
                tinyinference::message::Message::user(format!(
                    "The agent is trying to: {task}\n\nTool that ran: `{tool_name}`\n\n\
                     Raw output:\n{raw}"
                )),
            ],
            model: Some(self.model_name.clone()),
            max_tokens: Some(MAX_SUMMARY_TOKENS),
            ..Default::default()
        };

        let response =
            match tokio::time::timeout(EXTRACT_TIMEOUT, self.model.invoke(&(), request)).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    tracing::warn!(
                        tool = tool_name,
                        bytes = original_bytes,
                        %error,
                        "[payload-extract] extraction call failed; the raw payload stands"
                    );
                    return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed));
                }
                Err(_) => {
                    tracing::warn!(
                        tool = tool_name,
                        bytes = original_bytes,
                        timeout_s = EXTRACT_TIMEOUT.as_secs(),
                        "[payload-extract] extraction timed out; the raw payload stands"
                    );
                    return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed));
                }
            };

        let summary = response.text();
        if summary.trim().is_empty() {
            tracing::warn!(
                tool = tool_name,
                bytes = original_bytes,
                "[payload-extract] extraction returned nothing; the raw payload stands"
            );
            return Ok(SummarizeOutcome::Unavailable(UnavailableReason::Failed));
        }
        // An "extraction" that grew the payload is not one. Rare, but a model
        // handed a small-but-over-threshold body can pad; taking it anyway
        // would spend a call to make the budget problem worse.
        if summary.len() >= original_bytes {
            tracing::debug!(
                tool = tool_name,
                original_bytes,
                summary_bytes = summary.len(),
                "[payload-extract] extraction did not shrink the payload; keeping the raw output"
            );
            return Ok(SummarizeOutcome::NotNeeded);
        }

        tracing::info!(
            tool = tool_name,
            from_bytes = original_bytes,
            to_bytes = summary.len(),
            ratio = format!(
                "{:.1}x",
                original_bytes as f64 / summary.len().max(1) as f64
            ),
            "[payload-extract] extracted the answering content from an oversized tool result"
        );
        Ok(SummarizeOutcome::Summarized(SummarizedPayload {
            summary_bytes: summary.len(),
            summary,
            original_bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The archetype is referenced, not restated. A copy would drift from
    /// upstream the moment either side edited the extraction contract, and the
    /// first version of this file learned that the expensive way — a
    /// hand-written prompt that omitted the per-record identifier line produced
    /// thirty issue numbers with no titles.
    #[test]
    fn the_prompt_is_openhumans_own_archetype() {
        assert_eq!(
            system_prompt(),
            oh::agent::registry::agents::summarizer::prompt::ARCHETYPE,
            "the prompt must stay the vendored archetype, not a local copy"
        );
        // The clauses whose absence was actually observed to cost something.
        assert!(
            system_prompt().contains("Identifiers preserved"),
            "the per-record identifier section is what gives each kept record a line"
        );
        assert!(
            system_prompt().contains("Never drop them"),
            "identifiers are the archetype's first-order rule"
        );
    }
}
