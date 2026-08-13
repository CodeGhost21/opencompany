//! Emitting [`SampleKind::TriageCall`] usage samples — what a triage
//! escalation costs, and who it is charged to (issue #678).
//!
//! # Why this is not a teammate's inference
//!
//! A triage escalation is the tool-less model call an operator message makes
//! when the lexical classifier in [`task_intent`](crate::company::task_intent)
//! abstained. It happens **before** any teammate is chosen — deciding whether
//! the orchestrator may write to the board at all is upstream of who answers —
//! so there is no agent to attribute it to. Like a planning pass, it is charged
//! to the whole-company bucket ([`UNATTRIBUTED_AGENT`]) with no `run_id`.
//!
//! It is deliberately not folded into
//! [`SampleKind::PlanningCall`](crate::ports::usage::SampleKind) either. The two
//! are driven by different things — planning by cards entering `planning`,
//! triage by raw chat volume — so sharing a kind would make the planning line
//! item move whenever chat got busier, and neither number could be tuned
//! against the other.
//!
//! # Both writes are logged and swallowed
//!
//! Same rule as the planning path: the classification has already been made and
//! the operator's turn is already running by the time this is called. A ledger
//! or meter hiccup must cost the accounting row and never the reply. The tokens
//! were genuinely spent either way, which is why the failure is logged rather
//! than silently dropped.

use crate::ports::types::{CompanyId, TokenUsage};
use crate::ports::usage::{SampleKind, UsageMeter, UsageSample};
use crate::ports::{CompanyStore, now_millis};

use super::inference::{UNATTRIBUTED_AGENT, inference_ledger_entry};

/// Builds the [`SampleKind::TriageCall`] sample for one completed escalation, or
/// `None` when it moved no tokens and cost nothing.
///
/// The `None` case is the offline/mock path, exactly as in
/// [`planning_sample`](super::planning_sample): a provider reporting no usage
/// yields a zero [`TokenUsage`], and a row for it would claim a call happened
/// that is indistinguishable from a real free one.
///
/// `agent` is not a parameter — attribution to [`UNATTRIBUTED_AGENT`] is the
/// rule this module holds, so no caller can bill a classification to a desk.
pub fn triage_sample(usage: &TokenUsage, provider: &str) -> Option<UsageSample> {
    if usage.is_zero() {
        return None;
    }
    Some(UsageSample {
        at_millis: now_millis(),
        agent: UNATTRIBUTED_AGENT.to_string(),
        provider: super::oauth::normalize_provider(provider),
        input_tokens: usage.input,
        output_tokens: usage.output,
        cached_input_tokens: usage.cached_input,
        cost_usd: usage.cost_usd,
        kind: SampleKind::TriageCall,
        run_id: None,
    })
}

/// Records one completed triage escalation: the Finances ledger entry (when it
/// cost USD) and the usage sample (when it moved tokens or money).
///
/// The ledger entry goes through the same [`inference_ledger_entry`] the cycle's
/// inference spend uses, under the same `inference.spend` kind — triage spend is
/// inference spend as far as the money is concerned, and only the *usage*
/// breakdown cares about the distinction.
pub async fn record_triage_usage(
    usage: &TokenUsage,
    provider: &str,
    company: &CompanyId,
    store: &dyn CompanyStore,
    meter: &dyn UsageMeter,
) {
    if usage.is_zero() {
        return;
    }
    tracing::debug!(
        company = %company,
        provider = %provider,
        input = usage.input,
        output = usage.output,
        cached_input = usage.cached_input,
        cost_usd = usage.cost_usd,
        "[usage] recording a triage escalation"
    );
    if let Some(entry) = inference_ledger_entry(usage, UNATTRIBUTED_AGENT)
        && let Err(err) = store.append_ledger(company, entry).await
    {
        tracing::warn!(
            company = %company,
            error = %err,
            "[usage] failed to append the triage spend entry; the classification still stands"
        );
    }
    if let Some(sample) = triage_sample(usage, provider)
        && let Err(err) = meter.record(company, &sample).await
    {
        tracing::warn!(
            company = %company,
            error = %err,
            "[usage] failed to record the triage usage sample; the classification still stands"
        );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn usage() -> TokenUsage {
        TokenUsage {
            input: 120,
            output: 3,
            cached_input: 0,
            cost_usd: 0.0004,
        }
    }

    #[test]
    fn a_completed_escalation_is_charged_to_the_company_not_a_teammate() {
        let sample = triage_sample(&usage(), "managed").expect("a sample for real spend");
        assert_eq!(sample.kind, SampleKind::TriageCall);
        assert_eq!(
            sample.agent, UNATTRIBUTED_AGENT,
            "triage runs before a teammate is chosen, so no teammate may be billed"
        );
        assert!(
            sample.run_id.is_none(),
            "an escalation belongs to no attempt"
        );
    }

    /// The offline path. A mock provider reports nothing, and a zero row would be
    /// indistinguishable from a real call that happened to be free.
    #[test]
    fn an_escalation_that_moved_nothing_writes_no_row() {
        assert!(triage_sample(&TokenUsage::default(), "managed").is_none());
    }

    /// Distinct from planning, on purpose: the two are driven by different
    /// things and tuned separately.
    #[test]
    fn triage_is_not_filed_as_planning() {
        let sample = triage_sample(&usage(), "managed").expect("a sample");
        assert_ne!(sample.kind, SampleKind::PlanningCall);
    }
}
