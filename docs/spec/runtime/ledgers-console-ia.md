# Ledgers console redesign: naming and information architecture

Filed as issue #1284, from a design discussion between the operator and their
assistant. This is a **surface** redesign only. The engine
(`docs/spec/runtime/ledgers.md`: declared `LedgerSpec`, the append-only fold,
bounds-are-code, one derived Markdown file per list) does not change — nothing
here alters `POST …/ledgers`'s request or response shape, the fold, or the
board's drag-and-drop. What changes is what the operator sees: the word they
read, where each list sits, and how a new one is declared.

## Why: three surface problems, one engine

1. **"Ledger" reads as a financial record** to almost anyone who did not build
   it. What it actually means — any tracked list of rows with a status
   (goals, decisions, risks, a hiring pipeline, customer promises) — is a far
   more useful concept than the name suggests, and the mismatch makes a
   first-time operator skip past the feature that would have answered "where
   do we track X?"
2. **`tasks` gets a nav row of its own; every other list does not.** `tasks`
   is a ledger under the hood (`LedgerSource::Native`,
   `docs/spec/runtime/ledgers.md#the-task-board-is-the-tasks-ledger`) but the
   console still treats it as special-cased chrome around the board, while
   `goals`, `decisions`, and anything a company declares are buried one level
   down inside a single "Ledgers" nav item, picked from an in-page list.
   Nothing about the engine draws that line — it is a console artifact left
   over from before issue #1140 merged the Tasks page into `LedgersView`.
3. **Declaring a list means authoring `fields[]`/`statuses[]` as JSON.** An
   operator who wants to track "customer promises" thinks in terms of "make a
   list," not "declare a schema."

## Rule 1: "Ledger" is an internal word only

Every user-facing surface names a list by its own title — **Tasks**,
**Goals**, **Decisions**, or whatever a company called something it declared.
"Ledger" does not appear in a nav label, a page heading, a button, a dialog
title, an empty state, a toast, or help copy anywhere in the console.

It stays exactly where it already is useful: module and file names
(`src/ledger/`, `LedgerSpec`, `LedgerStore`), Rust and TypeScript type and
function names (`LedgerSummary`, `defineLedger`, `listLedgers`), route and API
shapes (`…/ledgers`, `#/ledgers/<slug>`), code comments, and this design
corpus. None of that is operator-facing, so none of it needs to change, and
renaming it would cost a mechanical diff across the engine for zero surface
benefit — precisely the "None of that needs to change" instruction issue
#1284 gives.

A concrete before/after for the strings that do move:

| today | becomes |
| --- | --- |
| nav item "Ledgers" | nav item **"Work"** — the hero surface below, see Rule 2 |
| "New ledger" button | "New list" (moved — see Rule 3) |
| "Declare a ledger" dialog title | wizard, titled by its first step (see Rule 4) |
| "This ledger leaves this screen…" (retire confirm) | "This list leaves the sidebar…" |
| "This company has no ledgers yet." | "This company has no lists yet." |
| "Rows here are opened elsewhere" banner | unchanged in substance, reworded off "ledger" |

## Rule 2: one nav row, a Tasks-hero page, every other list a tab

This rule went through three drafts before landing; the first two are kept
below because the reasoning that ruled them out is the part worth not
re-litigating.

### Draft 1 (rejected): every list its own sidebar row

The first cut took "no list should be a click further from the sidebar than
any other" literally: `NAV` stopped being a fixed array and grew one row per
list the company held, spliced in where the single "Ledgers" row used to sit.

It did not survive contact with the cap. A company may declare up to
`MAX_DECLARED` = 12 lists (`src/ledger/spec.rs`) on top of the 3 built-ins
(`tasks`, `goals`, `decisions`) — 15 list rows, sharing the sidebar with the 8
other fixed `NAV` entries (Overview, Company, Chat, Brain, Workspace,
Approvals, Workflows, Settings). Twenty-three rows in one sidebar is not a
list of destinations an operator scans, it is a wall. The draft read fine
during review only because it was checked against a demo company holding
exactly three lists — the cap itself was never rendered and never looked at.

### Draft 2 (rejected): a collapsible "Lists" section

The second cut tried to keep flat, always-visible rows while bounding their
count: a collapsible sidebar section, one header row, with every list but
Tasks as an indented, persistently-collapsible child. Tasks stayed a
top-level row outside it.

This was abandoned before it shipped, for a reason upstream of scaling: a
sidebar section — collapsible or not — is still built on the premise that an
operator's daily relationship to a declared list is *finding it in the nav*.
That premise does not hold. `goals`, `decisions`, and anything a company
declares are `LedgerSource::Events`, written almost entirely by **agents**
through `record_entry`/`close_entry` (`docs/spec/runtime/ledgers.md`); a
person's own relationship to them is closer to "check what we decided" than
"work out of this every day." `tasks` is the one genuine exception —
`LedgerSource::Native`, its rows live in the task store, its columns fire
dispatch, and an operator works out of it constantly. Reserving permanent nav
real estate for surfaces that are consulted rather than worked was solving
the wrong half of the original complaint. The actual defect Draft 1 correctly
identified — that reaching Goals or Decisions meant an in-page picker of bare
cards that said nothing about what was in them — was never "not enough nav
rows"; it was "the one existing entry point told you nothing useful before
you clicked."

