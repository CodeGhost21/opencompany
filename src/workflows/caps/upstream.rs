//! What an upstream node is allowed to hand a downstream agent turn, and what
//! to say when the provider refuses the turn anyway (issue #849).
//!
//! Issue #782 gave an upstream node's output a channel into the next agent's
//! turn. Nothing bounded that channel, so a fan-in — gather N sources, rank them
//! — concatenated every predecessor's payload into one turn verbatim. Three
//! `web_fetch` nodes reading busy pages intermittently pushed that turn past the
//! model's context window, and the run discovered it as a raw provider 400
//! **after** the fetches were already paid for:
//!
//! ```text
//! inference returned 400 Bad Request: {"success":false,
//!  "error":"The conversation is too long for model 'chat-v1'. Please start a new chat.",
//!  "errorCode":"CONTEXT_LENGTH_EXCEEDED"}
//! ```
//!
//! Four of seven runs of the *same* graph over the *same* sources succeeded; the
//! only variable was how much text the pages happened to return that minute. A
//! workflow whose outcome is a coin flip is the defect, not the 400.
//!
//! # The three things this module does
//!
//! 1. **Bounds the payload.** [`budget_chars`] fixes how many characters of
//!    upstream input one agent node may carry, and [`allocate_fairly`] divides
//!    that budget across the predecessors so one enormous source cannot crowd
//!    the others out. Whatever is cut is cut *visibly*: [`truncation_marker`]
//!    leaves a marker in the turn itself, so the agent knows it is reasoning
//!    over a fragment and cannot present it as the whole.
//! 2. **Says so before spending.** The budget is applied while the turn is being
//!    composed — before the request reaches the provider — and
//!    [`UpstreamReport::notice`] names how much arrived, how much fitted and how
//!    many sources were cut, on [`WorkflowRun::notices`](crate::ports::WorkflowRun::notices),
//!    the run surface the operator reads.
//! 3. **Translates the refusal.** [`context_overflow_advice`] rewrites a
//!    provider context-window 400 into something actionable inside a workflow.
//!    "Please start a new chat" is advice for a chat product; a workflow step has
//!    no chat and the operator has no button for it.
//!
//! # Truncate-and-report rather than fail-the-run
//!
//! The issue asks to fail before spending. The spending has already happened by
//! the time a fan-in is composed — the fetches ran first — so failing the run
//! here would throw that paid work away and deliver the operator nothing, to
//! avoid a turn that a bounded input makes succeed. So the boundary is enforced
//! by bounding, and the "which inputs overflowed" report rides the marker and
//! the notice instead of an error. The one overflow that can still reach the
//! provider — a turn whose *own* instruction, tool schemas or accumulated
//! session history is what is too large — is what [`context_overflow_advice`]
//! exists for.
//!
//! # Why not
//! [`bound_node_output`](crate::ports::bound_node_output)
//!
//! That bound already exists and does not help here: it clips the **persisted**
//! run-output snapshot the console reads back, so its budget is a durable-storage
//! ceiling (`RUN_OUTPUT_MAX_BYTES`) applied per string across the whole node map,
//! after the run has settled. This one is applied while a single turn is composed,
//! its budget is a share of a model's context, and it has to divide that budget
//! *between* predecessors — which is the property a per-string cap cannot express.
//! Two bounds on two different resources; a node's output can be well inside the
//! storage cap and still be more than a turn will take.
//!
//! Every function here is pure, so the boundary is driven in tests by a
//! synthetic oversized payload rather than by whatever a live page returns.

/// The most upstream input, in characters, any single agent node's turn carries.
///
/// A ceiling rather than a target: nearly every run is far below it and folds
/// its upstream input untouched. Roughly 8k tokens at the conservative
/// [`CHARS_PER_TOKEN`] estimate — enough for a genuine multi-source gather (the
/// reported failure fanned in three pages of 8.9k–14.7k characters each) while
/// leaving the rest of the window to the step's instruction, its tool schemas
/// and the teammate's session history.
pub(crate) const DEFAULT_UPSTREAM_BUDGET_CHARS: usize = 32_000;

