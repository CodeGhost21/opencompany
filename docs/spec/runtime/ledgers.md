# Dynamic ledgers

A company keeps more than a task board. It keeps goals, decisions, risks,
customer promises, experiments run, invoices chased — and every one of those is
the same shape: **a set of rows with an id, a status, some prose, and a reason
each closed one closed.**

Before this, exactly one of those shapes was first-class. `TaskStore` had six
hard-coded columns, and every other axis a company needed was written into a
note, a chat message, or a workspace file nobody designed. The company's own
record of what it had decided was prose scattered across three surfaces, and
nothing could search it, bound it, or stop a teammate re-deciding it next week.

So the shape is **declared** rather than compiled. A `LedgerSpec` names its
fields, its statuses, which of them close a row, and how the rendered file is
laid out; one engine folds an append-only log into rows and renders a built-in
and a company-declared ledger identically. The runtime ships three
(`tasks`, `goals`, `decisions`) and a company writes the rest, up to
`MAX_DECLARED` (12).

## The four rules

### Append-only, and every write is a merge

There is one write operation. Opening a row, amending it, blocking it, closing
it and re-prioritising it are all *record an event against this id*; the fold
applies events in order and leaves absent keys alone. Closing a goal is
`{status: "met", reason: "shipped in March"}` and nothing else.

That is not simplification for its own sake. A vocabulary of operations means a
vocabulary of **inverses** to get wrong — what `unblock` does to a row that was
never blocked, whether `reopen` restores the old status or the default — and
every one of those is a decision the fold would have to make silently. A merge
has no inverse to get wrong, and the log keeps the history either way.

It also fixes what a rewritten-whole board loses. Storing *current state* loses
**why** (a card that moved to Done records the landing, not the verdict) and
loses **what was there before** (a rewrite that drops a row is byte-identical to
a rewrite that never had it, so a bug that eats work looks exactly like work
nobody did).

A JSON `null` clears a field — the one thing a merge cannot otherwise express.

### Agents create and record; only people delete

`AuthorKind::may_delete` is that rule, in one place. It is not a permission
setting and it is not configurable.

An agent's whole relationship with a ledger is **additive**: it opens rows,
amends them, and closes them with a reason, and every one of those is
recoverable by reading the log. Deletion is not, and a runtime where a turn can
erase the record of what it did is one whose record means nothing. Being
finished with a row is `close_entry`, which keeps the reason — and the reason is
the entire value of a closed row to whoever reads it next.

`AuthorKind::System` is deliberately **not** exempt: a sweep that could delete
rows is the same loss with nobody to ask about it.

Concretely:

| | agent | person | platform credential |
| --- | --- | --- | --- |
| read, record, close | yes | yes | yes |
| declare a ledger | yes | yes | yes |
| delete a row | no | **yes** | no |
| retire a ledger | no | **yes** | no |

The asymmetry on *declare* is the point. A company discovers which axes it needs
while it is running, so a declaration that required an operator would be
discovered and then not made. What an agent cannot do is undo one.

Enforcement lives in `company::ledgers` and nowhere else. The REST routes turn
`ScopedCompany::actor` into a `LedgerAuthor` and the agent tools stamp the
teammate — both then call the same service. A route that decided for itself
would be a second answer to the question, and the tools would need a third.

### Everything renders into `derived/`, and nothing hand-writes it

Every ledger renders one Markdown file into `derived/<NAME>.md` in the company's
shared workspace, rewritten on every write to that ledger. That is what makes a
ledger legible to everything that already reads the workspace — an agent's file
tools, the console's workspace view, a search, an export — without any of them
learning a new API.

The rule is not *these particular files are generated*; it is **nothing in this
folder is hand-written**. A per-file rule fails open: a ledger declared next week
renders a file no guard has heard of, somebody edits it, the edit is silently
erased by the next derivation, and they have no way to know. The folder rule
fails closed.

`DerivedGuardWorkspace` is that guard — a `WorkspaceStore` decorator wrapped at
the single place the store is chosen, so every writer obeys without knowing it
does (the same argument `QuotaEnforcedWorkspace` and `WorkspaceAnnouncer` make).
It refuses `write`, `create`, `adopt_or_create_folder`, `rename_move` (in **and**
out), and the binary writes. `WorkspaceOrigin::Seed` passes, which is what the
runtime's own derivation stamps.

**Deleting is deliberately allowed.** A delete is not the failure this exists to
prevent: nothing is silently lost, the next write re-derives the file, and a
retired ledger has to leave something somebody can clear.

The refusal is **per ledger**, not generic, because *what to do instead* differs:
an events ledger takes `record_entry` and the task board does not. Telling the
board's caller otherwise sends them to a tool that refuses them a second time —
a refusal naming the wrong remedy is barely better than one naming none.

### Bounds are code, not intent

A ledger's rendered file is read by people in the console **and** routed into
agent turns, so its size is a bill paid on every read. Every section is clamped
against `budget::MAX_LISTED` (40 rows) and every prose field against
`REASON_CHARS` (600) **on the way in**, so a declaration cannot grow its own file
past what a reader is asked to hold. Clamped rather than refused, so a ledger
stored when the bound was looser keeps rendering after the bound tightens.

