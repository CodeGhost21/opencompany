//! Per-call judgement: which calls stop for a human, beyond what the static
//! configuration says (issue #338, spine epic #183 §4).
//!
//! Specified in `docs/spec/company-brain/per-call-judgement.md`.
//!
//! # The gap this fills
//!
//! Whether a call stops is answered entirely by configuration decided before
//! the run starts: reserved `never_do`, the `readonly` brake, a live grant,
//! `always_approve`, the daily cap, `auto_approve_under_usd`, then the mode.
//! Nothing looks at what the run is about to do.
//!
//! That is a bad trade in both directions. Set the bar low and every run stops
//! constantly, which is the batching pressure #183 decision 3 exists to avoid.
//! Set it high — `full` — and an agent-picked call can send, pay, delete or
//! otherwise reach beyond a boundary this layer can see without asking anyone.
//! Publishing is the deliberate #658 exception described below.
//!
//! `full`'s stated contract is "the agents act without asking, **except the few
//! things on the always-ask list**". Without this module that exception is
//! whatever one operator remembered to type into `always_approve`. With it, the
//! irreversible acts stop on their own merits.
//!
//! # This can only ever ADD a stop
//!
//! The single most important property here, and the one the tests pin. This is
//! consulted **only where the rest of the chain has already decided to allow**,
//! and its only possible answers are "stop" and "say nothing". It cannot
//! un-deny a `readonly` refusal, cannot skip `always_approve`, cannot spend a
//! grant, and cannot soften the reserved `never_do` slot. Static configuration
//! stays authoritative everywhere it speaks; this speaks only in the silence
//! after it.
//!
//! The invariant, phrased so it survives the next tier: **this arm only ever
//! speaks where the mode allowed.** A tier that already parks or denies a call
//! keeps its own answer, whatever the tier is called and however many there
//! are.
//!
//! Which tiers that leaves the arm speaking on is deliberately **not
//! enumerated** — not here, not in [`crate::harness::policy`], and not in the
//! spec. A prose tier list is a copy to miss the next time a tier lands, and
//! `auto` (issue #560) landing mid-review is the proof that they land. The
//! question is answered instead by the one copy that cannot go stale:
//! `the_arm_adds_nothing_under_auto` walks **every declared tool** and asserts
//! this arm is silent on each one a tier still allows — so a new tier or a new
//! declaration fails a test rather than outdating a paragraph.
//!
//! # Where the classification lives
//!
//! The issue asks whether this is a property of the tool definition or a runtime
//! judgement per candidate call. It is a **hybrid**, and deliberately not the
//! third option:
//!
//! - **Declared**, from [`crate::policy::consequence`]'s table — the same
//!   declaration the approval card and the standing-grant rule already read. No
//!   new column and no second taxonomy: a second list would drift against the
//!   first, and the drift would be silent in the safe-looking direction.
//! - **Runtime**, from the arguments of the actual call — a `composio_execute`
//!   slug that names a send, or a declared amount of money. Deterministic
//!   inspection, not inference.
//! - **Not a model call.** A classifier LLM on the approval path costs money and
//!   latency per candidate call, is itself an external effect being made to
//!   decide whether external effects are allowed, and — fatally — returns a
//!   different verdict on different days. "Why did it stop?" must have an
//!   answer that can be re-derived from the record months later, which #242's
//!   trace exists to make possible. A sampled verdict cannot be audited.
//!
//! # It does not learn
//!
//! Firmly, and not as an oversight. If an operator approves the same shape of
//! call five times, the sixth still stops. Consent is an operator writing a
//! rule — that is issue #563, and a rule is a thing they can read, revoke and
//! be shown. A classifier noticing a habit is not consent: nobody agreed to it,
//! it cannot be pointed at, and it converts a security boundary into a
//! frequency count. The two mechanisms must not converge, so this one holds no
//! state across calls at all.
//!
//! That is also why **novelty is not a signal here**, though #338 lists it.
//! Every use of "we have seen this before" makes the gate *looser*, and a
//! looser second call is precisely what #243's single-use grant already governs:
//! the operator approves one call, redeeming it consumes it, and the next
//! identical call parks again. A novelty rule that skipped the second stop would
//! quietly repeal that. It is recorded as a signal on the trace instead, where
//! it informs a human without deciding anything.
//!
//! # Which path a call arrives on decides whether this speaks at all
//!
//! Issue #674's ruling, and the reason this module takes a [`CallPath`] rather
//! than judging every call the same way.
//!
//! A workflow `tool_call` node is **refused at author time** unless its
//! namespace is one the workflow invoker wires *and* the company's
//! `[tools].allow` grants it. So a saved `shell` node has already passed **two
//! operator gates** — the manifest grant, then authoring — and the operator has
//! seen the actual command they saved.
//!
//! An agent turn has passed **neither**. The model picks the tool and the
//! arguments at run time. That is a real difference in what the operator
//! consented to, which is why `full` meaning "do not ask me about what I
//! authored" is coherent while "do not ask me about anything the model decides
//! to run" is not.
//!
//! So: **#338's rules govern [`CallPath::Agent`]. #614's position governs
//! [`CallPath::AuthoredWorkflowNode`]** — on that path this module is silent,
//! and the operator's controls are the two gates above plus `always_approve`,
//! which still gates a node the tier would allow.
//!
//! ## The boundary condition, without which the split is a hole
//!
//! **A node whose arguments are templated from an upstream node's output is not
//! pre-declared**, and follows the *agent* rule. The operator declared the
//! *shape*; the *content* arrives at run time from data they never saw.
//!
//! Without this the split is trivially defeated: author one `shell` node whose
//! `command` is `=previous.output` and the two-gate argument stops describing
//! what actually runs. See [`any_argument_is_templated`].
//!
//! # What this deliberately does NOT stop
//!
//! #338's acceptance says "send, publish, pay, or delete". Read against the
//! split above: on the agent path **send, pay and delete are gated and publish
//! is not**; on the authored-node path none of the four are gated here unless
//! the boundary condition returns the node to the agent rule.
//!
//! `publish_artifact` is carved out on **both** paths, permanently — see
//! `DEFERRED` and issue #658.
//!
//! `shell` stays gated on the agent path, which is what keeps "or delete" real:
//! destruction has no `EffectGroup` of its own, and `shell` is how a run
//! deletes.
//!
//! # Failure is a stop
//!
//! There is no "unclassified, so allow" path. A tool nobody declared stops
//! ([`StopReason::Undeclared`]) unless `consequence_of` can see from its name
//! that it only reads, and a Composio slug nobody classified is already treated
//! as a send by the catalogue's own cautious fallback.
//!
//! The undeclared case gets its **own** reason rather than borrowing the group
//! one, because for an undeclared tool the group is a guess made from words in
//! the name. That guess is good enough to label a card and not good enough to
//! justify a stop to an operator: `write_file` contains "file", so the guess is
//! `Sign`, and "signs or files a document" is a confident false statement about
//! a file write. The call stops either way; only the sentence differs, and the
//! sentence is what a human reads.

