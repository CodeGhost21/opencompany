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

The invariant, phrased so it survives the next tier: **this arm only ever speaks
where the mode allowed.** A tier that already parks or denies a call keeps its
own answer, whatever it is called and however many tiers there are — so a tier
an operator has already reasoned about does not shift under them.

Today that leaves **`full` alone**. `supervised` parks a `Consequence` call,
`readonly` denies it, and `auto` (#560) parks everything that is not
`Grantable` — which covers every rule below, so this adds nothing there. That
last one holds by a property of the declaration table rather than by
construction: the day a `Grantable` + `Consequence` tool with a named
consequence class is declared, `auto` would shift silently. The test
`the_arm_adds_nothing_under_auto` walks the whole table and fails on that day,
rather than the claim resting on a paragraph.

`full`'s contract is "act without asking, *except the few things on the
always-ask list*", and this is what makes that exception mean something without
an operator having to anticipate each one by name.

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
pay, or delete". Of those, **send, pay and delete are gated; publish is not**,
and the outward-HTTP family is held back as well. Both carve-outs live in
`DEFERRED` in `src/policy/judgement.rs`:

- **`publish_artifact`** — issue #658. It qualifies under rule 1
  (`EffectGroup::Publish` + `Reach::Consequence`), but this gate's only escape
  is a per-call grant, so stopping it would end unattended publishing for every
  `full` company with no operator override. That is a product decision owned by
  #244, not a side effect of adding a classifier.
- **`http_request`, `curl`, `web_fetch`** — issue #674. They qualify under rule
  3, and #614 (merged as #627) states the opposite premise in a test named
  `an_http_request_node_does_not_gate_under_full`. A named test is a stated
  position; choosing between two defensible premises belongs to #614's owner.

**`shell` stays gated**, which is what keeps "or delete" real: destruction has
no `EffectGroup` of its own, `shell` is how a run deletes, and #614 does not
touch it. `git_operations` and `run_workflow` stay too.

A test asserts every deferred tool *qualifies* on the declaration and is silent
anyway, so deleting a carve-out fails loudly rather than silently re-opening
the question.

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

