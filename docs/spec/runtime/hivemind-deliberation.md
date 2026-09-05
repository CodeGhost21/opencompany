# Deliberation mechanics on a hive-mind desk

The move grammar, desk memory, speaker diversity, and turn failure — the four
mechanisms that separate a desk that *deliberates* from a desk that holds a
vote. The rest of the hive-mind contract, including when a room opens at all and
the full manifest table, is in [`hivemind.md`](hivemind.md).

## Why these exist

Live evidence, from a run of `companies/hive_math_lab` over Project Euler
1–145. Every episode was three independent `!propose`s of the same number in the
blind round, then `!commit` — quorum carried because in `tinyhivemind` a
`!propose` counts as its author's own support. No `!support ^N`, `!object`,
`!evidence`, `!question` or `!pin` was ever deposited, and nine episodes
produced two memory writes between them. Three agents agreeing in parallel is
not deliberation; it is three answers with a quorum rule stapled on.

## The per-member move grammar

A room whose members may all make every move is a room that votes. On a live
run of `companies/hive_math_lab` over Project Euler 1–145, **every** episode was
three independent `!propose`s of the same number in the blind round followed by
`!commit` — and quorum carried, because in `tinyhivemind` a proposal already
counts as its own author's support. No `!support`, `!object`, `!evidence`,
`!question` or `!pin` was deposited in nine episodes. Three agents agreeing in
parallel is not deliberation.

`hive.moves` assigns each seat the markers it may **open a line** with:

```toml
hive = { moves = { solver = ["propose", "support", "commit"],
                   checker = ["object", "refute", "evidence", "question"],
                   archivist = ["evidence", "pin", "question"] } }
```

- A member the table does not name may make **every** move, so an omitted table
  is a no-op for every manifest written before it existed.
- A member named with an **empty** list also keeps every move. An empty list is
  a table somebody started and never filled in far more often than it is a vow
  of silence, and the other reading hands a seat the floor with nothing legal
  to say.
- `pin` covers `!unpin` too: both write the same board.
- At least one seat must keep `commit`, or the room reaches quorum with nobody
  able to record it. Refused at validation.
- An unknown kind and an unknown member id are refused at validation, because
  both fail **open** at runtime — the member silently keeps every move and the
  desk goes on voting, which is the exact symptom the table exists to fix.

### What the prompt shows

Only the markers this seat holds, phase-gated on top: `commit` is the library's
to authorize (it is absent while the room deliberates, present alone in the
Commit phase), and every other marker is the desk's to assign. A seat that was
actually narrowed is also told so, once:

```text
Reply with ONE line only, beginning with exactly one of these markers:
!object >N ^M  then why, objecting to message N and citing message M
!evidence #topic ^N  then a fact, adding grounds without taking a side; …
<the rules those moves are read under>
These are the ONLY markers this desk gives you. A line opening with any other
marker is handed back to you once for correction, and on a second attempt it is
journaled with its marker stripped — it will say what you wrote and count for
nothing.
```

### What enforcement does

| Attempt | What happens |
| --- | --- |
| A marker the seat holds, or no marker | Journaled unchanged |
| First barred marker | The **same** prompt plus one line — "You may not \`!propose\`; your moves are !support, !evidence, !question. Reply again with ONE line beginning with one of those markers." — and one more attempt |
| Second barred marker | Journaled with the leading `!` removed, and counted as a violation |

A demoted line keeps the member's words and loses its trace: `resolve` reads a
marker at the start of a line and nowhere else, so a demoted line folds to
nothing and can never be counted as support for anything. That is the property
the whole mechanism turns on — a barred move that still carried a topic would
be a rule the fold does not enforce.

One correction, not a loop: a member that has misunderstood the grammar must
not be able to spend the desk's whole budget being told about it.

The closing `hive-report` row names every demotion, so an operator can tell a
desk whose grammar is wrong for the work from a desk whose members are being
unhelpful:

```text
The desk settled on #stage after 6 turns (backed by planner, critic). 1 line
demoted for a move its author may not make on this desk: @scout !propose.
```

## What the desk remembers

An episode used to leave nothing behind but a thirty-message transcript window
the next similar question scrolls straight past. On the live `hive_math_lab`
run, nine episodes produced **two** memory writes between them — because the
writes came from whichever teammate happened to call `memory_store` inside its
own turn, and a member spending its one line on a marker rarely does.