use serde_json::Value;

use crate::policy::consequence::{Consequence, Reach, consequence_of, declared_tools};
use crate::ports::types::EffectGroup;

/// Why a call stopped, in the operator's terms.
///
/// Carried into the reason string on the parked effect, so the answer to "why
/// did this stop?" is on the approval card and in the attempt's trace rather
/// than only in this module's head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The tool's declared consequence class is one the company cannot take
    /// back: spending, sending, signing, publishing, hiring, or touching the
    /// company's identity.
    Irreversible(EffectGroup),
    /// The call changes state or reaches out, and what it can reach is not
    /// bounded by anything this layer can see — arbitrary code, an arbitrary
    /// address, or a saved workflow that performs whatever it performs.
    UnboundedReach,
    /// The arguments declare money leaving the company.
    MoneyLeaves,
    /// Nobody declared this tool, so what it does cannot be established here.
    ///
    /// The fail-closed case, and its own reason rather than a borrowed one. A
    /// tool absent from the table still gets a `group` — [`consequence_of`]
    /// falls back to matching *words in the name*, which exists so an approval
    /// card for an unknown tool is still labelled. That heuristic is fine for a
    /// label and wrong for a verdict: `write_file` contains "file", so it is
    /// labelled `Sign`, and reporting "signs or files a document" to an operator
    /// about a file write is a confident false statement. Saying "this is not
    /// declared" is the true one, and it stops the call just the same.
    Undeclared,
}

impl StopReason {
    /// The operator-facing half of the reason string.
    ///
    /// Deliberately says what the *call* does, not which rule fired: an operator
    /// deciding whether to approve needs the consequence, not this module's
    /// name. The tool name is prepended by the caller.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Irreversible(EffectGroup::Spend) => "spends money, which cannot be taken back",
            Self::Irreversible(EffectGroup::Send) => {
                "sends to a counterparty, which cannot be taken back"
            }
            Self::Irreversible(EffectGroup::Sign) => {
                "signs or files a document, which cannot be taken back"
            }
            Self::Irreversible(EffectGroup::Publish) => {
                "publishes externally, which cannot be taken back"
            }
            Self::Irreversible(EffectGroup::Hire) => {
                "hires or contracts, which cannot be taken back"
            }
            Self::Irreversible(EffectGroup::Identity) => {
                "changes the company's identity, which cannot be taken back"
            }
            // Unreachable via `judge` — `Other` is the unclassified bucket and
            // never produces `Irreversible` — but stated rather than
            // `unreachable!()`, because a panic on the approval path would be a
            // denial of service triggered by a tool name.
            Self::Irreversible(EffectGroup::Other) => "has a consequence that cannot be taken back",
            Self::UnboundedReach => {
                "can run arbitrary code or reach an arbitrary address, so what it \
                 changes cannot be bounded from here"
            }
            Self::MoneyLeaves => "moves money out of the company",
            Self::Undeclared => {
                "is not a declared tool, so what it does cannot be established here"
            }
        }
    }
}

/// The verdict for one candidate call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judgement {
    /// This module has nothing to add; whatever the chain decided stands.
    Silent,
    /// Stop for a human, whatever the mode says.
    Stop(StopReason),
}

impl Judgement {
    /// The reason this stopped, if it did.
    pub fn stop_reason(self) -> Option<StopReason> {
        match self {
            Self::Stop(reason) => Some(reason),
            Self::Silent => None,
        }
    }
}

/// Is this consequence class one the company cannot take back?
///
/// Every named class is. [`EffectGroup::Other`] is the *unclassified* bucket
/// rather than a "harmless" one — it is where a tool lands when the classifier
/// found no particular consequence to name — so it is decided by reach below
/// instead of being waved through here.
fn is_irreversible_group(group: EffectGroup) -> bool {
    match group {
        EffectGroup::Spend
        | EffectGroup::Send
        | EffectGroup::Sign
        | EffectGroup::Publish
        | EffectGroup::Hire
        | EffectGroup::Identity => true,
        EffectGroup::Other => false,
    }
}