/// Characters per token, used only to turn a model's advertised token window
/// into a character budget.
///
/// Deliberately **low**. An underestimate makes the same text look like more
/// tokens than it is, so the derived budget errs small; an overestimate would
/// err large, which is the direction that reproduces the bug.
const CHARS_PER_TOKEN: usize = 3;

/// The share of a model's advertised input window upstream input may claim.
///
/// The window is not ours alone: the step's own instruction, the run request,
/// every tool schema the teammate carries, and that teammate's accumulated
/// session history are all in the same turn. An eighth leaves room for all of
/// them.
const UPSTREAM_WINDOW_SHARE: usize = 8;

/// How many characters of upstream input a node's turn may carry, given what the
/// configured model advertises as its input window.
///
/// An advertised window can only make the budget **smaller**, never larger, and
/// that asymmetry is the whole point. The reported failure was on `chat-v1`,
/// whose nominal window is enormous, and the turn was refused a long way inside
/// it — because the advertised figure describes the model, not the turn, and
/// says nothing about the system prompt, the tool schemas or the session history
/// sharing it. Trusting it upward is exactly how a fan-in gets handed more than
/// the provider will take. Trusting it downward is safe and worth doing: a small
/// model gets a proportionally smaller budget rather than the flat ceiling.
pub(super) fn budget_chars(max_input_tokens: Option<u64>) -> usize {
    let Some(tokens) = max_input_tokens.filter(|tokens| *tokens > 0) else {
        return DEFAULT_UPSTREAM_BUDGET_CHARS;
    };
    let advertised = usize::try_from(tokens)
        .unwrap_or(usize::MAX)
        .saturating_div(UPSTREAM_WINDOW_SHARE)
        .saturating_mul(CHARS_PER_TOKEN);
    DEFAULT_UPSTREAM_BUDGET_CHARS.min(advertised)
}

/// Divides `budget` across sources of the given `sizes`, returning how many
/// characters each may keep (positionally aligned with `sizes`).
///
/// Max-min fair: every source is offered an equal share, a source needing less
/// than its share takes only what it needs, and what it leaves is redistributed
/// among the larger ones. So three 10k sources under a 32k budget are untouched,
/// while a 300-character source beside two 200k ones keeps all 300 and the two
/// split the rest evenly — a big source is never allowed to starve a small one,
/// which is precisely what "concatenate until the provider complains" did.
///
/// Deterministic regardless of input order: ties break on position, and the
/// allocation is computed by size order rather than arrival order.
pub(super) fn allocate_fairly(sizes: &[usize], budget: usize) -> Vec<usize> {
    let mut by_size: Vec<usize> = (0..sizes.len()).collect();
    by_size.sort_by_key(|&index| (sizes[index], index));

    let mut kept = vec![0usize; sizes.len()];
    let mut remaining = budget;
    let mut unallocated = sizes.len();
    for index in by_size {
        // `unallocated` is non-zero on every iteration by construction: it starts
        // at the number of sources and is decremented exactly once per source.
        let share = remaining / unallocated;
        let take = sizes[index].min(share);
        kept[index] = take;
        remaining -= take;
        unallocated -= 1;
    }
    kept
}

/// Truncates `text` to `keep` characters, on a character boundary.
///
/// Characters, not bytes: a byte slice through the middle of a multi-byte
/// character panics, and a fetched page is exactly where a non-ASCII character
/// lands on an arbitrary offset.
pub(super) fn truncate_chars(text: &str, keep: usize) -> String {
    text.chars().take(keep).collect()
}

/// The marker left in the turn where a source was cut.
///
/// **Visible on purpose.** Silent truncation is worse than the 400 it prevents:
/// the agent would rank three sources believing it had read all of each, and
/// state its answer with the confidence of complete information. The marker says
/// which source, how much it produced, how much survived, and — the load-bearing
/// sentence — that the rest was not read.
pub(super) fn truncation_marker(index: usize, of: usize, produced: usize, kept: usize) -> String {
    if kept == 0 {
        return format!(
            "\n\n[TRUNCATED BY OPENCOMPANY — source {index} of {of} produced {produced} \
             characters and none of it fitted this step's input budget. It was NOT read.]"
        );
    }
    let dropped = produced - kept;
    format!(
        "\n\n[TRUNCATED BY OPENCOMPANY — source {index} of {of} produced {produced} characters; \
         the {kept} above are all that was kept and the remaining {dropped} were NOT read. Do not \
         describe this source as if you have seen all of it.]"
    )
}

