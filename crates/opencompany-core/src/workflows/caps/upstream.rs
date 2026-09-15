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
//! # Nothing this module emits grows with the number of sources
//!
//! Stated as a standing property because breaching it is how the bound broke the
//! first time (see [`bound_sections`]). Every string produced here is either
//! O(1) in the number of predecessors, or emitted a number of times that
//! [`max_rendered_sources`] bounds:
//!
//! | Emitted | How many | Size |
//! | --- | --- | --- |
//! | [`truncation_marker`] | ≤ [`max_rendered_sources`] per fold | ≤ [`MAX_MARKER_CHARS`], pinned by test |
//! | [`omitted_sources_marker`] | ≤ 1 per fold | ≤ [`MAX_MARKER_CHARS`], pinned by test |
//! | the [`clamp_to_budget`] marker | ≤ 1 per fold | a fixed literal |
//! | [`UpstreamReport::notice`] | 1 per agent node turn | fixed text + five numbers |
//!
//! The notice is the only one that leaves the turn (it goes to the run), and it
//! *aggregates* — one sentence however many sources were cut. Its per-run count
//! is bounded by the number of agent nodes in the graph, which is the same rate
//! the other [`RunNotices`](super::RunNotices) producers already push at.
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

/// The most characters [`truncation_marker`] and [`omitted_sources_marker`] can
/// produce, whatever numbers they are handed.
///
/// Pinned rather than measured at run time because it is *reserved* out of a
/// source's own allowance before the marker exists, and asserted against both
/// markers at their numeric extremes by
/// [`no_marker_can_exceed_its_reserved_size`](tests::no_marker_can_exceed_its_reserved_size)
/// — so a future edit that makes a marker wordier fails a test rather than
/// silently spending budget it was not given.
const MAX_MARKER_CHARS: usize = 320;

/// What separates one rendered source from the next.
const SECTION_SEPARATOR: &str = "\n\n---\n\n";

/// The least text a source must be able to carry for rendering it to be worth
/// anything.
///
/// Below this a "source" is a sentence fragment: it cannot be read, ranked or
/// quoted, and it still costs a separator and a truncation marker. Handing a
/// model a thousand 32-character shards is not a smaller version of the gather —
/// it is noise with the shape of data.
const MIN_SOURCE_SHARE_CHARS: usize = 1_024;

/// How many predecessors are worth rendering under `budget`, before each one's
/// fair share drops below [`MIN_SOURCE_SHARE_CHARS`].
///
/// At least one, always: a single source is rendered (and truncated) however
/// small the budget, because dropping the only input is never the better answer.
pub(super) fn max_rendered_sources(budget: usize) -> usize {
    (budget / MIN_SOURCE_SHARE_CHARS).max(1)
}

/// The characters the separators between `rendered` sections will cost.
fn separators_chars(rendered: usize) -> usize {
    rendered.saturating_sub(1) * SECTION_SEPARATOR.chars().count()
}

/// The single line standing in for the predecessors there was no room to render
/// at all.
///
/// **One line for all of them, deliberately.** This is the half of the accounting
/// that must not scale with the number of sources — the per-source marker below
/// is bounded by [`max_rendered_sources`], but the sources *past* that limit are
/// unbounded in principle (a `split_out` over a large array), so what stands in
/// for them has to be O(1). It still names the count, so "we did not show you
/// 970 sources" never reads to the model as "there were only 30".
fn omitted_sources_marker(omitted: usize, total: usize) -> String {
    format!(
        "\n\n[OMITTED BY OPENCOMPANY — {omitted} of this step's {total} inputs are not shown at \
         all. This step's input budget cannot carry a useful amount of each. They were NOT read. \
         Summarise the sources in earlier steps, or feed this step fewer of them.]"
    )
}

