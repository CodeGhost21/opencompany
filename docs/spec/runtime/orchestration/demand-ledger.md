# The demand ledger

*Phase P2. The work model. Normative.*

A **demand** is a stated need: something the company does not yet know or does
not yet have, said by whoever walked into it, with what they would do about it
and what would change their mind. Demands are deduped against what the company
already knows, and a demand closes only when something on disk **cites it**.

The demand ledger replaces the kanban board as the work model. The board's
columns survive as a projection of demand state.

Terms: [glossary](../../glossary.md). Depends on
[`CLAIMS.md`](alignment.md#claimsmd--the-evidence-ledger), which is what closes a
demand.

---

## Why replace the board

### The board's own documentation names the disease

[`src/ports/tasks.rs`](../../../../src/ports/tasks.rs) argues that auto-filing a
successful run as Done *"would make the terminal column mean 'the model stopped
talking' rather than 'we shipped this'"*. That is exactly right, and the fix
applied there was to require a human on every close.

That fixes who decides. It does not fix what "the work happened" is read from.
A card in Done proves that a person moved it. A demand closes when a deliverable
carries its id — so *was the need met* is read off the record rather than
asserted by whoever went looking.

### Three properties a card board cannot have

**Dedup.** Two agents blocked on the same thing produce **one row**. A card
board produces two cards, and nothing notices. This is also the bound that makes
agent-initiated work safe: a loop restating the same need yields one demand, not
an unbounded fan of cards.

**Pull, not push.** A card is created by a planner who decided what should
happen. A demand is stated by whoever is stuck. The role that discovers a gap is
whichever one walked into it, so **every agent holds the tool** — confining it
to a planner means the gap is stated by whoever is going looking rather than by
whoever is blocked.

**Answered from what is already known.** A demand is checked against the claim
ledger *before* it is recorded. The common case — the company knows this and has
forgotten — costs a lookup instead of a run. This is the company's reluctance to
duplicate work made mechanical rather than requested.

---

## Shape

```rust
struct Demand {
    id: DemandId,               // derived from the need text; see Identity
    need: String,               // what is missing
    why: String,                // what the asker would do with it
    falsifies: String,          // what would show the current belief wrong
    from: AgentId,              // who is blocked
    for_: Option<Assignee>,     // who could unblock it, if known
    state: DemandState,
    blocked_by: Vec<DemandId>,
    answered_by: Vec<ClaimId>,
}
```

`falsifies` is the field that earns the demand ledger its keep. It turns a
topic into a question. *"Anything on our churn numbers"* is a search; *"whether churn is
concentrated in the first billing cycle, because if it is not, the onboarding
rewrite does not pay for itself"* is something evidence can answer or fail to.
A demand with an empty `falsifies` SHOULD be refused with that explanation.

### States

```text
open ──claim──> claimed ──a claim cites the id──> answered ──operator──> accepted
 │  ▲              │  ▲
 │  └── blocker answered ──┐
 └──────────── blocked ────────────┘            (any state) ──> dropped
```

`blocked` is not terminal: when the blocking demand referenced in `blocked_by`
reaches `answered`, code moves the blocked demand back to the state it was in
when it became blocked — `open` if nobody had claimed it yet, `claimed` if an
agent already had. Blocking never discards work in progress.

| State | Meaning | Who moves it |
| --- | --- | --- |
| `open` | stated, nobody on it | — |
| `claimed` | an agent is working it | an agent, or an operator assignment |
| `answered` | a claim on disk cites the id | **code**, on ledger re-derivation |
| `accepted` | the operator accepts the result | **a person, only** |
| `blocked` | `blocked_by` names an unclosed demand | code |
| `dropped` | withdrawn | an operator |

`blocked_by` MUST be validated on write: a self-reference, or any cycle formed
by following `blocked_by` edges transitively, MUST be rejected rather than
recorded. An accepted cycle leaves every demand on it permanently `blocked`
and undispatchable — the one state this ledger cannot recover from on its
own, since nothing on either demand can ever close to break the loop.

---

## Answered is not accepted

This is the invariant that keeps the collapse compatible with the product.

`COLUMN_DONE` is today reachable **only by a person** — a deliberate operator
decision that supersedes the original epic's automatic route, precisely so the
terminal column does not come to mean "the model stopped talking".

Evidence-closure and human acceptance are **different axes**, and the demand
ledger keeps them separate rather than choosing between them:

- `answered` is a **machine** transition. It says the need has an answer on
  disk, and it is derived, so it cannot be asserted by the agent that wanted it
  to be true. Deliberately, `answered` does not gate on the citing claim's
  `status` — a `heuristic` or `asserted` claim closes a demand exactly as an
  `verified` one does. Requiring `verified` here would duplicate the quality
  gate the next transition already provides and would let a demand sit
  invisibly `claimed` while an operator has no record that *any* answer, weak
  or strong, has surfaced. `status` stays on the claim and is what a reviewer
  reads before the next transition.
- `accepted` is a **human** transition. It says the operator wants this. This
  is where evidence quality is actually judged: an operator reviewing an
  `answered` demand reads the citing claim's `status` alongside it, and a
  `heuristic` claim is exactly the signal that should make them hesitate
  before accepting.

An agent MUST NOT be able to move a demand to `accepted`, including its own.
*Agents propose; the Operator disposes* ([agentic/](../../agentic/README.md)) is
unchanged by this section.

## The column projection

The board columns become a derived view. No column is stored on a demand.

| Column | Demand state |
| --- | --- |
| `todo` | `open` |
| `in_progress` | `claimed` |
| `in_review` | `answered` |
| `done` | `accepted` |
| `paused` | `blocked` |
| `planning` | **removed** — becomes the first attempt of the [loop](loop.md) |
| *(none — dropped)* | `dropped` |

`planning` is the one column that does not survive. It exists today as a
transient station firing a single tool-less model call with no retry; the
[attempt loop](loop.md) makes that the first attempt of something that can
retry, which is what the station was reaching for.

`dropped` has no column: a withdrawn demand is removed from the board view
entirely rather than parked in a terminal column, because it was withdrawn,
not finished. The [verification](#verification) claim that the projection
round-trips "every demand state" means every state that is still on the
board — `dropped` is the one state that is, by definition, off it.

---

## Identity and dedup

A demand's id MUST be a **deterministic** derivation from its `need` text — a
hash of the normalized (whitespace-collapsed, case-folded) text — so the same
gap stated twice, byte-for-byte, produces the same id without a lookup. This
is `DemandId` derivation, and it is separate from the dedup check below:
identity answers "is this the same string", not "is this the same gap in
different words", and only a deterministic function keeps `Claim.answers`
referring to a stable id rather than one that could shift between two
semantically-equivalent restatements.

**Dedup — recognizing that a *differently worded* demand is already known —
MUST be semantic, not lexical.** The sibling runtime derives its id from a
hash of the whitespace-stripped text (identity only) and separately checks "do
we already know this" with a two-distinctive-word overlap against the claim
ledger. That overlap check dedupes exact restatements and a little more; it is
the weakest part of an otherwise strong design, and it is weak because that
runtime had no semantic retrieval to hand.

OpenCompany does, once [P0](memory.md) binds `MemoryRecall`. The already-known
check — "does an existing claim already answer a gap worded like this one" —
MUST go through `MemoryRecall` as **candidate matching**, replacing the
sibling's lexical overlap threshold outright rather than layering semantic
recall on top of it. A candidate claim MemoryRecall surfaces still needs the
same relevance bar the lexical check was reaching for — a return on a subject
is not the same as an answer to a specific `falsifies` — so the recall
integration MUST rank or threshold candidates on relevance to the demand's
`falsifies`, not merely its topic; this is the one place the port deliberately
improves on its source; it does not change what `DemandId` is derived from.

### Refusal is informative

A demand refused because the company already knows the answer MUST return **the
claims themselves**, not merely a refusal. The point is to put what is known in
front of the asker, not to send them looking for it.

---

## Dispatch

### What is being replaced

Dispatch today edge-fires in `CompanyRuntime::upsert_task` on the *column
transition* into `in_progress`, and that column write is the only thing that
starts a board run. [`src/workflows/caps/mod.rs`](../../../../src/workflows/caps/mod.rs)
records why that matters: run → card → dispatch → run stays bounded *because
every dispatch still requires an operator act*.

A derived ledger has no column and no drag. So the trigger must be replaced, and
the replacement must carry its own bound.

### Claiming is the dispatch edge

A demand moving `open → claimed` starts the work. Three bounds hold, and
together they are stronger than "a human dragged it":

1. **Dedup.** A runaway agent restating one need produces one demand. The
   pathological case a card board makes cheap is the case this makes impossible.
2. **Depth and cycles.** The existing delegation scope chain caps depth
   (`max_delegation_depth`, default 2, bounded 1..=4) and refuses a target
   already on the chain.
3. **Closure is not self-assertable.** An agent cannot mark its own demand
   answered; only a claim on disk does that. So a loop cannot manufacture
   apparent progress.

An operator dragging a card remains *one* way to claim, not the only one.

---

## Staging

The blast radius is real and the migration MUST NOT be attempted in one step:
~2,600 lines of REST handlers, ten endpoint families across two mounts, seven
`CompanyEvent` variants, ~6,000 lines of console code, five store
implementations plus a conformance suite, and a hand-mirrored copy of the column
vocabulary in the frontend that no Rust test can see.

### P2a — alongside

Implement the demand ledger as a decorator over `TaskStore`, in the position
`BoardAnnouncer` ([`src/runtime/board_events.rs`](../../../../src/runtime/board_events.rs))
already occupies. Cards keep dispatching. Nothing user-visible changes. This
stage exists to prove dedup and evidence-closure against real traffic before
anything depends on them.

### P2b — invert

The ledger becomes authoritative and `TaskStore` becomes a projection over it.
REST, GraphQL and the console are unchanged, because they read the projection.
Claiming becomes the dispatch edge; the column write becomes one way to claim.

### P2c — rehome the card's extras

A card is a hand-curated object with identity: a discussion thread with
per-message redaction, a steer endpoint, a plan, a workflow proposal with
apply/reject. A **derived** ledger is by definition not hand-editable, so each
of those needs either a ledger-native home keyed by demand id, or a thin card
projection that retains it. Decide per feature; do not collapse them wholesale.

---

## What does not change

**A2A.** `POST /a2a/{handle}` with `tasks/send` is an external protocol contract
and carries payment. An inbound A2A task becomes a demand, and the externally
addressable task id survives as its projection. The wire format is not ours to
change.

**Approvals, metering, budgets.** A claimed demand runs through the same gate,
meter and caps a dispatched card does.

---

## Verification

- Two agents stating the same need produce one demand.
- A demand the claim ledger already answers is refused, and the refusal carries
  the claims.
- A demand with an empty `falsifies` is refused with the reason.
- A demand cannot reach `answered` except by a claim citing its id.
- A `blocked_by` self-reference, and a `blocked_by` cycle of any length, are
  both rejected on write.
- No agent-reachable path moves a demand to `accepted`.
- An agent cannot accept its own demand.
- The column projection round-trips every on-board demand state (every state
  except `dropped`), and `planning` is absent.
- The dispatch bound holds: an agent restating one need in a loop cannot spawn
  unbounded runs.
- A demand blocked by an unclosed demand projects to `paused`, and unblocks when
  its blocker is answered.