/// The tools whose reach this layer cannot bound: arbitrary code, an arbitrary
/// address, or a saved workflow that performs whatever it performs.
///
/// # Why this is a list and not a derivation
///
/// It was a derivation, and the derivation was wrong. `Reach::Consequence` +
/// `Standing::PerCall` looked like it named exactly this set — issue #444
/// refuses a standing grant to everything here — so it was tempting to read
/// "an operator cannot consent to this for a week" as the same judgement as
/// "an operator should see this once".
///
/// It is not. `Standing::PerCall` is refused for **two** different reasons that
/// the flag does not distinguish: a tool can be unbounded (`shell`), or it can
/// be bounded but overwrite state the operator authored (`workspace_write`).
/// Only the first warrants stopping a call that changes nothing outside the
/// company. Deriving the set from the flag swept up `workspace_write` and
/// `workspace_create`, which is how the company publishes to its own note
/// tree — and stopping those broke publishing outright: thirteen of the
/// `publish_turn_test` cases as the suite then stood went from passing to "the
/// model was never handed a publish receipt", because a parked call is a call
/// that never happens.
///
/// So the set is named. `every_unbounded_tool_is_declared` keeps it honest: a
/// rename in the declaration table fails that test rather than silently
/// dropping a tool out of this list.
const UNBOUNDED: &[&str] = &[
    // Arbitrary code, in a sandbox whose permissions this layer cannot see.
    // Also the way a run *deletes*: destruction has no `EffectGroup` of its
    // own, so this is what makes #338's "or delete" acceptance real.
    "shell",
    // An arbitrary address, with the tenant's own credentials.
    "curl",
    "http_request",
    "web_fetch",
    // Can push to a configured remote — an address this layer does not see.
    // The declaration table already singles it out for that, refusing it the
    // standing grant its filesystem siblings get.
    "git_operations",
    // Performs whatever the saved workflow performs, which is not visible here.
    "run_workflow",
];

/// Does this call change things without a bound this layer can see?
///
/// The [`Reach::Consequence`] half still matters: it is what keeps a metered
/// read out. `web_search` is [`Reach::Money`] — nothing changes and nothing
/// leaves — and parking it would be worse than useless, because openhuman
/// resolves a `RequireApproval` inline, so a parked search is a search that
/// never happens and an agent with no search invents citations.
fn is_unbounded(tool: &str, consequence: Consequence) -> bool {
    consequence.reach == Reach::Consequence && UNBOUNDED.contains(&tool)
}

/// Tools this gate never speaks about, on either path.
///
/// **This is a deliberate exclusion, not an oversight, and it is not
/// provisional. Do not delete it without reading issue #658.**
///
/// `publish_artifact` is declared `EffectGroup::Publish` + `Reach::Consequence`,
/// under a table comment reading "Externally visible and not reversible by the
/// company alone" — so by rule 1 below it qualifies, and #338's acceptance names
/// "publish" outright. It is excluded anyway, and **#658 has now ruled that this
/// is the correct behaviour rather than a stopgap**: under `full` a company
/// publishes without asking, and `always_approve` is the operator's override for
/// companies that want to be asked.
///
/// The argument #658 settled: this gate has exactly one escape — a single-use,
/// argument-exact grant per call (#243). There is no manifest knob. So stopping
/// `publish_artifact` here would mean **every company running `full` stops for a
/// human on every deliverable it publishes**, permanently, with no way to opt
/// out short of editing the declaration table. That is not a call to make as a
/// side effect of adding a classifier, and #658 declined to make it: an operator
/// who wants publishing gated names it in `always_approve`, which is read before
/// this arm and is a thing they can see, change and revoke.
///
/// The size of the alternative is visible in the tests: excluding it keeps all
/// 19 `publish_turn_test` cases green, and gating it would need a grant minted
/// per scripted call in a suite that is about publish mechanics, not approvals.
/// That count is re-checked against #659's merged fixtures rather than carried
/// forward — #659 landed first and touched this suite, so the evidence the
/// carve-out rests on was re-measured on top of it, not assumed.
///
/// `http_request`, `curl` and `web_fetch` **were** on this list pending #674 and
/// are not deferred any more. #674 ruled that they are governed by #614 on the
/// authored-node path and by #338 on the agent path — which is a scoping of the
/// rule, not an exclusion from it, so it is expressed by [`CallPath`] rather
/// than here. See the module docs.
const DEFERRED: &[&str] = &[
    // Issue #658 — see the note above. Ruled, not pending.
    "publish_artifact",
];

/// The prefix tinyflows gives an expression bound at run time.
///
/// Both binding forms wear it: the dotted shorthand (`=item.brief`) and a jq
/// program (`=.items | length`). `t_transform_resolves_expr_bindings_engine_side`
/// in [`crate::workflows::runner`] pins that they resolve engine-side, and
/// `every_reachable_workflow_tool_is_classified_by_name_alone` in
/// [`crate::workflows::gate`] pins that a node's args may still be carrying them
/// unresolved when the gate pass runs — which is exactly the window this reads.
const EXPRESSION_PREFIX: &str = "=";

