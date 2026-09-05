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
- `commit`, `question` and `defer` are **ungated**: every seat keeps them
  however narrow its entry, and an entry naming one is accepted and ignored
  rather than refused — it describes what the seat could already do. See
  [What no table can take away](#what-no-table-can-take-away).
- An unknown kind and an unknown member id are refused at validation, because
  both fail **open** at runtime — the member silently keeps every move and the
  desk goes on voting, which is the exact symptom the table exists to fix.

### What no table can take away

`commit` is bookkeeping, not authorship. The library authorizes a commit turn by
setting the phase, and the *attention market* picks who holds the floor when it
does — not the manifest. A six-member `hive_math_lab` desk (`quorum = 3`,
`require_evidential = true`) reached quorum at the third evidential `!support`,
flipped to `Commit`, and the fold then gave the floor to the skeptic, the brute
forcer and the archivist for the remaining eight turns. None of them held
`commit`, so each was shown a no-move block, wrote prose ("the answer is ready
for someone to commit"), and the room reported itself **exhausted** on an answer
it had already decided. Recording a topic the room has carried re-derives
nothing, so no seat is too cheap to do it.

`question` and `defer` are the two honest things a member with nothing to add
can say. A seat barred from both has only silence or a guess left, and prose
deposits no trace while still costing the room a turn.

So `hive.moves` gates the deliberation kinds only — `propose`, `support`,
`object`, `refute`, `evidence`, `pin` — and never these three.

### The Commit phase names the topic

Because the committing seat is regularly not one of the seats that carried the
topic, the Commit block names the id outright rather than describing it:

```text
The room has reached quorum and carried `#euler12-triangle`; record it. Reply
with ONE line only:
!commit #euler12-triangle ^N  then why, citing the evidence it rests on
…This is bookkeeping, not a fresh judgement: record the topic the room actually
settled on rather than the one you would have preferred, and do not re-derive
the answer.
```

The id comes from the same fold the floor block is rendered from, so a seat can
never be told to record a topic its own standings do not show.

## Topic-id discipline

One live episode coined `#euler12`, `#euler12-triangle` and
`#euler12-triangular` for a single number. Support split across three names adds
up to nothing, and the room spent its budget one supporter short of a quorum it
had really reached three times over.

The driver therefore derives **one** canonical id per task and repeats it in
every deliberating prompt:

| Input | Id |
| --- | --- |
| a `topic: #foo` line anywhere in the operator's message | `foo` — the operator naming it beats any derivation |
| `Project Euler 12: Highly divisible triangular number` | `euler12` — the short head before the colon, framing words dropped, a trailing number folded onto the word before it |
| `Decide the rollout.` | `decide-rollout` |
| nothing sluggable | `answer` |

```text
Topic id for this task's answer: `#euler12`. Every !propose, !support and
!evidence about the answer uses exactly this id. Only a genuinely different
candidate value gets a different id (`#euler12-2`). Never invent a synonym for
an id already on the floor.
```

The floor block still lists the ids actually on the floor with their standings;
the line above it says what the room should have called this one in the first
place.

## Citation discipline under `require_evidential`

Under `require_evidential` the fold counts a `!support` only when its citation
chain reaches an `!evidence`. Two properties of that walk are easy to miss, and
the live run missed both: a `!propose` is **not** evidence, so `!support
#euler12 ^2` naming the proposal is well-formed, reads as grounded and counts
for nothing; and topic ids are **not** compared during the walk, which is by
sequence only.

A support that counts for nothing is worse than a missing turn, because the
transcript reads as though it counted. So:

- the rules block tells a desk that requires it that a `!support` must cite an
  `!evidence` line, or a support that does, and that citing a `!propose` alone
  counts for nothing;
- the driver checks the same chain the fold will, over what this turn could
  see, and hands back **one** correction — the same one-retry mechanism a
  barred move gets:

```text
Your `!support` reaches no `!evidence`, so this desk counts it for nothing:
citing a `!propose` is not grounds. Evidence on the floor: ^3, ^7. Reply again
with ONE line citing one of those, or deposit your own fact with
`!evidence #topic ^N`.
```

With no evidence on the floor at all the correction says so and points at
`!evidence` / `!question` instead of naming an empty list.

The second attempt is journaled **as-is**, never demoted: the host does not get
a veto over what a member is allowed to think, and the fold decides what the
line is worth. Only the move grammar demotes.

### What the prompt shows

Only the markers this seat holds, phase-gated on top: `commit` is the library's
to authorize (it is absent while the room deliberates, present for **every**
seat in the Commit phase), and the deliberation markers are the desk's to
assign. A seat that was actually narrowed is also told so, once:

```text
Reply with ONE line only, beginning with exactly one of these markers:
!object >N ^M  then why, objecting to message N and citing message M
!evidence #topic ^N  then a fact, adding grounds without taking a side; …
!question  then what you need that nobody has established
!defer #topic  then who should answer instead, when this is not your area
<the rules those moves are read under, ending>
If you have nothing to add, reply !defer #topic naming who should act next, or
!question. Prose without a marker counts for nothing and costs the room a turn.
These are the ONLY markers this desk gives you. A line opening with any other
marker is handed back to you once for correction, and on a second attempt it is
journaled with its marker stripped — it will say what you wrote and count for
nothing.
```

`!question` and `!defer` are in that list whatever the table said, so the block
always offers a way out that is not prose.

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