/// What one predecessor contributed to a turn, and how much of it survived the
/// budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceBudget {
    /// Characters the source rendered to.
    pub(super) produced: usize,
    /// Characters of it the turn actually carries.
    pub(super) kept: usize,
}

impl SourceBudget {
    /// Whether this source was cut.
    fn truncated(&self) -> bool {
        self.kept < self.produced
    }
}

/// What the budget did to one turn's upstream input — one row per renderable
/// predecessor, in the order they were rendered.
///
/// Produced on **every** fold, including the overwhelmingly common one where
/// nothing was cut; [`notice`](Self::notice) is `None` in that case, so an
/// ordinary run says nothing new.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct UpstreamReport {
    /// Per-source accounting, positionally aligned with the rendered sources.
    pub(super) sources: Vec<SourceBudget>,
    /// The budget those sources were allocated out of.
    pub(super) budget: usize,
}

impl UpstreamReport {
    /// Whether anything was cut. The production path asks
    /// [`notice`](Self::notice) instead (it needs the sentence, not the bool),
    /// so this exists for the assertions that want to say what they mean.
    #[cfg(test)]
    pub(super) fn truncated_any(&self) -> bool {
        self.sources.iter().any(SourceBudget::truncated)
    }

    /// The operator-facing notice for this fold, or `None` when everything
    /// fitted.
    ///
    /// It lands on [`WorkflowRun::notices`](crate::ports::WorkflowRun::notices)
    /// — the surface issue #638 added for exactly this shape of fact: something
    /// the operator needs, that is not a failure, and that no other field on a
    /// run can carry. The run succeeded and its output is valid; it simply did
    /// not see everything, and the operator has to be able to know that without
    /// reading the host's logs.
    pub(super) fn notice(&self) -> Option<String> {
        let cut = self.sources.iter().filter(|s| s.truncated()).count();
        if cut == 0 {
            return None;
        }
        let sources = self.sources.len();
        let produced: usize = self.sources.iter().map(|s| s.produced).sum();
        let kept: usize = self.sources.iter().map(|s| s.kept).sum();
        let budget = self.budget;
        let plural = if sources == 1 { "" } else { "s" };
        let verb = if cut == 1 { "was" } else { "were" };
        Some(format!(
            "A step in this workflow was handed {produced} characters from {sources} \
             source{plural} — more than the {budget} characters one step may pass to a model — so \
             {cut} of them {verb} truncated and the step ran on the {kept} characters that \
             fitted. Its result did not see everything. To use the whole of each source, \
             summarise each one in a step of its own before this one, or have each source return \
             less."
        ))
    }
}

/// Substrings that identify a provider refusal as a context-window overflow.
///
/// Matched case-insensitively against the whole error chain, because the wording
/// is the provider's and varies: the reported one is a managed-backend
/// `CONTEXT_LENGTH_EXCEEDED` with an OpenAI-style sentence, and the OpenAI /
/// Anthropic / OpenRouter surfaces a BYOK tenant reaches all phrase it
/// differently.
const CONTEXT_OVERFLOW_SIGNATURES: &[&str] = &[
    "context_length_exceeded",
    "context length exceeded",
    "conversation is too long",
    "maximum context",
    "context window",
    "prompt is too long",
    "too many tokens",
];

