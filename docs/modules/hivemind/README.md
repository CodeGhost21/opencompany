# Hivemind Module

`src/hivemind/` turns a desk with two or more members into a room: an operator
message addressed to it runs a bounded deliberation episode instead of one
teammate's turn. The normative account — when a room opens, what the prompt
looks like, what lands in the journal, the manifest keys — is
[`docs/spec/runtime/hivemind.md`](../../spec/runtime/hivemind.md). This page is
the module's own shape and the reasoning behind its boundaries.

## Layout

| File | Holds |
| --- | --- |
| `types.rs` | `HiveConfig` (the `[[group_chat]].hive` block), `HiveMember` / `HiveDesk` (the snapshot an episode runs over), `HivePolicy` (config + size → `EpisodePolicy`), `EpisodeEnding` / `EpisodeOutcome`, and `desk_episode` — the gate |
| `log.rs` | `EventLogSessionLog`: the company journal read as a `tinyhivemind::SessionLog`, narrowed to one desk |
| `prompt.rs` | `EpisodePrompt` (what one authorized turn is shown) and `marker_line` (what its answer contributes) |
| `episode.rs` | `EpisodeDriver` (the host loop) and `HiveTurnRunner` (the one-function turn seam) |
| `test.rs` | the module's unit tests |

## Why it is ungated

The module is compiled in every build, unlike `src/harness/`. Two reasons:

1. **The dependency is pure.** `tinyhivemind-hive` is serde + thiserror,
   executor-neutral, no ports, no storage, no network. There is nothing in it
   the default build would want to shed.
2. **The decision is a routing decision.** "Does this desk answer as a room?"
   is the same class of question as `desk_lead` and `chat_responder`, both of
   which live outside the harness precisely so a non-harness build can answer
   them. Putting the gate behind `#[cfg]` would put a routing rule in one build
   and not another.

Only the *hook* is gated, because only the harness has turns to run.

## The seams, and why they are where they are

**`HiveTurnRunner` is one function wide.** An agent id and a prompt in, one
reply out. The production implementation (`HiveDeskRunner`, in the brain) is a
plain `RunTurn::run`; a test scripts replies. Anything wider would have started
to encode what a turn *is* into this module, and the entire point is that a
deliberating turn and a single-responder turn are the same turn.

**The driver takes a `HiveDesk` snapshot, not a `&CompanyRecord`.** An episode
is several turns long. A room whose membership changed underneath it would hand
the floor to somebody the earlier turns never saw, and the roster the library
validated against would stop matching the desk it was validated for.

**The log adapter holds no roster.** It is opened for one desk and reads rows
written by teammates who may since have left it, so an id is its own label
there. A seated member's display name is applied by the prompt, which does hold
the roster.

## Two contracts that are easy to break

**Append before commit.** `step` returns a `next_state` that is only valid once
the turn it authorized is durably journaled. `EpisodeDriver::run` appends, then
assigns. Reversing those two lines would let a failed write leave the room
believing in a turn nothing can read back.

**The page contract.** `SessionLog::read_before` must return rows newest-first,
strictly descending, no larger than asked, with an exclusive cursor no newer
than the oldest row — and an *empty page must carry no cursor*, because an
empty page means the log is finished. A company journal carries approvals, task
cards and workflow runs between a desk's rows, so a single raw chunk can easily
hold no chat at all; the adapter therefore loops over raw chunks until it has at
least one qualifying row or the journal runs out. Returning the empty chunk
would truncate the transcript at whatever the last busy stretch of the journal
happened to be.

## Running the tests

```bash
cargo test --features openhuman hivemind
```

The module's own tests need no feature (`cargo test hivemind` covers them); the
brain's routing tests need `openhuman`. Nothing in the suite makes a model call:
the log adapter is exercised against an in-memory journal, and every episode is
driven by a scripted runner, so a three-agent room converging is a deterministic
assertion rather than a live-run hope.
