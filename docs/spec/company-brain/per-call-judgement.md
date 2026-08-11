# Per-call judgement

How the approval gate decides which calls warrant a human *on their own merits*,
in the gap the static configuration leaves (issue #338, spine epic #183 §4).

Read [approvals.md](approvals.md) first: this is **step 7** of the precedence
chain documented there, and the placement is most of the argument.

Steps 1–6 are all decided before the run starts, by an operator writing a
manifest. Nothing in them looks at what the run is about to do. That is a bad
trade in both directions: a low bar stops the run constantly, and `full` lets it
send, publish, pay or delete without asking anyone.

Step 7 closes that gap. `src/policy/judgement.rs` asks, per candidate call,
whether it warrants a human on its own merits — and it can **only ever add a
stop**. It runs only where step 6 already allowed the call, so:

- the reserved `never_do` slot, the `readonly` brake, both grant arms,
  `always_approve`, the daily cap and `auto_approve_under_usd` have each
  already had their say and returned before it is reached;
- it cannot turn a deny into a park, skip `always_approve`, or spend a grant.

`supervised` and `readonly` are untouched by construction: a `Consequence` call
already parks under one and is already denied under the other, so every call
this would stop has already been stopped. That leaves the two autonomous tiers.
`full`'s contract is "act without asking, *except the few things on the
always-ask list*", and this is what makes that exception mean something without
an operator having to anticipate each one by name. `auto` (#560) already parks
what leaves the company or spends money, so this adds only the unbounded tools
its line does not draw — `shell` and its family. The same rule applies in both;
no tier gets a carve-out of its own.

## What stops

**In order:** Three rules, in order, all derived from declarations that
already exist rather than a new taxonomy:

1. a **declared consequence class** (`Spend`, `Send`, `Sign`, `Publish`,
   `Hire`, `Identity`) **paired with `Reach::Consequence`**. Both halves are
   required: `web_search` is declared `Spend` because the backend bills per
   request, but its reach is `Money` — the spend buys the call, nothing changes
   and nothing leaves. Stopping on the group alone parks every search in the
   company, and a parked search is a search that never happens. Media
   generation is `Spend` + `Consequence` and does stop, because it moves real
   money on submit;
2. **money named in the arguments** — a positive `amount_usd`, whatever the
   tool's own class says;
3. **unbounded reach** — a named set (`shell`, `curl`, `http_request`,
   `web_fetch`, `git_operations`, `run_workflow`) that is `Consequence`.
   This is how a *delete* stops: destruction has no `EffectGroup` of its own,
   and `shell` is how a run deletes.

Rule 3 is a **list, and was briefly a derivation**. `Consequence` +
`Standing::PerCall` looks like it names exactly this set, since #444 refuses a
standing grant to all of it — but `PerCall` is refused for two reasons the flag
cannot tell apart: *unbounded* (`shell`) versus *bounded but overwriting
operator-authored state* (`workspace_write`). Deriving from it swept up the
workspace tools, which is how a company publishes to its own note tree, and
broke publishing outright. What keeps it narrow at the other end is
`Grantable`: the agent's own scratch writes (`file_write`, `edit`,
`apply_patch`, `memory_store`) are `Consequence` too and do not stop, so "a run
that only reads, searches, or drafts does not stop" stays true.

**What it deliberately does not stop.** #338's acceptance says "send, publish,
pay, or delete"; this delivers three of the four. **`publish_artifact` is
excluded** (`DEFERRED` in `src/policy/judgement.rs`), pending issue #658. It is
declared `EffectGroup::Publish` + `Reach::Consequence` and so qualifies under
rule 1, but this gate's only escape is a per-call grant — so stopping it would
end unattended publishing for every `full` company with no operator override,
which is a product decision belonging to the owners of #244 rather than a side
effect of adding a classifier. #658 carries the argument and the options.

**It does not learn.** If an operator approves the same shape of call five
times, the sixth still stops. The module holds no state across calls at all.
Consent is an operator writing a rule — that is issue #563 — and a rule can be
read, revoked and shown. A classifier noticing a habit is none of those, and
turning a security boundary into a frequency count is how it stops being one.
The two mechanisms must not converge.

This is also why **novelty is not a signal**, though #338 lists it among the
four. Every use of "we have seen this before" makes the gate looser, and the
looser second call is already governed by the single-use grant: the operator
approves one call, redeeming it consumes it, the next identical call parks
again. A novelty rule that skipped the second stop would quietly repeal that.

**It is not a model call.** A classifier LLM on the approval path costs money
and latency per candidate, is itself an external effect deciding whether
external effects are allowed, and returns different verdicts on different days —
and "why did this stop?" has to be answerable from the trace months later.

**Failure is a stop.** There is no unclassified-so-allow path: an undeclared
tool reaches `consequence_of`'s cautious fallback and a Composio slug nobody
has classified is treated as a send, so both stop. Fail-closed is not a branch
to remember — it falls out of reusing a classifier that is already cautious.

**Coverage.** The workflow `tool_call` path never consults `ApprovalPolicy` at
all (issue #460), so nothing here reaches it. The acceptance criterion "stops
regardless of mode" holds on the harness path; on that path it is unreachable
until #460 lands.