/// Rewrites a context-window refusal into something an operator can act on, or
/// `None` when the error is anything else.
///
/// The message the console showed for the reported failure ended in *"The
/// conversation is too long for model 'chat-v1'. Please start a new chat."* —
/// two vendor sentences that are unactionable here twice over: there is no
/// conversation an operator owns, and no chat for them to start. This names what
/// is actually too big and what to do about each candidate, and keeps the
/// provider's own words at the end so support can still match them.
///
/// It deliberately does not claim the upstream input is the cause. By the time a
/// turn reaches the provider that input has already been bounded, so the
/// remaining suspects are the ones listed.
pub(super) fn context_overflow_advice(error: &str) -> Option<String> {
    let haystack = error.to_lowercase();
    if !CONTEXT_OVERFLOW_SIGNATURES
        .iter()
        .any(|signature| haystack.contains(signature))
    {
        return None;
    }
    Some(format!(
        "the model refused this step's turn because the turn is larger than its context window. \
         This is a workflow step, so there is no chat to restart. The input this step received \
         from earlier steps is already bounded (any truncation is reported on this run), which \
         leaves: the number of steps feeding this one — summarise each source in a step of its \
         own first; this step's own instruction — shorten it; or the teammate's accumulated \
         session history — assign this step a teammate that is not also handling chat. Provider \
         reported: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An advertised window may lower the budget and must never raise it — the
    /// asymmetry the reported `chat-v1` failure is the argument for. A model
    /// claiming a million tokens still gets the flat ceiling.
    #[test]
    fn an_advertised_window_only_lowers_the_budget() {
        assert_eq!(budget_chars(None), DEFAULT_UPSTREAM_BUDGET_CHARS);
        assert_eq!(budget_chars(Some(0)), DEFAULT_UPSTREAM_BUDGET_CHARS);
        assert_eq!(budget_chars(Some(1_000_000)), DEFAULT_UPSTREAM_BUDGET_CHARS);
        // 32k tokens → an eighth of it, at 3 chars/token.
        assert_eq!(budget_chars(Some(32_000)), 12_000);
        // A tiny window yields a tiny budget rather than the ceiling.
        assert_eq!(budget_chars(Some(8_000)), 3_000);
        // No overflow on an absurd advertisement.
        assert_eq!(budget_chars(Some(u64::MAX)), DEFAULT_UPSTREAM_BUDGET_CHARS);
    }

    /// Everything fits → nothing is touched, and the budget is not spent for its
    /// own sake.
    #[test]
    fn sources_that_fit_are_untouched() {
        assert_eq!(allocate_fairly(&[10, 10, 10], 32_000), vec![10, 10, 10]);
        assert_eq!(allocate_fairly(&[], 32_000), Vec::<usize>::new());
    }

    /// The headline allocation property: one enormous source cannot starve a
    /// small one, and the total never exceeds the budget.
    #[test]
    fn a_small_source_survives_beside_enormous_ones() {
        let kept = allocate_fairly(&[300, 200_000, 200_000], 32_000);
        assert_eq!(kept[0], 300, "the small source is served in full");
        assert_eq!(
            kept[1], kept[2],
            "the two large sources split what is left evenly"
        );
        assert_eq!(
            kept.iter().sum::<usize>(),
            32_000,
            "the budget is not exceeded"
        );
    }

    /// Equal-sized oversized sources split the budget equally, and the split is
    /// independent of the order they arrive in.
    #[test]
    fn equal_oversized_sources_split_the_budget_and_order_does_not_matter() {
        assert_eq!(
            allocate_fairly(&[90_000, 90_000, 90_000], 30_000),
            vec![10_000, 10_000, 10_000]
        );
        let ascending = allocate_fairly(&[10, 50_000, 90_000], 20_000);
        let descending = allocate_fairly(&[90_000, 50_000, 10], 20_000);
        assert_eq!(ascending[0], descending[2]);
        assert_eq!(ascending[1], descending[1]);
        assert_eq!(ascending[2], descending[0]);
    }

    /// A budget too small to give every source a character still terminates and
    /// still respects the budget — the degenerate case a division-by-share must
    /// not divide by zero on.
    #[test]
    fn a_budget_smaller_than_the_source_count_still_allocates() {
        let kept = allocate_fairly(&[100, 100, 100], 2);
        assert_eq!(kept.iter().sum::<usize>(), 2);
        assert_eq!(allocate_fairly(&[100], 0), vec![0]);
    }

    /// Truncation is on a character boundary — a fetched page is exactly where a
    /// multi-byte character lands on an arbitrary offset, and a byte slice there
    /// panics.
    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo");
        assert_eq!(truncate_chars("abc", 99), "abc");
        assert_eq!(truncate_chars("abc", 0), "");
    }

    /// The marker names the source, both sizes, and that the rest was not read —
    /// an agent that cannot tell it is holding a fragment will present the
    /// fragment as the whole.
    #[test]
    fn the_marker_says_what_was_cut_and_that_it_was_not_read() {
        let marker = truncation_marker(2, 3, 14_700, 10_000);
        assert!(marker.contains("source 2 of 3"), "{marker}");
        assert!(marker.contains("14700"), "{marker}");
        assert!(marker.contains("10000"), "{marker}");
        assert!(
            marker.contains("4700"),
            "the dropped count is stated: {marker}"
        );
        assert!(marker.contains("NOT read"), "{marker}");

        // Nothing survived at all: the wording must not read as "0 characters
        // above are all that was kept".
        let none = truncation_marker(1, 2, 500, 0);
        assert!(none.contains("none of it fitted"), "{none}");
        assert!(none.contains("NOT read"), "{none}");
    }

    /// A fold where everything fitted says nothing to the operator.
    #[test]
    fn an_untruncated_fold_raises_no_notice() {
        let report = UpstreamReport {
            sources: vec![
                SourceBudget {
                    produced: 10,
                    kept: 10,
                },
                SourceBudget {
                    produced: 20,
                    kept: 20,
                },
            ],
            budget: 32_000,
        };
        assert!(!report.truncated_any());
        assert_eq!(report.notice(), None);
    }

    /// A fold that cut something reports how much arrived, how much fitted, how
    /// many sources were cut, and the remedy.
    #[test]
    fn a_truncated_fold_reports_the_sizes_and_the_remedy() {
        let report = UpstreamReport {
            sources: vec![
                SourceBudget {
                    produced: 100,
                    kept: 100,
                },
                SourceBudget {
                    produced: 40_000,
                    kept: 31_900,
                },
            ],
            budget: 32_000,
        };
        assert!(report.truncated_any());
        let notice = report.notice().expect("a cut fold speaks");
        assert!(notice.contains("40100 characters"), "{notice}");
        assert!(notice.contains("2 sources"), "{notice}");
        assert!(notice.contains("1 of them was truncated"), "{notice}");
        assert!(notice.contains("32000 characters"), "{notice}");
        assert!(notice.contains("summarise each one"), "{notice}");
    }

    /// The reported failure's exact provider string is recognised, and the
    /// rewrite drops the advice that cannot be followed while keeping the
    /// provider's own words.
    #[test]
    fn the_reported_provider_refusal_is_translated() {
        let raw = r#"tinyagents harness run failed: model error: inference returned 400 Bad \
             Request: {"success":false,"error":"The conversation is too long for model \
             'chat-v1'. Please start a new chat.","errorCode":"CONTEXT_LENGTH_EXCEEDED"}"#;
        let advice = context_overflow_advice(raw).expect("recognised as a context overflow");
        assert!(
            advice.contains("no chat to restart"),
            "the unactionable vendor advice is contradicted: {advice}"
        );
        assert!(
            advice.contains("summarise each source"),
            "a remedy is named: {advice}"
        );
        assert!(
            advice.contains("session history"),
            "the remaining suspects are named: {advice}"
        );
        assert!(
            advice.contains("CONTEXT_LENGTH_EXCEEDED"),
            "the provider's own words survive for support: {advice}"
        );
    }

    /// The other providers' wordings are recognised too — the phrasing is the
    /// provider's, and a BYOK tenant reaches several.
    #[test]
    fn other_provider_wordings_are_recognised() {
        for raw in [
            "This model's maximum context length is 8192 tokens",
            "prompt is too long: 210000 tokens > 200000 maximum",
            "error code: context_length_exceeded",
            "input exceeds the context window",
        ] {
            assert!(
                context_overflow_advice(raw).is_some(),
                "not recognised: {raw}"
            );
        }
    }

    /// Anything that is not a context overflow is left exactly as it was — this
    /// must never become a catch-all that relabels unrelated failures.
    #[test]
    fn unrelated_errors_are_not_relabelled() {
        for raw in [
            "inference returned 401 Unauthorized",
            "tool_call 'web_fetch': URL is not allowed",
            "the conversation was cancelled",
            "",
        ] {
            assert_eq!(context_overflow_advice(raw), None, "wrongly claimed: {raw}");
        }
    }
}