Either bound alone is the same file by another route: forty rows of
five-kilobyte prose is not bounded. The ceiling test asserts both, and asserts
the property a ceiling alone cannot catch — *past the bound, more rows must not
mean more file*.

A section cut to its bound while reading as complete is worse than a long one,
because the reader concludes there is nothing more and re-proposes what was cut.
So every truncation says how many it dropped and names the call that fetches
them.

## The task board is registered, not re-implemented

`tasks` is `LedgerSource::Native`: its rows stay in `TaskStore` and its columns
keep firing dispatch, planning passes and run settles. None of that is
expressible as a declaration and none of it should be — a declared status cannot
open an attempt, and a company that could redefine `in_progress` into something
that does not dispatch has broken its own runtime from a JSON file.

It is registered anyway so `list_ledgers` names **every** ledger and
`read_ledger` reads every ledger. A discovery surface that covers the ledgers a
company invented but not the one it already had is a surface an agent stops
trusting, and then stops using.

Its statuses are `BOARD_COLUMNS` verbatim, and `done` is the only closed one: a
card in review or paused is *stopped*, not finished, and calling either closed
would make "what is still outstanding" answer wrong.

## What a declaration cannot do

- **It cannot reason.** `Check` is a closed set — a required field, an unknown
  status, a close with no reason. There is no expression language: a company
  that could write predicates into a ledger declaration has written a rules
  engine nobody can review.
- **It cannot raise a bound** (see above).
- **It cannot shadow a built-in**, so `tasks` is always the board and every
  prompt and route naming it stays right.
- **It cannot claim another ledger's derived path.** Two writers on one file is
  how each one's work disappears.
- **It cannot be `native`**, which is for ledgers the runtime renders in Rust.

A declaration that breaks one of these is skipped **with a fault** rather than
failing the registry: a company that wrote one bad ledger must still reach its
board. The faults ride the listing so the company can see why something stopped
appearing.

## The agent surface: five tools, however many ledgers

`list_ledgers`, `read_ledger`, `record_entry`, `close_entry`, `define_ledger`.

The count does not grow when a company adds an axis, and that is forced rather
than chosen: the tool schema is built once when the agent is constructed, so a
ledger declared mid-run can get no tool of its own and can appear in no `enum` in
anybody's schema. `ledger` is therefore a plain **string** checked against the
registry at call time, and an unknown slug comes back with the real ones — the
discovery path a model actually follows, in one turn, without having thought to
list them first.

Reads are `ReadOnly`; the three writes are `Write`, so a supervised policy parks
them like any other consequence. There is no delete tool and no retire tool.

The prompt carries a **catalogue** — every ledger named with its purpose — not a
sentence saying `list_ledgers` exists. A tool granted, unmentioned and never
called is the observed failure mode, not a hypothetical one. The catalogue is
built from the registry resolved at agent-build time, so a ledger declared
mid-run is reachable by every tool immediately and named in the *prompt* from the
next build. That is the honest limit: system prompts are assembled once, and
nothing can retroactively edit one already in flight.

## Storage

`LedgerStore` (`ports/ledgers.rs`) keeps two things with very different
lifetimes: **declarations** (small, rewritten in place, at most 12) and
**events** (append-only, unbounded). Built-ins are never stored — they ship with
the runtime, and persisting a copy would let a company's stored version drift
from the code every prompt and route is written against.

The store's only ordering obligation is that `events` returns what `append`
appended, in that order. Everything the fold promises rests on that and on
nothing else — in particular not on an event's timestamp, which is written by
whichever replica is running.

All three backends implement it:

| backend | declarations | events |
| --- | --- | --- |
| fs | `ledgers.json` | `ledgers/<slug>.jsonl`, one `write_all` per line under `O_APPEND` |
| sqlite | `ledger_specs` | `ledger_events`, ordered by `AUTOINCREMENT seq` |
| mongodb | `ledger_specs` | `ledger_events`, ordered by a per-company counter |

One event file per ledger rather than one shared log: the fold reads a single
ledger at a time, and a shared file would make every read of the goals scan every
task event ever written.

`delete_spec` leaves the events alone. A ledger nobody reads is worth retiring;
the work recorded in it is not, and deleting a log to tidy a registry is exactly
the loss the append-only shape exists to prevent. Deleting the rows too is
`purge_ledger` — a separate, deliberate act, and a person's.

## The console

The Ledgers section renders from the ledger's own `fields`, `statuses` and
`sections`, never from anything hard-coded. A ledger a teammate declared this
morning renders correctly this afternoon with no console release; a screen that
hard-coded the goals columns would have made "declare your own axis" a promise
the UI quietly broke.

Two things it shows rather than hides. The delete control exists here and nowhere
an agent can reach, with **Close** offered first as the ordinary way to be
finished with a row. And a native ledger renders its `writtenBy` sentence in
place of a compose box, rather than offering a form whose save the host refuses.

The compose form reads `needsReason` off the declaration and asks for the reason
*before* the save — the same rule the host enforces, met earlier.
