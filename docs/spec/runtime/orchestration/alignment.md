# The alignment layer

*Phase P1. How many agents on one company stay pointed at one goal.*

Three of the four alignment mechanisms. The **brief** is the one document
nearly every reasoning role is told; the **ledgers** are what the brief is
derived from and what closes a [demand](demand-ledger.md); the **board** is
where a role asserts something it cannot yet establish.

The fourth — **context routing**, which decides what each role is told at all —
is large enough to own a file: [context-routing.md](context-routing.md).

Terms: [glossary](../../glossary.md). Principles:
[README](README.md#the-three-principles).

---

## The brief

### What it is

`brief.md` is the one document nearly every reasoning role is routed. It carries
what the company has established, what it has ruled out, what the numbers look
like, and what it holds from durable memory that this run has not itself
re-derived.

It has exactly one writer: a `curator` role. A file every role is told to append
to and none owns is how it drifts — the sibling runtime retired exactly such a
file (`MEMORY.md`) after a live run reached a verified result, a working program
and seventeen recorded claims without writing a single line of it, because no
prompt ever showed it to anybody.

### Established versus recalled

The brief MUST separate what **this company established** from what it
**recalled** from durable memory. They are different epistemic goods: the first
is owned and re-checkable, the second is inherited and may be stale. Collapsing
them is how a company comes to treat a year-old fact as a current finding.

### The budget, and where it is spent

The brief MUST be held to a token budget (default 10,000). "Token" here is a
provider-billed unit, not a Rust `char` or byte; the clamp below sizes on
codepoints as a cheap, tokenizer-free upper bound (one token typically spans
several codepoints for the encodings in use, so the codepoint count is never
smaller than the token count), which is conservative — it may cut earlier
than the true token budget requires, never later. A future revision MAY size
against the actual provider tokenizer if the slack this leaves is measured to
matter.

The clamp MUST be applied **where the brief is spent** — at prompt assembly —
and MUST NOT be applied by refusing the write. Refusing the write costs the
company whatever the curator was about to record; clamping at assembly costs
only the tail of one prompt. The clamp keeps the **leading** portion, because
the file is written most-established-first, cuts on a UTF-8 character
boundary (never splitting a codepoint), and appends a visible marker saying it
was cut and to what budget.

An unreadable brief — a missing file, or one that fails to parse as UTF-8 —
measures as empty rather than erroring. This is a deliberate, narrow exception
to the [routed-document hard-error rule](context-routing.md#failure-handling) above: every other
routed document is authored by a person or an agent, so a corrupt one signals
a bug worth surfacing loudly. `brief.md` is instead **wholly machine-derived
and machine-consumed** — no prompt is ever written around its absence the way
a role's prompt is written around a named workspace document — so a corrupt
brief is evidence of a curator bug, not of missing context a role expected.
Treating it as empty and letting the next curator pass re-derive it keeps a
transient corruption from becoming a company that cannot start; erroring here
would make a summary into a dependency.

### Cadence

The curator runs at a **defined point in the loop**, not on a wall-clock timer:
**once per [attempt](demand-ledger.md), after that attempt's [routing
decision](loop.md#routing) concludes and before the next attempt begins.**
This point is chosen because it is the one place the [attempt
loop](loop.md#the-shape) already guarantees sequencing — attempts against one
demand run one after another, never concurrently — so a curator run scheduled
there inherits that guarantee for free instead of needing one of its own.
Concretely: the curator is the last step of an attempt's post-routing
handling, and the loop's next attempt (if `Retry`/`Diversify` routed one)
does not start until the curator step for the prior attempt has completed.

This narrows the sibling design deliberately. Its documentation describes a
periodic standing team on a configurable interval; its code deleted that team
after a live run put two curators on the same empty workspace in the same
second, and the interval variable is now read nowhere. Port the single-writer
rule and the budget clamp, which are live and load-bearing. Do not port the
cadence, which is documentation about code that no longer exists.

---

## The derived ledgers

### The pattern

A derived ledger is a Markdown file **written by code, never by an agent**,
re-derived whole from a directory of source items on every relevant write. An
item's *source* is either a dedicated file (one file per item — this is how
`threads.md` is sourced) or a fenced block embedded in a workspace note that
may hold several items (this is how `claims.md` is sourced, from ` ```claim `
blocks in any note). Either way, the ledger addresses one item at a time by
scanning its sources for the fenced-block or whole-file unit, never by
treating a source file itself as the unit of retrieval.

Four properties follow, and all four are the point:

1. **It cannot drift.** It is not a summary anybody maintains; it is a
   projection of files that exist.
2. **Concurrency is nearly free.** Two agents writing distinct items conflict on
   nothing. They would conflict on the render, so one mutex around the render is
   the whole coordination story.
3. **It is retrievable one statement at a time.** A file is the wrong unit to
   retrieve: an agent about to act needs one statement with its conditions, not
   the note that happens to contain it.
4. **Contradictions become visible.** Cross-references between items are checked
   at derivation, so a conflict nobody noticed is reported rather than latent.

### What ships

Two ledgers. Not the sibling's ten — the rest are proof-shaped and specific to
mathematics.

#### `claims.md` — the evidence ledger

Derived from fenced ` ```claim ` blocks in any note under the company workspace.

| Field | Meaning |
| --- | --- |
| `id` | stable identifier, cited elsewhere |
| `statement` | one thing held to be true |
| `conditions` | what it depends on |
| `holds-here` | whether it applies to this company's situation |
| `status` | `verified` \| `sourced` \| `asserted` \| `heuristic` |
| `contradicts` | another claim id |
| `answers` | a [demand](demand-ledger.md) id — the closure edge |

Two checks fall out of derivation that a prompt could ask for and never verify:

- `contradicts` naming a claim that exists on disk produces a **contradiction
  the company can see**.
- `holds-here: yes` with `status: asserted` is a **load-bearing belief nobody
  verified** — the distinction a long-running company forgets it made.

A block missing its `id` or `statement` MUST be **reported, not dropped**. A
claim silently discarded leaves the note reading as though it recorded
something.

#### `threads.md` — the direction ledger

Derived from ` ```thread ` blocks, one file per thread: `question`, `status`,
`rests-on`, `blocked-by`, `next`.

- **Dead threads are kept.** A known dead end is a result, and the reason it
  died is what stops the next agent paying for it again.
- A thread resting on a claim id not on disk MUST be reported.
- A thread marked blocked with no blocker named MUST be reported: a blocker
  stated precisely is the next [demand](demand-ledger.md); one left blank is a
  mood.

### The re-derivation cascade

A write that could change a ledger MUST trigger re-derivation, and the tool
result MUST **name every derived ledger that moved**. A model not told the ledger
changed has no reason to read it again.

Search over a ledger MUST re-derive rather than read the rendered file. A stale
table is the one failure a derived ledger exists to prevent.

### Locking

Two process-global mutexes, and the distinction matters:

- **The write lock** is held across a write *and its entire re-derivation
  cascade*. It MUST be taken at a tool-call boundary and never below one: it is
  not reentrant, and the cascade re-enters the document store several times.
- **The commit lock** is separate. One git index cannot serve two concurrent
  `git add` operations; the loser leaves a stranded lock file and its commit is
  lost silently.

Per-file atomicity comes from write-temp-then-rename and depends on neither.

#### Search re-derivation takes no lock

A search re-derives (above) rather than reading the rendered file, which raises
the obvious question of whether it must take the write lock. **It MUST NOT**,
and the reason is that it is a different operation than it looks:

- A search re-derivation **renders into memory and writes nothing**. The write
  lock guards a write *and its cascade*; a read that produces no file has
  nothing to serialise against.
- Taking it would be actively harmful. The lock is not reentrant and is held
  across an entire cascade, so a search issued from inside a cascade — a tool
  the cascade itself invokes, now or after some later change — would deadlock
  against a lock its own call stack already holds. Every read contending on the
  writer's lock would also serialise reads behind unrelated writes.

What a lock-free search can observe is a source directory mid-cascade: some
items updated, others not. That is acceptable and bounded, because each source
item is written temp-then-rename, so a search sees every individual item either
wholly before or wholly after its write — **never torn**. The worst case is a
result set that omits an item written microseconds ago, which is the same
staleness any concurrent reader accepts and is strictly better than the failure
this design exists to prevent: reading a *rendered* table that no longer matches
its sources at all.

A caller needing a point-in-time-consistent view across several ledgers takes
the write lock itself, at its own tool-call boundary, and performs its reads
inside it — the ordinary rule, not a special case for search.

---

## The assertion board

### Why it is not the work board

The work board (soon the [demand ledger](demand-ledger.md)) tracks what the
company is *doing*. The assertion board carries what one agent wants to tell the
others but **cannot yet establish**: a dead end, a lesson, a hunch, an offer, a
question.

The two must not merge, because their epistemic status is opposite. A demand is
closed by evidence. A board post is, permanently, somebody's opinion.

### Three enforcements

1. **A post MUST NEVER be an input to a derived ledger.** There is no code path
   from a post to a claim. Assertion cannot launder itself into evidence.
2. **The sender is bound at registration**, not passed as an argument. The post
   tool's schema carries `kind`, `body` and `refers` — never `from`. An agent
   cannot post as another.
3. **Readership is routed** — to roles that decide what to do next, and away
   from every role that weighs evidence.

### Mechanics

Append-only JSONL, rendered to Markdown. One complete line written in a single
call on an append handle is the entire concurrency story: concurrent posters
interleave whole lines, never halves, so **no lock is required** — but this
holds only up to a bound, and the bound MUST be enforced, not assumed. A
single `write(2)` (or platform equivalent) is only guaranteed atomic up to
`PIPE_BUF` (4,096 bytes on every POSIX target this runs on today), and no
such guarantee exists at all on a non-POSIX target. **A board post line MUST
therefore be capped well under that bound** — 2,048 bytes is a safe margin —
and a post that would exceed it MUST be refused at the tool boundary rather
than written and risking a split write that interleaves with another
poster's line. This is a size limit on one JSONL record, not a limit on what
an agent can say: a post that does not fit is a sign it should have been a
workspace note with the board carrying a pointer to it, not a reason to widen
the bound. The kind is a closed set, and an unrecognised kind parses to the
most conservative reading rather than being dropped — a typo must not lose
the post or upgrade it.

The rendered file MUST state its own status in its header: everything here is
asserted, not established; treat a dead end as a reason not to repeat someone's
work, not as proof the route is closed.

### Tell the roles it exists

A capability nobody is told about is not a capability. The sibling runtime ran a
live three-hour session in which the board tool was called **zero times**,
because no prompt mentioned it. Roles granted the post tool MUST also be routed
a brief explaining what the board is for.

---

---

## Verification

- A derived ledger re-derived from a fixture directory is byte-stable across runs.
- A claim with `holds-here: yes` and `status: asserted` is reported.
- A `contradicts` edge to a claim on disk surfaces as a contradiction.
- A claim block missing `id` or `statement` is reported, never dropped.
- A thread resting on an absent claim id, and a blocked thread with no blocker,
  are both reported.
- The routing table gives each role exactly its declared `context` documents
  and nothing else from the routed-workspace-documents section of the
  assembled prompt — asserted per role, not sampled. This is a claim about
  `role_context`'s output specifically: it does not cover the shared method
  policy or the boundary sentence, which are universal and not declared per
  role, or the workspace overlay, which is a company-owned file outside
  `context` entirely (see [assembly order](context-routing.md#assembly-order-is-a-cache-decision)).
- The brief clamp cuts on a character boundary, keeps the leading portion, and
  appends the marker.
- An agent cannot post to the board as another agent.
- No code path exists from a board post to a claim.
- A board post over the size cap is refused at the tool boundary, never
  written truncated or split.
- The curator step for one attempt completes before the loop starts the next
  attempt against the same demand.