/// Clips a fully composed section to `budget`, marking it if it had to.
///
/// The **unconditional backstop**. Everything above aims to land inside the
/// budget by construction; this makes the guarantee hold whatever the arithmetic
/// above does, so the invariant is one assertion in one place rather than a
/// property a reader has to re-derive from three interacting caps. It should
/// never fire — [`the_backstop_is_never_what_enforces_the_budget`](tests::the_backstop_is_never_what_enforces_the_budget)
/// pins that it does not on the paths we know of — and it is here for the paths
/// we do not.
fn clamp_to_budget(section: String, budget: usize) -> String {
    if section.chars().count() <= budget {
        return section;
    }
    const CLAMPED: &str = "\n\n[TRUNCATED BY OPENCOMPANY — this step's whole input was clipped to its character \
         budget. The rest was NOT read.]";
    let marker_chars = CLAMPED.chars().count();
    // A budget too small to hold the marker gets a bare clip: a marker that
    // itself overflowed the budget would be the bug it exists to report.
    if budget <= marker_chars {
        return truncate_chars(&section, budget);
    }
    let mut clamped = truncate_chars(&section, budget - marker_chars);
    clamped.push_str(CLAMPED);
    clamped
}

/// Composes the upstream section from already-rendered predecessor texts, inside
/// `budget` — **markers, separators and all**.
///
/// # The guarantee
///
/// The returned string is never longer than `budget` characters. Not "the source
/// text is never longer": the whole section. That distinction is issue #849's own
/// bug reappearing inside its fix — the first cut of this code bounded source
/// text with [`allocate_fairly`] and then appended an *unbudgeted* marker per cut
/// source and an unbudgeted separator between each pair, so a thousand oversized
/// sources kept 32,000 characters of text and added roughly 216,000 characters of
/// our own accounting on top. A bound that the reporting of the bound can breach
/// is not a bound.
///
/// # How the accounting is paid for
///
/// Three mechanisms, each where it fits:
///
/// * **The separators and the omitted-sources line are charged up front**,
///   deducted from the budget before any source text is allocated. Both are known
///   before a single character is rendered.
/// * **Each truncation marker is charged to the source it describes**, reserved
///   out of that source's own allowance. This is the honest place for it: the
///   marker exists *because* that source was cut, and it stands in the space the
///   cut text vacated.
/// * **Sources past [`max_rendered_sources`] are aggregated into one line**
///   rather than rendered at all. Per-source markers can only be afforded while
///   their number is bounded, and this is what bounds it — a fan-in of a thousand
///   is not served by a thousand shards too small to read.
///
/// The per-source marker survives for the sources that *are* rendered because it
/// is doing work no aggregate can do: it sits at the exact point the text stops,
/// which is what stops a model describing a source it has only seen the opening
/// of. Aggregating it away would buy budget by removing the signal.
pub(super) fn bound_sections(rendered: &[String], budget: usize) -> (String, UpstreamReport) {
    let total = rendered.len();
    let render_count = total.min(max_rendered_sources(budget));
    let omitted = total - render_count;

    // Charged before any text: both are known up front, and neither is optional.
    let omitted_note = (omitted > 0).then(|| omitted_sources_marker(omitted, total));
    let fixed = separators_chars(render_count)
        + omitted_note
            .as_deref()
            .map_or(0, |note| note.chars().count());
    let text_budget = budget.saturating_sub(fixed);

    let sizes: Vec<usize> = rendered[..render_count]
        .iter()
        .map(|text| text.chars().count())
        .collect();
    let allowances = allocate_fairly(&sizes, text_budget);

    let mut sections = Vec::with_capacity(render_count);
    let mut sources = Vec::with_capacity(total);
    for (index, (&produced, &allowance)) in sizes.iter().zip(allowances.iter()).enumerate() {
        if allowance >= produced {
            sections.push(rendered[index].clone());
            sources.push(SourceBudget {
                produced,
                kept: produced,
            });
            continue;
        }
        // Cut, and said so in the turn itself: an agent that cannot tell it is
        // holding a fragment will present the fragment as the whole source. The
        // marker is reserved out of THIS source's allowance, so saying it costs
        // the budget nothing extra.
        let kept = allowance.saturating_sub(MAX_MARKER_CHARS);
        let mut section = truncate_chars(&rendered[index], kept);
        section.push_str(&truncation_marker(index + 1, total, produced, kept));
        sections.push(section);
        sources.push(SourceBudget { produced, kept });
    }
    // The sources there was no room for are still accounted for — at zero — so
    // the operator notice counts every input the step was handed, not only the
    // ones it managed to show.
    for text in &rendered[render_count..] {
        sources.push(SourceBudget {
            produced: text.chars().count(),
            kept: 0,
        });
    }

    let mut section = sections.join(SECTION_SEPARATOR);
    if let Some(note) = omitted_note {
        section.push_str(&note);
    }
    (
        clamp_to_budget(section, budget),
        UpstreamReport {
            sources,
            budget,
            omitted,
        },
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
    /// Per-source accounting, one row per predecessor the step was handed — in
    /// order, and including the ones there was no room to render, which carry
    /// `kept: 0`.
    pub(super) sources: Vec<SourceBudget>,
    /// The budget those sources were allocated out of.
    pub(super) budget: usize,
    /// How many predecessors were not rendered at all (the tail past
    /// [`max_rendered_sources`]). Reported separately from truncation because
    /// they are a different fact: a truncated source is present in the turn as a
    /// fragment, an omitted one is not present at all.
    pub(super) omitted: usize,
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
        // `cut` is the sources that are in the turn as fragments, `omitted` the
        // ones that are not in it at all — separated rather than double-reported.
        //
        // Counted over the rendered prefix directly rather than by subtracting
        // `omitted` from every `truncated()` row, because that subtraction
        // assumed every omitted row is also truncated. One that produced nothing
        // has `kept == produced`, so it is not truncated, and subtracting it
        // anyway under-counted `cut` — to zero in the two-source case, which took
        // the omission-only branch and never told the operator a rendered source
        // had been cut at all. `bound_sections` always appends the omitted rows
        // last, so the rendered ones are exactly the leading `sources - omitted`.
        let rendered = self.sources.len().saturating_sub(self.omitted);
        let cut = self.sources[..rendered]
            .iter()
            .filter(|source| source.truncated())
            .count();
        if cut == 0 && self.omitted == 0 {
            return None;
        }
        let sources = self.sources.len();
        let produced: usize = self.sources.iter().map(|s| s.produced).sum();
        let kept: usize = self.sources.iter().map(|s| s.kept).sum();
        let budget = self.budget;
        let plural = if sources == 1 { "" } else { "s" };
        // Both halves are stated whenever they happened, and each is only stated
        // when it did — an operator reading "0 were truncated" learns nothing and
        // trusts the next sentence less.
        let what = match (cut, self.omitted) {
            (0, omitted) => format!("{omitted} of them could not be shown to it at all"),
            (cut, 0) => {
                let verb = if cut == 1 { "was" } else { "were" };
                format!("{cut} of them {verb} truncated")
            }
            (cut, omitted) => format!(
                "{cut} of them were truncated and a further {omitted} could not be shown to it at \
                 all"
            ),
        };
        Some(format!(
            "A step in this workflow was handed {produced} characters from {sources} \
             source{plural} — more than the {budget} characters one step may pass to a model — so \
             {what}, and the step ran on the {kept} characters that fitted. Its result did not see \
             everything. To use the whole of each source, summarise each one in a step of its own \
             before this one, or have each source return less."
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
            omitted: 0,
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
            omitted: 0,
        };
        assert!(report.truncated_any());
        let notice = report.notice().expect("a cut fold speaks");
        assert!(notice.contains("40100 characters"), "{notice}");
        assert!(notice.contains("2 sources"), "{notice}");
        assert!(notice.contains("1 of them was truncated"), "{notice}");
        assert!(notice.contains("32000 characters"), "{notice}");
        assert!(notice.contains("summarise each one"), "{notice}");
    }

    /// An empty source in the omitted tail must not erase the truncation half of
    /// the notice.
    ///
    /// CodeRabbit on PR #851: `cut` was derived by subtracting `omitted` from the
    /// count of every `truncated()` row, which assumes every omitted row is also
    /// truncated. One that produced nothing has `kept == produced`, so it is
    /// *not* truncated, and subtracting it anyway under-counted `cut` — here to
    /// zero, which took the omission-only branch and left the operator never told
    /// that a rendered source had been cut.
    #[test]
    fn an_empty_omitted_source_does_not_hide_a_truncated_one() {
        let report = UpstreamReport {
            sources: vec![
                // Rendered, and cut.
                SourceBudget {
                    produced: 40_000,
                    kept: 31_900,
                },
                // Omitted, and produced nothing — so `truncated()` is false.
                SourceBudget {
                    produced: 0,
                    kept: 0,
                },
            ],
            budget: 32_000,
            omitted: 1,
        };
        let notice = report.notice().expect("a cut fold speaks");
        assert!(
            notice.contains("1 of them were truncated"),
            "the truncated rendered source is still reported: {notice}"
        );
        assert!(
            notice.contains("a further 1 could not be shown"),
            "the omission is still reported alongside it: {notice}"
        );
    }

    // ── The bound must hold for the reporting of the bound, too ──
    //
    // CodeRabbit on PR #851: the first cut of this module bounded source text and
    // then appended an *unbudgeted* marker per truncated source and an unbudgeted
    // separator between each pair. A thousand oversized sources therefore kept
    // 32,000 characters of text and added ~216,000 characters of our own
    // accounting — 7.7× the budget, which is issue #849's own failure reappearing
    // through its fix. These are the assertions whose absence let that through.

    /// `n` synthetic oversized sources.
    fn oversized_sources(n: usize, chars: usize) -> Vec<String> {
        (0..n)
            .map(|index| format!("SOURCE_{index} {}", "x".repeat(chars)))
            .collect()
    }

    /// The headline invariant, at the scale that broke it: the **whole rendered
    /// section** — text, markers, separators, omission line — is inside the
    /// budget.
    #[test]
    fn a_thousand_oversized_sources_stay_inside_the_budget() {
        let budget = DEFAULT_UPSTREAM_BUDGET_CHARS;
        let (section, report) = bound_sections(&oversized_sources(1_000, 200), budget);
        assert!(
            section.chars().count() <= budget,
            "the section must be bounded including its own accounting: {} characters for a \
             {budget}-character budget",
            section.chars().count()
        );
        // Every input is still accounted for, even the ones with no room.
        assert_eq!(report.sources.len(), 1_000);
        assert!(report.omitted > 0, "the tail is omitted, not silently kept");
        let notice = report.notice().expect("the operator is told");
        assert!(notice.contains("1000 sources"), "{notice}");
        assert!(notice.contains("could not be shown"), "{notice}");
    }

    /// The same at absurd scale, and with a tiny budget — the two directions that
    /// break a cap derived by division.
    #[test]
    fn the_bound_holds_at_every_scale_and_budget() {
        for count in [1usize, 2, 31, 32, 999, 5_000] {
            for budget in [
                0usize,
                1,
                120,
                1_023,
                1_024,
                5_000,
                DEFAULT_UPSTREAM_BUDGET_CHARS,
            ] {
                let (section, _) = bound_sections(&oversized_sources(count, 400), budget);
                assert!(
                    section.chars().count() <= budget,
                    "{count} sources under a {budget}-character budget produced {} characters",
                    section.chars().count()
                );
            }
        }
    }

    /// A source too small to be worth reading is not rendered as a shard — it is
    /// aggregated into one line that names how many were left out, so the count
    /// cannot scale with the number of sources.
    #[test]
    fn sources_past_the_useful_limit_are_aggregated_into_one_line() {
        let budget = DEFAULT_UPSTREAM_BUDGET_CHARS;
        assert_eq!(max_rendered_sources(budget), 31);
        let (section, report) = bound_sections(&oversized_sources(500, 5_000), budget);
        assert_eq!(report.omitted, 500 - 31);
        assert_eq!(
            section.matches("OMITTED BY OPENCOMPANY").count(),
            1,
            "one line for all of them, never one per source"
        );
        assert!(
            section.contains("469 of this step's 500 inputs"),
            "{}",
            &section[section.len().saturating_sub(400)..]
        );
        assert!(
            section.matches("TRUNCATED BY OPENCOMPANY").count() <= 31,
            "per-source markers are bounded by the render limit"
        );
    }

    /// The marker is reserved out of the source it describes, so saying "this was
    /// cut" costs the budget nothing extra — and a source still keeps a readable
    /// amount of text after paying for its own marker.
    #[test]
    fn a_truncation_marker_is_paid_for_by_its_own_source() {
        let budget = 8_000;
        let (section, report) = bound_sections(&oversized_sources(4, 100_000), budget);
        assert!(section.chars().count() <= budget);
        for source in &report.sources {
            assert!(
                source.kept >= MIN_SOURCE_SHARE_CHARS.saturating_sub(MAX_MARKER_CHARS) / 2,
                "a rendered source keeps a readable amount after its marker: {source:?}"
            );
        }
    }

    /// Neither marker may outgrow the space reserved for it, at any numbers. A
    /// wordier marker in a future edit fails here rather than quietly spending
    /// budget it was not given.
    #[test]
    fn no_marker_can_exceed_its_reserved_size() {
        let extremes = [
            truncation_marker(usize::MAX, usize::MAX, usize::MAX, usize::MAX - 1),
            truncation_marker(usize::MAX, usize::MAX, usize::MAX, 0),
            truncation_marker(1, 1, 1, 0),
            omitted_sources_marker(usize::MAX, usize::MAX),
            omitted_sources_marker(1, 2),
        ];
        for marker in extremes {
            assert!(
                marker.chars().count() <= MAX_MARKER_CHARS,
                "marker is {} characters, over the {MAX_MARKER_CHARS} reserved: {marker}",
                marker.chars().count()
            );
        }
    }

    /// The backstop is a backstop. If it is what enforces the budget on an
    /// ordinary path, the arithmetic above it is wrong and this says so — a clamp
    /// that fires routinely would be hiding a mis-accounting rather than guarding
    /// against one.
    #[test]
    fn the_backstop_is_never_what_enforces_the_budget() {
        for count in [1usize, 3, 31, 500] {
            for chars in [10usize, 5_000, 200_000] {
                let (section, _) = bound_sections(
                    &oversized_sources(count, chars),
                    DEFAULT_UPSTREAM_BUDGET_CHARS,
                );
                assert!(
                    !section.contains("whole input was clipped"),
                    "the clamp fired for {count} sources of {chars} characters, so the budgeting \
                     above it did not add up"
                );
            }
        }
    }

    /// And the common case is still untouched: sources that fit are rendered
    /// verbatim, joined by the same separator, with no marker and no notice.
    #[test]
    fn an_ordinary_fold_is_unchanged_by_any_of_this() {
        let rendered = vec![
            "Predecessor A: market is up.".to_string(),
            "Predecessor B: sentiment is positive.".to_string(),
        ];
        let (section, report) = bound_sections(&rendered, DEFAULT_UPSTREAM_BUDGET_CHARS);
        assert_eq!(section, rendered.join(SECTION_SEPARATOR));
        assert!(!report.truncated_any());
        assert_eq!(report.omitted, 0);
        assert_eq!(report.notice(), None);
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