The driver therefore owns desk memory rather than the members:

- **Before the first turn** it recalls once with the operator's task text and
  renders a bounded "The desk remembers:" block into **every** prompt of the
  episode. Once rather than per turn, because the answer cannot change
  mid-episode. The block is attributed as memory and carries no sequence, so a
  member cannot cite it with `^N` — it is not a line in this conversation.
- **On `Converged`** it writes exactly one note: the task's first line, the
  carried topic, its supporters, every `!evidence` line, the last `!pin`ned
  lines, and the committing line.
- **On `Deadlocked` / `Exhausted`** it writes a shorter "unresolved" note naming
  the topics that competed. Knowing the desk has already argued `#stage` against
  `#ship` without settling it is worth having; pretending it concluded something
  would be worse than silence.
- **On `Idle`** it writes nothing. Nobody spoke, so there is nothing to have
  learned.

| | |
| --- | --- |
| Label | `hive/<desk id>/<slug of the note title>` |
| Namespace | one desk, exactly as `agent-memory/<agent id>/…` is one teammate |
| Port | `ContextStore` — the same `put` / `list` / `search` / `peek_many` the per-turn memory loop and the `memory_recall` belt use, so a hosted-memory overlay (CortexDB under `OPENCOMPANY_MEMORY=remote`) applies unchanged |
| Redaction | title and body both go through `redact_secrets`, at the note's single construction point |

A converged note:

```text
Decide the rollout. — #stage

Task: Decide the rollout.
Carried: #stage
Supporters: planner, critic
Evidence:
- [4] @critic: !evidence #stage ^1 The last full rollout took checkout down.
Pinned:
- [6] @planner: !pin ^4 Keep the outage on the board.
Committed:
- [8] @critic: !commit #stage ^4 The room settled on staging.
```

Recall is desk-scoped by intersecting the store's own ranked `search` hits with
the addresses actually stored under this desk's prefix — `ContextStore::search`
ranks company-wide and returns no label, so the scope has to be applied after
the ranking. A search that matched nothing on this desk falls back to the desk's
most recent notes; recency is the fallback and never a supplement, or a stale
note could outrank a relevant one.

**Both halves are best-effort.** A failed recall renders no block, which is what
a cold store renders anyway. A failed write is logged and the episode finishes:
the decision is already durable in the transcript above it. A memory store that
is briefly unreachable is not a reason to refuse the operator's message.

## Speaker diversity

`dominance_cap` and `repetition_cap` are the library's own damping and are now
exposed on the manifest (above). On top of them, when the fold hands the floor
straight back to the member who just held it **and** somebody has not spoken at
all in this episode, the prompt gains one line:

```text
Members who have not spoken yet: @scout, @critic. You have the floor twice in a
row. If what the room is missing is theirs to supply, !question them or !defer
#topic to them rather than restating your own position.
```

It is a **prompt and never an override**. The library picked this speaker under
invariants this host does not get to break, so the repair available is to tell
the speaker who is missing and let it route the question — which is exactly what
`!question` and `!defer #topic` are for.

## When a member's turn fails

A turn that fails does **not** end the room. The motivating case: on Project
Euler 145 one member's turn hit the harness's per-turn wall-clock ceiling, the
`?` propagated out of the driver and the brain hook, and the cycle answered the
operator with a 500 — throwing away three good turns for one slow one.

Instead the driver:

1. journals the miss on the desk under `hive-report` — `@verifier's turn did not
   finish: <first line of the error>` — which the log adapter reads back as a
   **system** row. It carries no marker, so it folds to no trace and can never
   be counted as support;
2. commits `turn.next_state`, so the budget still advances and the room cannot
   loop on a seat that is down;
3. steps again, choosing the next speaker from a transcript that shows what
   happened.

The episode fails only when the journal append itself fails — nothing can read
the room back, so there is no room — or when **members × 2** turns fail
consecutively, which is every seat twice over: a harness that is down rather
than a room having a bad turn. The count reaches the closing row:

```text
The desk settled on #stage after 6 turns (backed by planner, critic). 1 turn did
not finish and the room continued without it.
```

