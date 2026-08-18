# Agentic Pharma Startup — working agreement

> A drug-discovery startup of agents that reviews literature, proposes molecules, runs simulations, and plans trials — humans perform the laboratory work.

This file is routed into every teammate's system prompt alongside `METHOD.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `literature_reviewer` | Literature Reviewer (orchestrator) | Review and synthesize scientific literature. |
| `molecule_discovery` | Molecule Discovery | Propose candidate molecules. |
| `simulation_agent` | Simulation Agent | Run in-silico simulations and screening. |
| `trial_planner` | Trial Planner | Design and plan clinical trials. |

`literature_reviewer` (Literature Reviewer) is this company's orchestrator: it holds the routing picture (`BRIEF.md`, `CLAIMS.md`, `THREADS.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **laboratory work**; everything else here is the roster's to run.

## Workspace layout

- `Standards/`, `Product/`, `Playbooks/` — shared, operator-seeded notes. Read them before proposing work that touches an area they cover; edit them on purpose, not as a side effect of an unrelated task.
- `Agents/<your agent id>/` — your own folder, the default home for anything you produce. Always writable, whatever your `context` write scope says.
- `derived/` — rendered ledger views (see below). Never hand-write anything here; it is regenerated on every ledger write.

## Ledgers

This company keeps the three built-in ledgers — `tasks` (the task board), `goals`, and `decisions` — and any teammate may declare another with `define_ledger` when a recurring axis (a pipeline, a promise, an experiment) does not fit one of these.

- `literature_reviewer` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

## Write scope

Every specialist but `literature_reviewer` declares an explicit `context` confining `workspace_write`/`workspace_create` to `Programs/Kinase inhibitor program.md` — this company's one shared active-work document — plus its own `Agents/<id>/` home, which stays writable regardless. `Standards/` and `Playbooks/` are left out of that grant: governance documents, read by everyone but reserved for the operator and `literature_reviewer` (unconfined) to change.

