# Overview — the knowledge graph

`#/overview` is the graph and nothing else: no page header, no strip, and the
console's own top bar is hidden for this view (see `app-shell.tsx`). It fills
the viewport beside the sidebar.

## What it draws

Five concentric rings, read outward from the centre:

| Ring | What | Where it comes from |
|---|---|---|
| 0 | the company, drawn as its memory constellation | `lib/memory.ts` (local store) |
| 1 | departments (pillars) | **derived** — see below |
| 2 | the jobs on each pillar, and the workflows it runs | `…/tasks` grouped through their assignee; workflows **derived** |
| 3 | the teammate who does each job, each workflow stage, and the humans | `…/team` matched by `task.assignee`; humans from `…/users`; stages **derived** |
| 4 | that teammate's tools | **derived** — see below |

Rings 2 and 3 each carry two kinds. A workflow sits beside the SOP tasks
because both are work a department runs; a workflow's stages sit beside the
workers because a stage is where the flow meets the person who performs it.

Hover any node to trace its whole pillar chain. Click a pillar to grow it into
a bottom-up tree; click a job for its steps, a teammate or tool for its card.
Click the core to bloom the memory constellation, with type-to-find over it.
Drag the background to pan. `←` / `→` turn the pillar wheel; `Escape` steps
back out.

Panning is an offset on top of whatever the camera is framing, not a separate
mode: the shot still tracks its subject, just off-centre by the amount you
dragged, and re-framing (selecting a node, opening the core) resets it.

## The derived rings — read this before trusting the org chart

**Ring 1 is no longer one of them.** Departments are the company's **desks**
(issue #486) — a `[[group_chat]]` in the manifest, or an operator-created
overlay desk. That is the one place the company declares how it is organised,
and it is the same source the Company org chart reads. `assignDepartment` and
its keyword table are gone.

What is still invented by `kg/adapter.ts`:

- **Tool assignments** are a deterministic deal from the company-wide tool list,
  because `[tools] allow` is company-wide and `[[agent]]` has no `tools` field.
- **Workflows and their stages** are templates, because the console has no flow
  API; the Workflows canvas draws a single hard-coded sample. One routine is
  dealt to each desk **by position**, wrapping, and its stages are dealt
  round-robin across that desk's agents. Nothing ties a routine to what the desk
  actually does — the console does not know.

Both are deterministic, so nothing jumps between renders — but neither is
something the company declared. `DERIVED_NOTICE` in `kg/adapter.ts` is the
standing caveat. When `[[agent]]` grows `tools` (issue #363), delete
`assignTools` and read it straight through.

### Who the graph does not place

Ring 1 can only place somebody the company seats. Two kinds of teammate it
cannot, and the graph says so rather than guessing:

- **A teammate on no desk.** They are on the roster and nowhere in the
  structure, so they hang off the company core in a sector of their own, with no
  pillar above them. Their open board cards are dropped, because ring 2 hangs
  off ring 1 and there is no honest desk to hang them from.
- **A human.** Desks staff agents, so the company declares no desk for a person
  and this graph does not guess one — the same answer the org chart gives, for
  the same reason. `assignHumanDepartment` is gone with `assignDepartment`:
  spreading humans across *invented* buckets was self-consistent fiction, but
  spreading them across *real desks* would assert a membership the desk's own
  member list contradicts.

An empty declared desk draws no pillar: `buildKnowledgeGraph` only draws a
department somebody claims. That is pre-existing behaviour, not a decision this
made.

Everything else is real: a card's assignee, a skill's category, the tools a
connected MCP server advertises, and who can sign in.

## Files

`kg/` holds the graph itself — `model.ts` (the five-ring node/edge model),
`adapter.ts` (our host's data, shaped into it), `tree-layout.ts` and
`memory-core.ts` (pure layout and camera maths), and the `KnowledgeGraph` /
`KnowledgeGraphFullscreen` / `KnowledgeDetail` components. `pulse.ts` holds
the two board predicates the adapter needs. Theme tokens live under `.oc-kg`
in `src/index.css`.

The graph is the whole page, so its chrome stays minimal: a pillar selector, a
kind legend, the side paddles, and the detail card. The docked directory index
and the entity/function/action lenses were removed — with nothing else on the
page competing for attention, they covered more of the graph than they earned.
