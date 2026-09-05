# Hive-mind desks

A desk with somebody to deliberate **with** answers an operator message as a
room rather than through one responder.

Before this, a message addressed to a `[[group_chat]]` selected exactly one
agent off a deterministic ladder — a `@mention`, else an `auto` channel's
per-message pick, else the desk lead — that agent took one turn, and the
interaction ended. That is still the whole story for a desk of one, a DM, the
General line, and a workflow copilot thread. For a desk of two or more it is
now the fallback rather than the rule: the message opens an **episode**, and
the episode runs a bounded sequence of single turns until the room converges on
an option, deadlocks between two, spends its turn budget, or finds it has
nothing to say.

The mechanics come from [`tinyhivemind`](https://github.com/tinyhumansai/tinyhivemind),
vendored at `vendor/tinyhivemind` and linked unconditionally: it is pure Rust,
executor-neutral, and adds no port. `tinyhivemind_hive::step` is a fold over a
transcript this host already holds; it never appends, never waits, and never
calls back.

## One message is still one turn at a time

An episode is not a fan-out. `step` authorizes exactly one speaker per call, so
the number of turns an operator message can start is bounded by the desk's turn
budget and by nothing else — the same bound a single-responder desk has at 1.

What a room buys over one responder is not parallelism. It is:

- **Independence.** The opening round is *blind*: a member forms its own
  position before it can read its peers'. A shared transcript destroys
  independence — the third speaker has already read the first two — and this is
  the cheapest available repair, costing a projection flag rather than any
  concurrency.
- **A reason to stop.** The episode ends on a quorum it can name, not when one
  agent decides it is finished.

It is deliberately *not* a quality claim. Conformity in a group of language
models rises with interaction time, so a long episode buys correlated error
rather than better judgement. That is why the default budget is small and why
raising it should be a decision rather than a default.

## When a room opens

`crate::hivemind::desk_episode` is the single gate, and every rung is a reason
**not** to open one:

| Condition | Behaviour |
| --- | --- |
| The message names a teammate (`@mention`) | Single turn — naming somebody addresses them, not the room |
| No addressed chat (`chat: None`) | Single turn — the company's own line, answered by the orchestrator |
| A General spelling (`""`, `main`, `general`, `General`) | Single turn |
| A `dm:<teammate>` key, or a bare roster id | Single turn |
| A key that resolves to no desk | Single turn |
| A workflow copilot thread | Single, **confined** turn — it returns before this gate |
| A desk with fewer than two effective roster members | Single turn |
| `hive = { enabled = false }` on the desk | Single turn |
| Anything else | **Episode** |

Membership is read through `CompanyRecord::effective_desk_members` — the same
source `desk_lead`, `desk_default_responder` and the console's desk list read —
so who is in the room and who the console says is in it cannot drift. Overlay
(operator-created) desks are included, and take the default config.

A one-teammate-per-desk company therefore behaves byte-for-byte as it did.

## The loop

```text
fold the desk transcript  →  step()  →  Speak?  →  run one ordinary turn
        ▲                                   │              │
        └───────── commit next_state ◀──────┴── append the line to the journal
```

The ordering is the contract. `step` hands back a `next_state` that is only
valid once the turn it authorized is **durably** in the journal; committing it
first would let a failed write leave the room believing in a turn nothing can
read back.

The transcript is re-read out of the journal on every iteration rather than
accumulated in memory. That is the same discipline seen from the other side:
what the room folds is exactly what a human reading the desk would see, so the
standings can never disagree with the transcript they came from.

## Each turn is an ordinary turn

The episode supplies the prompt and decides who is asked. Everything else is
unchanged: the answering teammate runs on its own harness, with its own tools,
its own `memory_store` / `memory_recall` / `memory_forget` belt, the same
retrieve→inject→store memory loop every turn runs, and the same approval gate.
The seam is one function wide (`HiveTurnRunner::speak` — an agent id and a
prompt in, one reply out), which is what lets a test drive a whole episode with
no model at all.

Model tiers are unchanged too: a member's `[[agent]].tier` still selects its
model through `[inference].models`, so a room is seated with specialists by
declaring them in the manifest, not by anything here.

## The prompt

Every block is a port of the reference host's own builder, and each exists
because a live room failed without it. A driver can predict exactly what an
agent sees from the transcript plus the turn's visibility:

```text
You are @<id>, the <role> on the <desk> desk. <sight sentence>

This desk is for: <desk description>            (when the manifest says)
In the room with you: @a (Role A), @b (Role B). Address them by the ids above.
The operator asked the desk:
<the operator's message, verbatim>

Pinned on this desk, whatever else has scrolled away:   (when the board holds any)
[12] #label opening words of the pinned message

<the phase's move list, then the rules those moves are read under>

<the options on the floor, each with its standings>     (or "no option yet")
You already said this, so do not repeat it — …          (when it has spoken)
Shared attributed transcript:
[7] operator: …
[9] planner: !propose #stage …

Your one line:
```

- **The move list is phase-gated.** `!commit` is absent while the room is
  deliberating: the library authorizes a commit turn by setting the phase, and
  a `!commit` deposited before that adds no supporter to anything — a room that
  reaches for it early spends its whole budget recording a decision it never
  reached.
- **The floor carries standings**, folded with the same `standings` the episode
  decides on. Two live failures come from a member not seeing them: models coin
  a fresh id for an idea the room already named (`#rollout` and
  `#rollout-strategy` in one episode) and support split across two names never
  adds up; and a member that cannot see how far an option is from carrying has
  no way to know one more supporter would settle it.
- **A member is shown its own last line.** Live models restate their previous
  line verbatim when they have nothing new; `repetition_cap` damps a restated
  *support* and cannot see this at all.
- **The pinboard is rendered.** The window is thirty messages, so what the desk
  settled long ago is otherwise gone. A `!pin` is how it stays unavoidable, and
  the board is folded fresh each turn — a pin laid down *during* the episode is
  on the board for the next speaker.
- **Blind turns see less.** Under `Visibility::Blind`, peers' episode messages
  are withheld; the operator's task, system rows, the member's own lines, and
  everything at or below the watermark remain.

## The grammar

A marker is recognised at the start of a line, outside fenced code blocks.

| Move | Means |
| --- | --- |
| `!propose #topic …` | puts a new option on the floor |
| `!support #topic ^N …` | backs an option, citing message N as grounds |
| `!object >N ^M …` | objects to message N, citing message M |
| `!evidence #topic ^N …` | adds grounds without taking a side |
| `!question …` | asks for what nobody has established |
| `!defer #topic …` | stands aside; costs the turn, adds no support |
| `!commit #topic ^N …` | records the decision — **commit phase only** |
| `!pin` / `!unpin ^N` | folds the desk's pinboard |

The `#` and the `^` are part of the grammar: `!propose canary …` names nothing
and is discarded, and a support with no citation does not count.

## What lands in the journal

Nothing new. There is no episode record and no second store — the transcript
*is* the episode, which is what makes the standings impossible to disagree with
the conversation they were folded from.

| Row | `chat_id` | `agent_id` | `text` |
| --- | --- | --- | --- |
| each turn | the desk id | the teammate that spoke | its one marker line |
| the close | the desk id | `hive-report` | how the episode ended |

Every row carries the same `parent` a single-responder reply would: the
operator message's own parent, so an answer joins the thread its question was
asked in rather than opening one underneath it. In the desk channel that is
`None`, and it is load-bearing there — the library's channel projection
promotes each root and each root's *first* reply, so turns parented to the
operator's message would be invisible to the rest of their own episode.

Only the **marker line** is journaled, not the whole reply. A harness turn can
return a paragraph however firmly it was asked for one line, and the whole
paragraph would then be rendered back into every subsequent prompt — thirty of
those is the transcript window spent on one answer. The marker is what the room
counts, so the marker line is what the transcript keeps; prose with the marker
buried in it is unwrapped, and a turn that deposits no trace falls through to
the first thing it actually said.

`hive-report` is hyphenated on purpose, exactly as `workflow-report` is: both
id minters reject a hyphen, so no roster teammate can ever hold the id and the
room's own summary can never be misattributed to one. The log adapter reads
those authors back as **system** rows, which is load-bearing in both
directions — a system row is never counted as a supporter, and is never hidden
by a blind round.

The episode journals its own turns, and the brain deliberately raises **no**
chat bubble for them: the REST chat route journals every response it is handed,
so a bubble would write a second row for a line that is already durable. The
console still sees each turn arrive live — the operator SSE feed projects
journal rows — and a reload reads the same transcript the room folded.

## The manifest knob

```toml
[[group_chat]]
id = "creative"
name = "Creative studio"
members = ["copywriter", "editor", "strategist"]
hive = { enabled = true, turn_budget = 9, quorum = 2, blind_round = true }
```

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | on at two members | `false` keeps the desk on a single responder. `true` cannot conjure a room out of one member |
| `turn_budget` | `3 × members` | hard cap on turns; the episode reports itself exhausted at it |
| `quorum` | `(n / 2 + 1).min(n - 1)` | distinct grounded supporters a topic needs. Clamped into `1..=members` on read |
| `blind_round` | `true` | whether the opening round hides peers' positions |

Both zeroes are rejected at validation rather than clamped — `quorum = 0` would
settle every topic the moment it was proposed, and `turn_budget = 0` opens a
room that is exhausted before anybody speaks — because an operator who wrote a
number meant it, and silently substituting a different one is how a desk ends up
behaving in a way its manifest does not describe.

The defaults are functions of the membership because the membership is the only
thing the runtime reliably knows. A quorum that is a simple majority *and still
leaves somebody outside it* means a decision is never contingent on the whole
room agreeing; for a pair that is exactly one supporter, which is the only
number available.

## How an episode ends

| Ending | Means | The `hive-report` line |
| --- | --- | --- |
| Converged | one topic carried and the room recorded it | "The desk settled on #topic after N turns (backed by a, b)." |
| Deadlocked | two or more carried and nobody broke the tie | "The desk deadlocked after N turns: #a and #b carried together…" |
| Exhausted | the turn budget ran out first | "The desk spent its N-turn budget without reaching a decision." |
| Idle | nobody's urge cleared their threshold | "Nobody on the desk had anything to add, so the room did not open." |

The summary says what happened rather than restating the decision's content:
the argument is in the transcript directly above it, and a paraphrase would be a
second, unattributed account of a conversation that already has one.

## Failure

A turn that fails ends the episode and propagates. The turns already appended
stay in the transcript — they are real — and no closing report claims a
decision the room did not reach. A closing report that cannot be appended is
logged and swallowed: the decision is already durable in the turns above it.

## Where the code is

| Path | Holds |
| --- | --- |
| `src/hivemind/types.rs` | `HiveConfig`, `HiveDesk`, `HivePolicy`, `EpisodeOutcome`, `desk_episode` |
| `src/hivemind/log.rs` | `EventLogSessionLog` — the journal as a `SessionLog` |
| `src/hivemind/prompt.rs` | `EpisodePrompt`, `marker_line` |
| `src/hivemind/episode.rs` | `EpisodeDriver`, `HiveTurnRunner` |
| `src/harness/built_in/brain.rs` | the routing hook and `HiveDeskRunner` |

## Testing

`src/hivemind/test.rs` pins the fold with a `HiveTurnRunner` that returns
strings — the manifest knob, the log adapter, the driver's authors, the blind
projection, and `marker_line`. It deliberately runs no model, so it can say
nothing about whether a *company* deliberates.

`tests/hivemind_e2e.rs` (gated `openhuman`, run by the `rust-gated` lane's
`cargo test --features openhuman --tests`) is that half. It boots a real
company — `RuntimeBuilder`, the embedded harness, the filesystem store, the
HTTP surface, loopback sign-in — and drives it through `POST
/api/v1/company/chat`, the route the console posts to. Only the model is
scripted, and the scripted endpoint is **content-aware**: it reads the prompt
each turn was handed, works out who is speaking and what that member can see,
and answers from that. A member citing `^N` has to find `N` in the transcript
it was given, which is the property a fixed reply queue cannot prove.

| Test | What it proves |
| --- | --- |
| `a_desk_deliberates_and_converges_through_the_fold` | one operator message, three members, one `AgentReply` per turn under the teammate that took it, one closing `hive-report` naming the topic and its supporters, no second responder, and exactly one model call per journaled turn in the same order |
| `the_opening_round_is_blind_and_every_later_line_is_attributed` | the opening round is one blind turn per member and no peer line reaches it — through the transcript, the memory preamble, or the assistant history; later turns render peers as `[seq] <id>: …`, never as the viewer's own words, and each member is shown its own last line |
| `an_objection_silences_an_advocate_and_a_second_topic_carries` | cross-inhibition end to end: a backed option is objected to by message, and the option the room records is the other one — with the silenced advocate absent from its supporters |
| `a_desk_reasons_with_what_it_stored_in_an_earlier_episode` | two episodes: `memory_store` in the first, `memory_recall` in the second, and the journaled line written from what came back |
| `a_desk_reasons_with_memory_held_in_a_remote_engine` | the same claim with the memory ports bound to a CortexDB mock through `StorageSettings`: the write lands on `/v1/experience`, the read is served from `/v1/recall` |
| `a_room_that_settles_on_nothing_reports_itself_exhausted` | the budget is the only bound, and the report says the budget was spent rather than inventing a decision |
| `two_carrying_topics_and_no_objection_deadlock` | two options carry, nobody is left to break the tie, and the close says `Deadlocked` |
| `a_single_member_desk_answers_with_one_ordinary_turn` | the same company's desk of one is never handed an episode prompt: one reply, no `hive-report` |

Two things the file asserts around rather than through, said here rather than
left for a reader to assume:

- **The memory loop's automatic injection is not asserted to contain the
  stored fact.** Its query is the whole incoming message — a multi-kilobyte
  episode prompt — and `store::lexical` ranks by term rarity against it, so
  which memories surface is the ranker's business. The deliberate
  `memory_recall` call is the load-bearing assertion, because it is the path an
  agent controls.
- **The deadlock script keeps supporting during the commit phase.** Two options
  can only carry at once if support keeps arriving after the first one has, and
  a live model that ignores the commit protocol is exactly how a real room gets
  there.

See also [`docs/modules/hivemind/README.md`](../../modules/hivemind/README.md).
