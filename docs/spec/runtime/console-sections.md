# The console's four sections

The sidebar is four rows — **Room**, **Company**, **Connections**, **Flows** —
with the contents of the one you are in listed beneath them. This file is the
record of that decision. It is Rule 8 of
[`ledgers-console-ia.md`](ledgers-console-ia.md) written out, because that file
is at its 500-line ceiling and this is the largest IA change it has seen.

Read Rule 6 there first. It governs what a view without a nav row must be, and
eight views need that call making here.

## Why four

Ten flat rows is not a list an operator scans, it is a wall — the same judgement
Rule 2 made when it rejected a sidebar row per declared list, one screen earlier
in the same column. Ten equal-weight destinations force a read of every label
before the eye can settle, and they grow: every surface added since has argued
for a row, and each argument was individually reasonable.

Four is not a target number. It is what is left when the ten are sorted by the
question an operator is answering:

| The question | The row |
| --- | --- |
| I want to say something, or see what was said | **Room** |
| Something about my company: who, what, where, what it remembers, what it costs | **Company** |
| Can my teammates reach X yet? | **Connections** |
| What does this company do on repeat? | **Flows** |

Everything else is chrome (Settings, Feedback, Discord in the footer; Overview
and Approvals in the window's title row) or is filed under one of the four.

## The sub-navigation is in the SIDEBAR

Finance, Settings and — until this change — Connections each drew a `w-60` rail
inside the content area. That is the wrong place once more than one section has
sub-pages, for two reasons that are not about layout fashion:

- It puts the same kind of list in two different places depending on which
  section you are in — the sidebar for the sections without sub-pages, a rail
  for the ones with — so there is no rule to learn.
- It charges the content pane 240px on every page under it, on a screen that
  already has a sidebar to its left.

`components/sidebar-navigation.tsx` owns the pattern, and all four sections use
it. `views/connections/ConnectionsSection.tsx` kept its dispatch and lost its
rail in the same change that introduced the pattern, so the console never had
two answers at once.

### Not an accordion

The four rows are always visible, always contiguous, always in the same place.
Selecting a section swaps the block **below** them; it does not expand a row in
place and does not displace a row's siblings. Exactly one section's contents are
on screen at a time.

```text
┌─────────────────┐
│ ■ Room          │  the four, fixed — they do not
│   Company       │  move when you switch section
│   Connections   │
│   Flows         │
│                 │  space, not a rule
│ CHANNELS      + │  the active section's contents,
│  # engineering  │  and only its contents. This is
│  # general      │  the block that scrolls.
│ DIRECT MSGS   ✎ │
│  Neil · Alex    │
├─────────────────┤
│ ⚙ Settings      │  pinned, unchanged
│ ⚑ Feedback      │
│ ✦ Discord       │
└─────────────────┘
```

The accordion — each row expanding under itself — was the first shape this took
and was rejected twice over. The rows move, so "Flows is the fourth thing" only
holds while nothing above it is open. And the one section whose contents are
unbounded, Room, pushes every row after it off the bottom at an ordinary twenty
channels: the wall, recreated inside one row.

A fixed block also has no per-row open/closed state to keep. Which section is
showing is which section you are in, and the route already carries that.

**Space, not a rule, between the two blocks.** The column is quiet and the
console draws no rule above its footer, so a divider here would have been the
only seam in it. The gap is set deliberately (`pt-5`) rather than left as
whatever a removed element's margins happened to be.

### On the collapsed rail

The four icons stay. A section whose contents are a fixed list hides them —
those rows are 3rem of nothing without their labels, and the parent icon still
leads to them. **Room is the exception**: `ChannelRail` has a compact variant
built for exactly that width, and dropping it would make collapsing the sidebar
silently lose the channel list — the regression issue #1018 filed about the
approvals badge, in a new place.

## Room is the chat column, moved whole

`views/chat/ChannelRail.tsx` is not reimplemented in the sidebar. It is
**portalled** into a slot the sidebar owns (`components/room-rail.tsx`), so
every behaviour it already had comes with it: collapsible sections, per-kind row
icons, unread and mention badges, the pinned Operator feed, the compact
collapsed variant, and the "New message" door.

A portal rather than a state lift, deliberately. `ChatView` stays the one owner
of the chat model, the rail renders from that state on the same pass, and the
dialogs it opens still mount inside `ChatView`'s tree — a portal moves the DOM
node, not the component tree. Lifting the model would have meant an effect
writing it up to the shell and a re-render of the whole console every time an
unread count changed.

Two consequences worth stating:

- **The 768–1023px two-rail band is gone by construction.** The rail was a
  second column competing with the app sidebar for the viewport, which is what
  issue #1383 was. It is a section of that sidebar now, so it has no breakpoint
  of its own: the sidebar decides once whether it is a column, a 3rem rail or a
  sheet, and the list follows.
- **`lib/chat-rail.ts` is deleted.** The sidebar's collapse *is* the channel
  list's collapse — one control, one persisted preference, no way for the two to
  disagree. The chat header's density toggle went with it, for the same reason:
  it had become a second control doing the sidebar's job, forty pixels from the
  sidebar's own.

### Room at its real cap

Validated against a company with twenty channels plus direct messages, not
against a demo company with three. The contents block is its own scroll region:
the four rows and the footer never move, and the channel list scrolls within it.

**Scroll, not truncate.** A "show all" was the alternative and is worse here: a
channel list is scanned for a name you already know, so hiding its tail behind a
control makes the one thing you came for the one thing you cannot see. Scrolling
costs a gesture only at the sizes where truncating would cost a click *and* a
gesture.

## The eight Rule 6 calls

Every view that lost, or never had, a nav row, and which of Rule 6's four
treatments it takes.

| View | Treatment | Why |
| --- | --- | --- |
| `overview` | **Discoverable elsewhere** | An icon in the window's title row, left of the profile. A place you jump to from anywhere is chrome, not a destination in a list of destinations. |
| `approvals` | **Discoverable elsewhere** | The same, and the count travels with it. The title row is visible from every page in every sidebar state — the sidebar badge was not, which is the whole reason `SidebarMenuDot` had to exist (issue #1018). It is deleted with the row. |
| `observatory` | **Discoverable elsewhere** | A row on the Settings rail. `#/settings/observatory` is *rewritten* onto `#/observatory` rather than rendered under Settings — the Observatory owns four query keys of its own and reads them off the hash's head (`views/observatory/hash.ts`), so under `#/settings/…` its analytics tab and agent/turn selection stop being addressable. The rail row is the doorway; the surface keeps its own address. |
| `tasks` | **Deep-link destination** | `#/tasks/<id>` is a card on Work's board, linked from chat, approvals, workflow rows and every card. Bare `#/tasks` is rewritten onto the board (Rule 2). |
| `team` | **Deep-link destination** | `#/team/<id>` is a seat on the org chart. Bare `#/team` is rewritten onto Agents (issue #1141). |
| `pages` | **Deep-link destination** | Agent-authored dashboards are direct-URL-only on purpose (issues #1171, #1172). What keeps `#/pages` answering is its `ROUTABLE` entry, never a commented nav row — that confusion is issue #1311. |
| `inbox` | **Parked but reachable** | Unchanged by this restructure. Issue #302 parked it; issue #1337 is the open question of what a parked surface should say. |
| `feedback` | **Discoverable elsewhere** | The sidebar footer links to it, as it always has. |

`isNavigationActive` in `lib/console-routes.ts` is where two of those deep-link
views are claimed by the section they belong to. Without it the sidebar empties
the moment an operator opens a card or a teammate: the section
goes dark and its contents block disappears with it.

## Labels and view ids are allowed to differ

"Room" is the `chat` view. "Flows" is `workflows`. "Work" has been `ledgers`
since #1284, and "Agents" is `company`.

A view id is an **address** — every `#/chat/<channelId>` link ever minted, every
`#/workflows/<id>` a run row points at — and renaming a row is not a reason to
break them. The `data-tour` anchors follow the view id for the same reason: they
are how the guided tour and the e2e specs find a row, and they should not move
when a word does.

The one thing that *must* track a rename is prose. `chat-approval-line.spec.ts`
used to build its selector by lowercasing a label, which turned exactly this
rename into a silent break; it takes a view id now.

## The tour is a deliverable of an IA change, not a follow-up

Four of the tour's eight stops pointed at rows this change removes. That fails
**silently**: `waitForTarget` resolves `false` when an anchor never mounts and
the controller treats it as a stop to *skip* rather than an error — deliberately,
so a slow lazy chunk cannot wedge the tour. A stop pointed at a deleted row is
therefore indistinguishable from one that loaded slowly. The tour runs, teaches
less than the product has, and nothing says so.

So the tour was rewritten rather than patched: seven stops, the four sections in
the order an operator meets them, the composer opening and closing it, and
Connect-your-tools on Apps. The Overview stop is gone rather than re-anchored —
a tour that teaches a destination an operator cannot then find is worse than one
that leaves it to be discovered where it lives.

`test/unit/tour-anchors.test.ts` is the check the tour never had. It asserts
every stop's view against `VIEWS` and every anchor against the rendered nav
table, so the silent-skip case is a failing test rather than a quieter product.

**Standing rule: an IA change that moves or removes a nav row updates the tour in
the same change.** There is no test that will tell you afterwards, unless it is
that one.

## Where this leaves the older records

- Rule 2's reasoning about a row per declared list is the argument this file
  generalises. Nothing in it is superseded.
- Rule 6 is unchanged and is now exercised by eight views rather than five.
- Rule 7's Connections section survives; only its rail moved into the sidebar.
- `finance-console.md` still describes Finance's sub-pages correctly. Its
  content rail is the last one left and is the obvious next thing to convert.