### What ships: one nav row, Tasks as the hero, everything else a tab

`NAV` keeps exactly the one row it always had for this surface, relabeled
**"Work"** (see the label discussion below) rather than "Ledgers". Clicking it
opens a single screen shaped like the board always was — a tab strip across
the top, Tasks first and selected by default, one tab per other list the
company holds, each tab carrying its own open count so the strip is worth
glancing at before clicking anything:

```
┌ Work ──────────────────────────────────────────┐
│ [ Tasks 12 ] [ Goals 3 ] [ Decisions 1 ] [More▾]│
├──────────────────────────────────────────────────┤
│  the selected tab's board or list, exactly as    │
│  LedgersView already renders it                  │
└──────────────────────────────────────────────────┘
```

This keeps the one property both earlier drafts were chasing — opening any
list is one click, direct, never through a picker of cards that says
nothing — while paying a **fixed** nav cost (one row) regardless of whether a
company holds 3 lists or 15. The tab strip absorbs what scaling cost there is,
and it is built to: tabs that do not fit the available width collapse behind
a **"More ▾"** menu computed from measured widths, not a hardcoded count, so
the strip degrades the same way at a narrow viewport as it does at the
12-declared-list cap. See the implementation plan for the measurement
approach.

**Why "Work" as the label.** It cannot say "Ledgers" (Rule 1), it cannot
imply the surface holds only Tasks (it also holds Goals, Decisions, and
whatever the company declared), and it cannot bury the word "Tasks" so deep an
operator hunting for the board cannot find it by scanning nav labels — Tasks
is the first tab, on screen the instant this row is clicked, with zero further
clicks. "Work" reads as the neutral container that is genuinely true of
every tab under it: the board *is* "the company's work board" already, in
its own screen's copy, and Goals/Decisions/whatever a company declares are
just as fairly described as records of the company's work as Tasks is.

**Routing.** `#/ledgers/<slug>` is unchanged — it always could name any list,
tab or not — and the tab strip is a rendering of the same address space, not
a new one. A bare `#/ledgers` (the nav row's own address, and what a fresh
click on it produces) resolves to the Tasks tab; `#/ledgers/<slug>` opens
directly on that list's tab, so a deep link to Goals lands on Goals rather
than on Tasks with an extra click to get there. `#/tasks/<id>` — the card
detail page, the one address issue #1140 kept alive when the standalone Tasks
page was retired — is untouched: it is a different route entirely (`view:
"tasks"`, not `"ledgers"`), reached from chat, an approval card, or a
workflow row exactly as before, and this redesign does not touch it.

## Rule 3: declaring a list moves out of the main nav

Today "New ledger" is a button in `LedgersView`'s own toolbar, reachable
the moment an operator opens what used to be the single Ledgers page. The
tab strip Rule 2 lands on is a screen for *working a list's rows*, tab by
tab — putting a "New list" control there would mean an operator sitting on
the Goals tab sees a button for creating an unrelated new list, which is a
settings action wearing a data-page's chrome, and it would have to be
findable from whichever tab happens to be open rather than from one obvious
place.

This follows the precedent `CompanyView` already set for desks
(`frontend/src/views/company/CompanyView.tsx`): the Company page
(`#/company`) is the roster, and desk creation, deletion, and restaffing live
one click away at `#/company/desks` (`OrgChartView`), reached through a
"Manage Desks" button on the roster. Lists get the same shape:

- The Company page grows a **Manage Lists** button beside Manage Desks,
  opening `#/company/lists`.
