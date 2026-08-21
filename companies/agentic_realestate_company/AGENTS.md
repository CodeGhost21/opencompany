# Agentic Real Estate Company — working agreement

> A real-estate operator of agents that finds properties, analyzes neighborhoods, underwrites deals, coordinates contractors, and manages tenants — humans approve purchases.

This file is routed into every teammate's system prompt alongside `METHOD.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `contractor_coordinator` | Contractor Coordinator | Coordinate contractors and renovations. |
| `deal_underwriter` | Deal Underwriter | Underwrite deals and model returns. |
| `neighborhood_analyst` | Neighborhood Analyst | Analyze neighborhoods, comps, and trends. |
| `property_scout` | Property Scout (orchestrator) | Find and screen candidate properties. |
| `tenant_manager` | Tenant Manager | Manage tenants and property operations. |

`property_scout` (Property Scout) is this company's orchestrator: it holds the routing picture (`BRIEF.md`, `CLAIMS.md`, `THREADS.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **purchase approvals**; everything else here is the roster's to run.

## Where the role rules live

Each teammate's `.toml` carries wiring only — tier, ledger grants, routed
context, delegation. The working rules live in `agents/prompts/<id>.md`, named by
that file's `prompt_files` entry and loaded into the prompt as **Your brief**
(see `docs/spec/runtime/agents.md`). Edit the brief to change how a role works;
edit the `.toml` to change what it may touch.

Print what any teammate's prompt assembles into with
`./scripts/dump-prompt.sh --company companies/<name> --agent <id>`.

## Workspace layout

- `Standards/`, `Product/`, `Playbooks/` — shared, operator-seeded notes. Read them before proposing work that touches an area they cover; edit them on purpose, not as a side effect of an unrelated task.
- `Agents/<your agent id>/` — your own folder, the default home for anything you produce. Always writable, whatever your `context` write scope says.
- `derived/` — rendered ledger views (see below). Never hand-write anything here; it is regenerated on every ledger write.

## Ledgers

This company keeps the three built-in ledgers — `tasks` (the task board), `goals`, and `decisions` — and any teammate may declare another with `define_ledger` when a recurring axis (a pipeline, a promise, an experiment) does not fit one of these.

- `property_scout` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

## Write scope

Every specialist but `property_scout` declares an explicit `context` confining `workspace_write`/`workspace_create` to `Deals/Maple St duplex.md` — this company's one shared active-work document — plus its own `Agents/<id>/` home, which stays writable regardless. `Standards/` and `Playbooks/` are left out of that grant: governance documents, read by everyone but reserved for the operator and `property_scout` (unconfined) to change.

