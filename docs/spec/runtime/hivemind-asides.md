# Private asides

Two members of a desk comparing notes without the room.

[Cross-desk referral](hivemind-referral.md) lets a room ask another desk a
question. An aside is the other direction and stays inside one desk: it lets two
members of the **same** desk say something the rest of that desk cannot read.
Both are agent-to-agent communication, and both obey the same rule at the
boundary — see [what crosses carries no vote](#an-aside-carries-information-never-support).

The mechanics come from [`tinyhivemind`](https://github.com/tinyhumansai/tinyhivemind)
(`vendor/tinyhivemind/docs/specs/private-asides.md`). This host supplies the
journal field, the manifest knob, the grammar in the prompt, and the fold that
counts what a pair has spent.

## It is off by default, because it was measured and it lost

The library ran the experiment
(`vendor/tinyhivemind/docs/experiments/2026-09-07-do-asides-help.md`): a private
pairwise check costs **2.5 points** at a tuned turn budget and nothing at an
unconstrained one, loses **15 points** on a hidden profile — averaging inside
one correlated desk imports the shared bias rather than cancelling it — and is
**indistinguishable** from the same exchange held in the open.

So the mechanism claims bounded independence and an auditable record of it. It
claims nothing about answer quality, `AsideConfig` defaults to disabled, and a
desk that wants asides has to say so. Enabling one should be a decision, not a
default.

## Auditable, not confidential

**Privacy here is between agents and is a deliberation device. It is never a
security boundary, and nothing may be built on it as one.** An operator and
every person reads every aside in full.

What narrows is the projection handed to a *peer agent*, and even there the row
is **elided rather than removed**: its sequence, its author and its audience
stay, and only the content is withheld.

```text
[7-10] @stock_controller → @fleet_tech · aside, 4 messages · settled at [11]
```

Three reasons the row stays, rather than disappearing:

- **Auditability.** Two agents cannot exchange anything without leaving an
  attributed, sequenced row. An absent row is precisely the covert channel the
  literature names.
- **Latent asymmetry.** An agent that cannot see that a peer knows something has
  no reason to ask, and that is the documented failure of collective reasoning
  under distributed information.
- **Citations.** The trace grammar addresses messages by sequence. Silently
  removing rows leaves `^N` naming nothing; a citation landing inside an elided
  range resolves to the collapsed row, so the reader learns that its citation
  names something it may not read — the honest answer, at one row instead of
  four.

## An aside carries information, never support

There is exactly one transcript for counting and it is the same for every
reader. `step` folds the whole transcript; `project_for` is the only thing that
narrows, per speaker.

That ordering is load-bearing, and `EpisodeDriver::run` implements it directly:
the `SessionQuery` this host issues carries `Viewer::Operator` — the true,
unelided medium — and the per-speaker narrowing happens in `project_for` on the
way into the prompt. A per-viewer fold would make `quorum::standings`,
`attention::bids` and `directory` return well-formed **wrong** answers with no
error path: a quorum one member can see and another cannot, and a floor-holder
that differs by reader when there is one floor.

The uniform rule, applied identically for every reader: **a trace deposited in a
row whose audience is not `Desk` contributes nothing.** It adds no supporter,
moves no option toward a decision, silences no advocate. To make an aside count,
a member spends a desk-visible turn saying so in the open — that is `!surface`,
and a `!surface` line is an ordinary desk row with no special handling.

This is the rule cross-desk referral already accepts at a channel boundary,
applied inside one desk.

## An aside costs no turn

**One authorized turn produces two rows**: the member's ordinary desk-visible
contribution, and — optionally — one private row riding alongside it. The aside
is not a turn, does not become one, and is not charged as one
([ADR 0011](https://github.com/tinyhumansai/tinyhivemind/blob/main/docs/adr/0011-an-aside-rides-alongside-a-turn.md)).

It did cost a turn at first, and that was measured and it was expensive:
upstream's benchmark found a room whose members may open one pairwise check
decides **15.4 points worse** on a hidden profile, and the control that writes
the identical words and *throws the answer away* loses more still — so the
transfer was never what cost anything. Under one-message-one-turn, a member
asking a peer is a member not depositing, not objecting and not refuting, while
the rest of the room goes on accumulating support for the option it stepped away
to ask about. The room reaches quorum on the decoy while its members are away
asking about it.

This host confirmed the same thing from the other side: a live six-day run of
`companies/vending_machine_co` with asides enabled used the marker **zero
times**, while the same seats repeatedly wrote `!question @peer` — the move that
was already in their list and did not cost them their contribution.

Three bounds keep it sound, and `EpisodeDriver::run` implements each:

- **At most one aside row per turn.** `split_reply` takes the first `!aside`
  line and no more, so a room of *n* members writes at most *n* private rows per
  round, each of which cost its author a turn it had already won.
- **An aside starts no turn.** The row is journaled and nothing is dispatched
  from it; the peer answers on its own next turn, which the attention market was
  going to give it anyway.
- **A refused audience is dropped, not published.** Falling back to the desk
  would put a second desk-visible contribution on one turn, which is the one
  thing a turn may not produce — and the member has already said its piece in
  the row above.

It is not free of a *sequence*: the private row still takes the next journal
sequence, so later desk rows land at a higher raw sequence than they would have.
`step`, `standings` and `spent` are all invariant under that, but
`salience::standing` scores recency from raw distance, so a busy aside round
shifts the floor-holder choice slightly. That is a known limitation upstream,
not something this host can close.

## The grammar

Two markers, taught **only when the desk enabled asides** — a grammar is a fixed
cost paid in every agent's system text on every turn, and teaching a move nobody
may make spends that budget for nothing.

| Marker | Means |
| --- | --- |
| `!aside @peer …` | a private line to that peer, on a **second line** under the member's move |
| `!surface …` | an ordinary desk move reporting back what the aside produced |

Neither is a deliberation move. Neither appears in `MOVE_KINDS`, neither is
gated by `hive.moves`, and neither deposits a trace — so they are taught after
the move list rather than inside it. `split_reply`
(`src/hivemind/prompt.rs`) is what separates the two: the marker line the room
counts, and the one `!aside` riding under it. A reply that is *only* an aside
still owes the room its turn, so its desk line falls through to `(no answer)` —
honest rather than lossy, because the member did spend a turn without saying
anything to the room.

## When a line becomes an aside

`EpisodeDriver::aside_audience` is the gate, and it runs **before** the row is
appended, because an audience is fixed at append time: widening one afterwards
could never be redelivered (a sharing watermark advances past filtered rows
unconditionally) and would invalidate every citation naming it.

Every rung below means **the private row is dropped**. The turn's own
desk-visible line is unaffected and still reaches the room — the member has
already said its piece — and a refusal is logged, never raised: none of these is
a reason to abandon an episode. Dropping rather than publishing is the point:
falling back to the desk would put a second desk-visible contribution on one
turn.

| Condition | Behaviour |
| --- | --- |
| The reply carries no `!aside` line | No private row to write |
| `aside = { enabled = false }` (the default) | Dropped — `Disabled` |
| The author is not an active member of this desk | Dropped — `AuthorNotOnDesk` |
| The line names no agent — `@#desk`, `@everyone`, or nobody | Dropped — `NoAudience` |
| It names only its own author | Dropped — `SelfOnly` |
| It names more peers than `max_members` | Dropped — `AudienceTooLarge` |
| A named peer is not an active member of this desk | Dropped — `TargetNotOnDesk` |
| This pair has spent `max_messages` | Dropped — `BudgetSpent` |
| `must_surface`, and this pair's last aside never surfaced | Dropped — `UnsettledAside` |
| Anything else | **Aside**, addressed to the named peers |

A first addressed id that cannot be resolved **stops** the decision rather than
being skipped: an audience assembled by quietly dropping the names it could not
resolve is not the audience the author wrote.

The two `hive-report` rows — a failed turn, and the episode's close — are always
desk-visible. A reader who could not see that a turn failed would be reading a
transcript with a hole in it that nothing accounts for.

## The budget is folded, not tracked

`aside::spent_and_unsettled` folds both figures out of the transcript the
episode already holds, rather than carrying them across loop iterations. That is
the same discipline the transcript itself follows: what the fold counts is
exactly what a person reading the desk would see, so the two can never disagree.

A **party** is the author plus its addressees, sorted and deduplicated, so the
same pair folds to one key whichever of them wrote the row — otherwise one
member's budget could be spent by the other. A party is per pair, not per desk:
`@a→@b` and `@a→@c` are two asides with two budgets.

"Unsettled" means the party opened an aside and no member of it has since
written a desk-visible `!surface`. A `!surface` by **either** member settles it —
the room is owed one settlement, not one per participant — and a `!surface` from
somebody who was never in the aside settles nothing, since they have no debt to
discharge. An ordinary desk turn by a member does not settle it either, or the
requirement would be paid by whatever anybody happened to say next.

`must_surface` is enforced at the **next** aside, not at the last turn. Nothing
compels a settlement before an episode ends, so a room can still close with one
unsettled. Making the episode refuse to converge on an unsettled aside was
considered upstream and rejected as too blunt for a first cut.

## What lands in the journal

No new row shape and no second store. `CompanyEvent::AgentReply` gains one
field:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
audience: Vec<String>,
```

Empty means desk-visible — every row written before this field existed, and
every ordinary turn now. The list holds the **addressees only**; the author's
admission to its own row is the library's rule (`Audience::admits`), not a
member of the set.

Additive on exactly the terms `task_id`, `parent` and `mentions` already are:
`#[serde(default)]` is what lets an already-persisted log load, and
`skip_serializing_if` is what keeps a desk-visible reply serializing
byte-for-byte as it did before, so **no stored record needs migrating**.

## The manifest knob

```toml
[[group_chat]]
id = "ops"
members = ["route_planner", "fleet_tech", "stock_controller", "field_realist"]

[group_chat.hive]
enabled = true
quorum = 2

[group_chat.hive.aside]
enabled = true
max_members = 1
max_messages = 2
must_surface = true
```

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | Whether a private line may be opened at all |
| `max_members` | `1` | Largest audience **excluding** the author. A pair. A caucus of three inside a desk of four is not an aside, it is a second desk with no quorum and no record — declare the desk instead |
| `max_messages` | `2` | Rows one aside may carry: a question and an answer. Small because every private row is a row of the desk's turn budget spent where the room cannot read it |
| `must_surface` | `true` | Whether the pair owes the room a settlement before opening another. Without it a pair can hold the whole episode privately and the record is a run of stubs |
| `require_thread` | `false` | Whether an aside must be a thread. **Defaults off here, unlike the library's suggestion**: a hive episode runs on the desk channel, so a desk requiring threads would authorize no asides at all — a confusing way to spell `enabled = false` |

A disabled config discards its own bounds: `AsideConfig::policy` returns
`AsidePolicy::DEFAULT` with every bound at zero, so a desk that is off cannot be
read as a desk that is on with limits.

## See also

- [`hivemind.md`](hivemind.md) — the episode, the loop and the move grammar.
- [`hivemind-referral.md`](hivemind-referral.md) — the same no-vote rule at a
  desk boundary.
- `companies/vending_machine_co` — a bundle whose desks talk to each other.