/// Which of the two paths a candidate call arrived on.
///
/// Issue #674's ruling made this the first thing [`judge`] asks. The variants
/// are not two flavours of caller; they are two different things an operator
/// consented to. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPath {
    /// A model picked this tool and these arguments during a turn. Nobody saw
    /// the call before it was made, so #338's rules apply in full.
    Agent,
    /// An operator authored this node into a saved workflow, past the manifest
    /// grant and the authoring refusal. #614's position applies: this module is
    /// silent — **unless the arguments are templated**, which un-declares the
    /// call and returns it to the agent rule.
    AuthoredWorkflowNode,
}

/// Does any argument of this call arrive from the run rather than from the
/// author?
///
/// **The boundary condition of #674's split, and the part that stops it being a
/// hole.** The authored-node path is silent because an operator saw the call.
/// They did not see `=previous.output`; they saw the *shape*, and the content
/// arrives at run time from data they never read. So a templated node is judged
/// as an agent call.
///
/// Without this, one `shell` node whose `command` is `=previous.output` defeats
/// the split outright: the two-gate argument stops describing what actually runs
/// while still being cited as the reason not to ask.
///
/// Recurses through objects and arrays, because an expression is a *value*
/// anywhere in the descriptor, not a top-level key — `{"body": {"cmd":
/// "=item.x"}}` is as templated as `{"cmd": "=item.x"}`.
///
/// Errs toward "templated": a literal string that merely starts with `=` is read
/// as an expression, which sends the call to the stricter rule. That is the
/// direction to be wrong in, and tinyflows reads it that way too.
fn any_argument_is_templated(args: &Value) -> bool {
    match args {
        Value::String(text) => text.starts_with(EXPRESSION_PREFIX),
        Value::Array(items) => items.iter().any(any_argument_is_templated),
        Value::Object(fields) => fields.values().any(any_argument_is_templated),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// The argument key a declared amount of money arrives under.
///
/// Mirrors the harness policy's own reader, which is where the spend arms
/// already look. Kept in sync by `amount_key_matches_the_harness_reader`.
const AMOUNT_KEY: &str = "amount_usd";

/// Money named in the arguments, when it is a positive number.
///
/// Zero and negative are not "money leaving": a zero-amount call moves nothing,
/// and a negative one is a refund shape this layer has no business guessing at.
fn declared_amount_usd(args: &Value) -> Option<f64> {
    args.get(AMOUNT_KEY)
        .and_then(Value::as_f64)
        .filter(|amount| *amount > 0.0)
}

/// Should this call stop for a human on its own merits?
///
/// Pure, total and deterministic: the same call judged twice gives the same
/// answer, which is what makes a stop explainable from the trace long after the
/// run. Consulted only where the rest of the chain has already decided to
/// allow — see the module docs for why that placement is the whole safety
/// argument.
///
/// `path` decides whether this speaks at all (issue #674). It is a required
/// argument rather than a defaulted one on purpose: a caller that has not
/// thought about which path it is on should not compile.
pub fn judge(tool: &str, args: &Value, path: CallPath) -> Judgement {
    // Issue #674's split, FIRST — before any rule can reach the call.
    //
    // An authored node passed two operator gates (the manifest grant, then
    // authoring) and the operator saw the call. An agent turn passed neither.
    //
    // The `!any_argument_is_templated` half is the boundary condition, not a
    // refinement: an authored node whose arguments come from an upstream node's
    // output was never pre-declared — the operator declared the shape, the
    // content arrives at run time — so it is judged as an agent call. Deleting
    // that half leaves a split that one `=previous.output` defeats.
    if path == CallPath::AuthoredWorkflowNode && !any_argument_is_templated(args) {
        return Judgement::Silent;
    }

    let consequence = consequence_of(tool, args);
    let name = tool.to_ascii_lowercase();
    let declared = declared_tools().any(|d| d == name);

    // Carved out on both paths — see `DEFERRED` and issue #658. Ahead of every
    // rule, so none of them can reach it and so the exclusion is impossible to
    // miss.
    if DEFERRED.contains(&name.as_str()) {
        return Judgement::Silent;
    }

    // Declared first: a named consequence class is the honest answer and the
    // one the operator's card already shows.
    //
    // Restricted to DECLARED tools, which is not a detail. An undeclared tool
    // is given a group by matching words in its name — a fallback that exists
    // to label a card, not to decide one. `write_file` contains "file" and is
    // therefore labelled `Sign`; gating on that would stop it while telling the
    // operator it "signs or files a document". Undeclared tools stop below, on
    // the honest ground that nobody has said what they do.
    //
    // Both halves are required, and the pairing is not obvious. The group names
    // *what kind* of consequence a tool has; the reach says whether this call
    // actually has one. `web_search` and the `media_generate_*` tools are
    // declared `Spend` because the backend bills per request, but their reach is
    // `Money`: nothing changes anywhere and nothing leaves the company — the
    // money buys the call itself. Stopping them on the group alone parked every
    // search, which is worse than useless: openhuman resolves a
    // `RequireApproval` inline, so a parked search is a search that never
    // happens and an agent with no search invents citations. The per-company
    // daily cap is the boundary for that spend, and it has already had its say
    // above.
    if declared
        && is_irreversible_group(consequence.group)
        && consequence.reach == Reach::Consequence
    {
        return Judgement::Stop(StopReason::Irreversible(consequence.group));
    }

    // Then the arguments of this actual call. A tool the table calls
    // unclassified can still be carrying money.
    if declared_amount_usd(args).is_some() {
        return Judgement::Stop(StopReason::MoneyLeaves);
    }

    // Fail closed on anything nobody declared that is not a pure read. The
    // read carve-out is what keeps fail-closed from collapsing into
    // stop-everything: `consequence_of` already answers `Reach::Nothing` for a
    // name that reads, and an unknown read changes nothing by definition.
    if !declared && consequence.reach != Reach::Nothing {
        return Judgement::Stop(StopReason::Undeclared);
    }

    // Then reach: declared, unclassified, but able to do anything.
    if is_unbounded(&name, consequence) {
        return Judgement::Stop(StopReason::UnboundedReach);
    }

    Judgement::Silent
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every test below that does not name a path is about the **agent** path —
    /// the one #338's rules govern. The authored-node path has its own section
    /// at the end of this module.
    fn judge_bare(tool: &str) -> Judgement {
        judge_agent(tool, &json!({}))
    }

    fn judge_agent(tool: &str, args: &Value) -> Judgement {
        judge(tool, args, CallPath::Agent)
    }

    fn judge_node(tool: &str, args: &Value) -> Judgement {
        judge(tool, args, CallPath::AuthoredWorkflowNode)
    }

    /// The agent-path half of #338's acceptance after #658's ruling: sends,
    /// payments and deletes stop, and an undeclared publish-shaped call fails
    /// closed. The declared `publish_artifact` exception is pinned separately.
    ///
    /// `delete` has no declared [`EffectGroup`] of its own — the taxonomy names
    /// six consequence classes and destruction is not one — so it arrives here
    /// as the unbounded-reach case. `shell` is how a run deletes.
    #[test]
    fn agent_calls_that_send_pay_delete_or_use_an_undeclared_publish_shape_stop() {
        // Declared, so the verdict names the consequence the table declares.
        assert_eq!(
            judge_bare("media_generate_image"),
            Judgement::Stop(StopReason::Irreversible(EffectGroup::Spend)),
        );
        assert_eq!(
            judge_agent("composio_execute", &json!({ "tool": "GMAIL_SEND_EMAIL" })),
            Judgement::Stop(StopReason::Irreversible(EffectGroup::Send)),
        );
        // Undeclared, so they stop on the fail-closed ground instead — see
        // `an_undeclared_tool_stops_without_borrowing_a_guessed_reason`.
        for tool in ["send_email", "publish_post", "pay_invoice"] {
            assert!(
                matches!(judge_bare(tool), Judgement::Stop(_)),
                "`{tool}` must stop"
            );
        }
        assert_eq!(
            judge_bare("shell"),
            Judgement::Stop(StopReason::UnboundedReach),
            "deleting is reached through arbitrary code, not a declared group"
        );
    }

    /// The other half of the acceptance, and the half that makes the feature
    /// usable rather than merely safe.
    #[test]
    fn reads_searches_and_drafts_do_not_stop() {
        for tool in [
            "file_read",
            "glob",
            "grep",
            "list",
            "read_workspace_state",
            "memory_recall",
            "image_info",
            "query_company",
        ] {
            assert_eq!(judge_bare(tool), Judgement::Silent, "`{tool}` reads");
        }
    }

    /// The agent's own scratch space. These mutate, so any tier that parks or
    /// denies a mutating call keeps its own answer — but they are the
    /// low-consequence writes #444 found safe enough to hand over for a week,
    /// so the judgement arm adds no stop of its own where a tier allowed them.
    #[test]
    fn the_agents_own_drafts_do_not_stop() {
        for tool in [
            "file_write",
            "edit",
            "apply_patch",
            "csv_export",
            "memory_store",
            "memory_forget",
        ] {
            assert_eq!(judge_bare(tool), Judgement::Silent, "`{tool}` is a draft");
        }
    }

    /// A metered call must never stop here: openhuman resolves a
    /// `RequireApproval` inline, so a parked search is a search that never
    /// happens.
    ///
    /// This is the sharp edge of the whole module, and the pair below is why
    /// the rule reads reach as well as group.
    ///
    /// Both are declared [`EffectGroup::Spend`], so a rule keyed on the group
    /// alone parked every search in the company. They differ in reach, and the
    /// reach is the part that matters: `web_search` is [`Reach::Money`] — the
    /// spend buys the call, nothing changes and nothing leaves, and the daily
    /// cap governs it — while media generation is [`Reach::Consequence`],
    /// because it "moves real money on submit" (issue #109).
    #[test]
    fn a_metered_read_never_stops_but_a_real_spend_does() {
        for tool in ["web_search", "media_generate_image", "media_generate_video"] {
            assert_eq!(
                consequence_of(tool, &json!({})).group,
                EffectGroup::Spend,
                "`{tool}` is declared Spend — that is the trap this test guards"
            );
        }
        assert_eq!(
            judge_bare("web_search"),
            Judgement::Silent,
            "billed, but nothing changes and nothing leaves"
        );
        for tool in ["media_generate_image", "media_generate_video"] {
            assert_eq!(
                judge_bare(tool),
                Judgement::Stop(StopReason::Irreversible(EffectGroup::Spend)),
                "`{tool}` moves real money on submit"
            );
        }
    }

    /// Arbitrary code and arbitrary addresses, which the table already refuses
    /// a standing grant for the same reason.
    ///
    /// The outward-HTTP family — `http_request`, `curl`, `web_fetch` — is
    /// asserted here rather than carved out. Before #674 it sat in `DEFERRED`
    /// and this loop skipped it; the ruling put it back under the rule **on this
    /// path**, and under #614's position on the authored-node path.
    #[test]
    fn unbounded_tools_stop() {
        for tool in ["http_request", "curl", "web_fetch"] {
            assert!(
                !DEFERRED.contains(&tool),
                "#674 ruled `{tool}` scoped by path, not deferred"
            );
        }
        // The filter is retained rather than dropped: nothing in `UNBOUNDED` is
        // deferred today, but `DEFERRED` is what is withheld from the rule and
        // this test is about the rule. The carve-outs have their own tests.
        for tool in UNBOUNDED.iter().filter(|t| !DEFERRED.contains(t)) {
            assert_eq!(
                judge_bare(tool),
                Judgement::Stop(StopReason::UnboundedReach),
                "`{tool}` is unbounded"
            );
        }
    }

    /// The company writing to its own note tree is not an unbounded act, and
    /// stopping it broke publishing (13 `publish_turn_test` cases at the
    /// time; the suite now stands at 19, all green). They are
    /// `Standing::PerCall` — issue #444 refuses them a standing grant because
    /// they overwrite guidance the operator authored — and that is a different
    /// reason from "what this reaches cannot be bounded". A tier that parks or
    /// denies them keeps its own answer; this arm adds nothing on top.
    #[test]
    fn writing_the_companys_own_notes_is_not_unbounded() {
        for tool in ["workspace_write", "workspace_create"] {
            assert!(
                !consequence_of(tool, &json!({})).standing.is_grantable(),
                "`{tool}` is PerCall — the trap this test guards"
            );
            assert_eq!(judge_bare(tool), Judgement::Silent, "`{tool}` is internal");
        }
    }

    /// The carve-out, pinned so it is a decision rather than a drift.
    ///
    /// `publish_artifact` satisfies rule 1 outright — declared `Publish` and
    /// `Consequence` — and is silent anyway. Issue #658 **ruled** that this is
    /// the behaviour it wants rather than a stopgap: under `full` a company
    /// publishes without asking, and an operator who wants to be asked names it
    /// in `always_approve`, which is read before this arm.
    ///
    /// Asserted on **both** paths: the carve-out is not part of #674's split, so
    /// a rework of that split must not accidentally make one path speak.
    #[test]
    fn publish_artifact_is_excluded_by_the_658_ruling() {
        let c = consequence_of("publish_artifact", &json!({}));
        assert_eq!(c.group, EffectGroup::Publish);
        assert_eq!(c.reach, Reach::Consequence);
        assert!(
            is_irreversible_group(c.group),
            "it qualifies — the exclusion is what keeps it silent"
        );
        for path in [CallPath::Agent, CallPath::AuthoredWorkflowNode] {
            assert_eq!(
                judge("publish_artifact", &json!({}), path),
                Judgement::Silent,
                "{path:?}: #658 ruled `full` publishes without asking; \
                 `always_approve` is the override"
            );
        }
        // Templated arguments return an authored node to the agent rule — and
        // the agent rule is silent here too, so the carve-out survives the one
        // condition that reopens every other node.
        assert_eq!(
            judge_node("publish_artifact", &json!({ "body": "=previous.output" })),
            Judgement::Silent,
        );
    }

    /// The arm adds nothing under `auto` (issue #560) — pinned as a rule over
    /// the whole declaration table, not as a snapshot of what it holds today.
    ///
    /// `auto` parks `parks_under_supervision() && !is_grantable()`, so the only
    /// calls it allows that `supervised` would park are the `Grantable` ones.
    /// Today every `d_grantable` row is `EffectGroup::Other`, none is in
    /// `UNBOUNDED` and none carries an amount, so no rule fires — but that is a
    /// property of the table's current contents, and nothing pinned it.
    ///
    /// The day somebody adds `d_grantable("send_invoice", EffectGroup::Send,
    /// Reach::Consequence)`, `auto` would shift silently under operators who
    /// chose it — exactly the failure the placement argument exists to prevent.
    /// This walks every declared tool and fails on that day instead.
    ///
    /// Judged on [`CallPath::Agent`] deliberately: that is the strict path, so
    /// this asserts the strongest form of the claim. If it held only on the
    /// authored-node path it would be asserting #674's split, not `auto`.
    #[test]
    fn the_arm_adds_nothing_under_auto() {
        let mut checked = 0;
        for tool in declared_tools() {
            let c = consequence_of(tool, &json!({}));
            if c.parks_under_auto() {
                continue; // `auto` already stops it; this arm is never reached.
            }
            checked += 1;
            assert_eq!(
                judge_agent(tool, &json!({})),
                Judgement::Silent,
                "`{tool}` runs under `auto` but this arm would stop it — `auto` \
                 has shifted under operators who chose it"
            );
        }
        assert!(
            checked > 0,
            "no declared tool runs under `auto` — the walk asserted nothing"
        );
    }

    /// Every carve-out must be a tool a rule would otherwise have stopped.
    ///
    /// The point of the list is to hold back calls this gate *would* gate. An
    /// entry that no rule fires on is not a deferral, it is a misunderstanding —
    /// and it would sit there looking like a decision. This fails if the
    /// declaration table moves under a carve-out, so a deferral cannot quietly
    /// become a no-op.
    ///
    /// It also fails if `DEFERRED` is emptied. Without that, deleting the
    /// `publish_artifact` carve-out would make this loop over nothing and pass —
    /// a test that scores green having asserted zero things, which is the
    /// failure mode it exists to prevent.
    ///
    /// `http_request` / `curl` / `web_fetch` were here and are not any more:
    /// #674 ruled them **scoped by path**, not deferred, so they are governed by
    /// `CallPath` and are asserted in the authored-node section below.
    #[test]
    fn every_deferred_tool_would_otherwise_be_stopped() {
        assert!(
            !DEFERRED.is_empty(),
            "DEFERRED is empty — this test would assert nothing. #658 ruled the \
             `publish_artifact` carve-out correct; read it before removing it"
        );
        for tool in DEFERRED {
            let c = consequence_of(tool, &json!({}));
            let would_stop = (is_irreversible_group(c.group) && c.reach == Reach::Consequence)
                || is_unbounded(tool, c);
            assert!(
                would_stop,
                "`{tool}` is deferred but no rule would stop it — the entry is inert"
            );
            assert_eq!(
                judge_bare(tool),
                Judgement::Silent,
                "`{tool}` is deferred; do not 'fix' this without reading #658"
            );
        }
    }

    /// Everything held back must be a declared tool, or the exclusion silently
    /// stops matching anything.
    #[test]
    fn every_deferred_tool_is_declared() {
        for tool in DEFERRED {
            assert!(
                declared_tools().any(|d| d == *tool),
                "`{tool}` is in DEFERRED but not declared"
            );
        }
    }

    /// A rename in the declaration table must fail loudly here rather than
    /// silently dropping a tool out of `UNBOUNDED`.
    #[test]
    fn every_unbounded_tool_is_declared() {
        for tool in UNBOUNDED {
            assert!(
                declared_tools().any(|d| d == *tool),
                "`{tool}` is in UNBOUNDED but not declared — it would never match"
            );
        }
    }

    /// Fail closed: a tool nobody declared stops rather than passing.
    #[test]
    fn an_undeclared_tool_stops() {
        assert_eq!(
            judge_bare("frobnicate_the_widget"),
            Judgement::Stop(StopReason::Undeclared)
        );
    }

    /// ...and it says so, rather than repeating a guess as if it were a fact.
    ///
    /// `write_file` is undeclared and `consequence_of` labels it `Sign`, purely
    /// because the name contains "file". Both verdicts stop the call; only one
    /// of them tells the operator something true. This is the regression guard
    /// for that: the reason must be the honest one, not the borrowed one.
    #[test]
    fn an_undeclared_tool_stops_without_borrowing_a_guessed_reason() {
        assert_eq!(
            consequence_of("write_file", &json!({})).group,
            EffectGroup::Sign,
            "the name heuristic guesses Sign — that is what must not be quoted"
        );
        assert_eq!(
            judge_bare("write_file"),
            Judgement::Stop(StopReason::Undeclared)
        );
    }

    /// ...but an undeclared tool that only reads still does not stop, so
    /// fail-closed does not collapse into stop-everything.
    #[test]
    fn an_undeclared_read_does_not_stop() {
        assert_eq!(judge_bare("get_the_weather"), Judgement::Silent);
    }

    /// A Composio slug nobody has classified is treated as a send by
    /// `consequence_of`, and a send stops.
    #[test]
    fn an_unclassified_composio_action_stops() {
        let args = json!({ "tool": "SOME_TOOLKIT_ACTION_NOBODY_CLASSIFIED" });
        assert_eq!(
            judge_agent("composio_execute", &args),
            Judgement::Stop(StopReason::Irreversible(EffectGroup::Send))
        );
    }

    /// Money named in the arguments stops even when the tool's own class is the
    /// unclassified bucket.
    #[test]
    fn a_declared_amount_stops_an_otherwise_silent_tool() {
        // `list` is a declared pure read: silent on its own.
        assert_eq!(judge_bare("list"), Judgement::Silent);
        assert_eq!(
            judge_agent("list", &json!({ "amount_usd": 12.5 })),
            Judgement::Stop(StopReason::MoneyLeaves)
        );
    }

    /// Zero is not money leaving, and neither is a negative.
    #[test]
    fn a_zero_or_negative_amount_is_not_money_leaving() {
        assert_eq!(
            judge_agent("list", &json!({ "amount_usd": 0 })),
            Judgement::Silent
        );
        assert_eq!(
            judge_agent("list", &json!({ "amount_usd": -5 })),
            Judgement::Silent
        );
    }

    /// The gate holds no state, so it cannot learn: the sixth identical call is
    /// judged exactly as the first. #563 is where consent lives, and consent is
    /// an operator writing a rule — not this noticing a habit.
    #[test]
    fn the_gate_does_not_learn_from_repetition() {
        let args = json!({ "to": "customer@example.com" });
        let first = judge_agent("send_email", &args);
        for _ in 0..5 {
            assert_eq!(
                judge_agent("send_email", &args),
                first,
                "repetition must not soften the verdict"
            );
        }
    }

    /// Same call, same answer — the property that makes a stop explainable from
    /// the trace months later, and the reason this is not a model call.
    #[test]
    fn the_verdict_is_deterministic() {
        let args = json!({ "body": "hello", "to": "a@example.com" });
        assert_eq!(
            judge_agent("send_email", &args),
            judge_agent("send_email", &args)
        );
    }

    /// Every stop can say why, in the operator's terms rather than this
    /// module's.
    #[test]
    fn every_stop_reason_describes_itself() {
        let groups = [
            EffectGroup::Spend,
            EffectGroup::Send,
            EffectGroup::Sign,
            EffectGroup::Publish,
            EffectGroup::Hire,
            EffectGroup::Identity,
            EffectGroup::Other,
        ];
        for group in groups {
            assert!(!StopReason::Irreversible(group).describe().is_empty());
        }
        assert!(!StopReason::UnboundedReach.describe().is_empty());
        assert!(!StopReason::MoneyLeaves.describe().is_empty());
        assert!(!StopReason::Undeclared.describe().is_empty());
    }

    // ---- #674: the split by path ------------------------------------------
    //
    // Everything above is the agent path. These are the authored-node path and
    // the boundary condition that governs when a node leaves it.

    /// #614's position, stated as a rule over the whole strict set rather than
    /// as the three tools whose tests forced the question.
    ///
    /// A node an operator authored has passed the manifest grant and the
    /// authoring refusal, and they saw the call. This module is silent on it.
    #[test]
    fn an_authored_node_with_literal_arguments_is_not_judged() {
        // Every tool the agent path stops for a reason other than a carve-out,
        // so this is the rule and not a sample of it.
        let cases: &[(&str, Value)] = &[
            ("shell", json!({ "command": "echo hello > out.txt" })),
            ("curl", json!({ "url": "https://api.example.com/x" })),
            (
                "http_request",
                json!({ "url": "https://api.example.com/x" }),
            ),
            ("web_fetch", json!({ "url": "https://example.com" })),
            ("git_operations", json!({ "op": "push" })),
            ("run_workflow", json!({ "workflow_id": "nightly" })),
            ("media_generate_image", json!({ "prompt": "a cat" })),
            (
                "composio_execute",
                json!({ "tool": "GMAIL_SEND_EMAIL", "to": "a@example.com" }),
            ),
            ("frobnicate_the_widget", json!({})),
            ("list", json!({ "amount_usd": 400.0 })),
        ];
        for (tool, args) in cases {
            assert!(
                matches!(judge_agent(tool, args), Judgement::Stop(_)),
                "`{tool}` must stop on the agent path — otherwise this case \
                 proves nothing about the split"
            );
            assert_eq!(
                judge_node(tool, args),
                Judgement::Silent,
                "`{tool}` was authored into a saved workflow: two operator gates \
                 and the operator saw the call (#674 / #614)"
            );
        }
    }

    /// **The boundary condition.** An authored node whose arguments come from an
    /// upstream node's output was never pre-declared, so it is judged as an
    /// agent call.
    ///
    /// This is the test that stops the split being a hole. Delete it and one
    /// `shell` node whose `command` is `=previous.output` walks through a gate
    /// justified by an operator having seen a command they never saw.
    #[test]
    fn an_authored_node_templated_from_upstream_output_follows_the_agent_rule() {
        assert_eq!(
            judge_node("shell", &json!({ "command": "=previous.output" })),
            Judgement::Stop(StopReason::UnboundedReach),
            "the operator declared the shape; the content arrives at run time"
        );
        // The same node with the command actually written out is silent — which
        // is what makes the assertion above about templating rather than about
        // `shell`.
        assert_eq!(
            judge_node("shell", &json!({ "command": "echo hi" })),
            Judgement::Silent,
        );
        // Money, and the jq binding form as well as the dotted shorthand.
        assert_eq!(
            judge_node(
                "list",
                &json!({ "amount_usd": 400.0, "note": "=.items[0]" })
            ),
            Judgement::Stop(StopReason::MoneyLeaves),
        );
    }

    /// An expression is a *value* anywhere in the descriptor, not a top-level
    /// key — so the check recurses. A node that buried `=previous.output` one
    /// object deep would otherwise read as fully authored.
    #[test]
    fn a_templated_value_is_found_at_any_depth() {
        for args in [
            json!("=item.everything"),
            json!({ "cmd": "=item.x" }),
            json!({ "body": { "cmd": "=item.x" } }),
            json!({ "argv": ["bash", "-c", "=item.x"] }),
            json!({ "body": { "steps": [{ "cmd": "=item.x" }] } }),
        ] {
            assert!(
                any_argument_is_templated(&args),
                "{args} is templated and must be judged as an agent call"
            );
            assert_eq!(
                judge_node("shell", &args),
                Judgement::Stop(StopReason::UnboundedReach),
                "{args}"
            );
        }
    }

    /// ...and a descriptor with nothing templated in it is not dragged back to
    /// the agent rule by a value that merely looks structured.
    #[test]
    fn a_fully_authored_descriptor_is_not_templated() {
        for args in [
            json!({}),
            json!({ "command": "echo hi", "timeout": 30, "quiet": true }),
            json!({ "url": "https://example.com/a=b?c=d" }),
            json!({ "argv": ["bash", "-c", "ls"], "env": { "A": "1" } }),
            json!({ "body": null }),
        ] {
            assert!(!any_argument_is_templated(&args), "{args}");
            assert_eq!(judge_node("shell", &args), Judgement::Silent, "{args}");
        }
    }

    /// The two paths are not two names for one behaviour. If a refactor
    /// collapsed them — passing `Agent` everywhere, or `AuthoredWorkflowNode`
    /// everywhere — every test above could still pass while the split was gone.
    #[test]
    fn the_two_paths_actually_differ() {
        // An ACTING command. Since issue #875 the agent path reads the command
        // rather than the tool name, so a read is judged silent on BOTH paths
        // and would collapse this assertion without the split having changed.
        let args = json!({ "command": "rm -rf ." });
        assert_ne!(
            judge_agent("shell", &args),
            judge_node("shell", &args),
            "the paths have collapsed into one — #674's split is not implemented"
        );
    }
}