- That page is where a list is created (the wizard, Rule 4), and where a
  company-declared list is retired. Both are "manage the set of lists that
  exist" actions, as distinct from "work the rows of one list" — the same
  line `CompanyView`'s doc comment draws between the roster and the chart.
  Retiring currently lives on each list's own toolbar
  (`LedgersView`'s "Retire" button, gated on `!ledger.builtin`); it moves to
  Manage Lists alongside creation, so a list's own screen is only ever about
  its rows — search, filter, board/list toggle, compose, delete a row — never
  about the existence of the list itself.
- A list's page keeps the two other things `docs/spec/runtime/ledgers.md`'s
  console section already documents as deliberately visible rather than
  hidden: row-level delete (person-only, "Close" offered first), and a native
  list's `writtenBy` sentence in place of a compose box.

Manage Lists shows every list the company holds — built-in and declared,
including `tasks` — each with its title, purpose, row counts, and whether it
is retireable. `tasks`, `goals`, and `decisions` cannot be retired (they are
not `LedgerSummary.builtin === false`), so the row shows why rather than
hiding the control inconsistently with how Manage Desks always shows every
desk.

## Rule 4: the declare dialog becomes a plain-language wizard

The current `DeclareDialog` (`frontend/src/views/LedgersView.tsx`) is a
`Textarea` seeded with a worked JSON example (`TEMPLATE`) and a "Declare"
button that calls `JSON.parse` on whatever the operator typed. Its own doc
comment says why it was built that way — "the declaration is small, the field
roles matter, and a wizard that produced a subset of what a teammate's
`define_ledger` can produce would leave the console unable to express a
ledger it can display." That tradeoff is real: `LedgerSpec` supports roles
the wizard below does not surface (`refs`, `number`, custom `sections`,
`checks` beyond the built-in three). The wizard is deliberately a **curated
subset** of what the wire format allows, not a full re-expression of it — an
operator who needs `refs` or a custom section order still has the same POST
body reachable by whoever builds agent tooling against `define_ledger`
directly; the console's wizard optimizes for the common case a human names in
one sentence.

The wizard replaces the JSON editor with four steps, each producing a piece of
the same `LedgerSpec` the host already accepts:

1. **What do you want to track?** — free text, becomes `purpose`. ("What we
   promised a customer, and whether we did it.")
2. **Name it** — free text, becomes `title`; the `slug` is derived from it
   (lowercase, `-`-joined, checked against the existing registry the way
   `OrgChartView` slugifies a desk name) with the slug editable if the
   derived one collides or reads badly.
3. **What stages does a row go through?** — becomes `statuses[]`. Two
   presets front and center:
   - **To do / In progress / Done** (`todo`, `in_progress` → `done`, closed,
     `needs_reason` off — a task-shaped list)
   - **Open / Closed** (`open` → `closed`, closed, `needs_reason` on — an
     event-shaped list, the shape `TEMPLATE`'s `customer-promises` example
     already uses for kept/broken)
   plus a **Custom** path: add named stages, mark which end the row
   (`closed: true`) and which of those need a reason
   (`needs_reason: true`) — the same two flags the JSON template already
   sets by hand, asked as two checkboxes per stage instead.
4. **What details does each row need?** — becomes `fields[]`. A title field
   is implicit and always present (`role: "title", required: true`, plus the
   `id` and `status` roles the engine always needs — the wizard fills those
   without asking). Presets offered as toggles: **Owner** (`role: "owner"`),
   **Notes** (`role: "prose"`), **Due date** (`role: "date"`) — the three
   `TEMPLATE`'s worked example already reaches for beyond title/status/reason
   — plus **Add a custom field** for anything else, asking for a name and
   picking from the same role list `FieldRole` already exposes
   (`frontend/src/api/ledgers.ts`). A closing reason field
   (`role: "prose"`, conventionally named `reason`) is added automatically
   whenever step 3 marks any status `needs_reason` — the wizard does not ask
   for it separately, since a status that needs a reason and a spec with
   nowhere to write one is exactly the mistake `TEMPLATE`'s own comment
   flags as "the commonest mistake."

A final review step shows the assembled plain-language summary ("Customer
promises, tracked open → kept/broken, each row has a customer and a due
date") rather than the JSON — the wizard's job is to make the *shape* legible
without asking the operator to read `LedgerSpec` to check its own work — and
submits the same object `defineLedger()` already POSTs today. `sections` is
assembled by the wizard, not asked about: one section per non-closed status
group (open stages) plus one "Settled"-style section for the closed ones,
mirroring the shape `TEMPLATE`'s own `Outstanding`/`Settled` split already
uses, so the operator never has to think about section headings to get a
working list. `checks` is fixed to
`["required-field", "known-status", "closed-needs-reason"]` — the same three
`TEMPLATE` ships — since nothing in the wizard's four steps produces a
`LedgerSpec` those three would ever reject.

## Rule 5: the board and drag-and-drop are untouched

`LedgerBoard` (`frontend/src/views/LedgerBoard.tsx`), the board/list toggle,
`patchTask` for the native board vs. `record_entry` for every other list, the
empty-column rail collapse (issue #1101), and the drag mechanics (issue #334)
are unchanged by this redesign. A list's own screen still renders through
that one shared component; only how the operator arrives at that screen
(Rule 2) and how the list came to exist (Rules 3–4) move.

## What stays out of scope

- `LedgerSpec`, the fold, `LedgerStore`, the derived-Markdown guard, and every
  Rust type under `src/ledger/` — unchanged.
- The five agent tools (`list_ledgers`, `read_ledger`, `record_entry`,
  `close_entry`, `define_ledger`) and their schemas — unchanged. The wizard is
  a console-only path to the same `define_ledger`/`POST …/ledgers` call a
  teammate's tool call already reaches.
- `[[agent]].ledgers` manifest semantics — unchanged.
- Row-level behavior on a list's own screen (search, status filter, compose,
  delete) — unchanged, only relabeled per Rule 1.

## Where the engine doc still applies

`docs/spec/runtime/ledgers.md`'s "## The console" section describes how a
list's *own* screen renders from its `fields`/`statuses`/`sections` (the
board/list duality, the task-card slot, the empty-rail collapse, the two
board specs) — all of that remains accurate after this redesign and is not
duplicated here. What that section's opening paragraphs describe as reached
through "The Ledgers section" is superseded by Rules 1–3 above: there is no
longer one section named Ledgers, and this document is the one to update
first if the IA changes again.
