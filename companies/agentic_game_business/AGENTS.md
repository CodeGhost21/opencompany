# Agentic Game Business — working agreement

> The business layer around a live game: user acquisition, monetization design, LiveOps events, community, store optimization, and player support.

This file is routed into every teammate's system prompt alongside `METHOD.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `analytics_analyst` | Analytics Analyst | Track KPIs, LTV, retention, and cohorts. |
| `community_manager` | Community Manager | Grow and moderate the player community. |
| `liveops_manager` | LiveOps Manager | Plan and run events and content updates. |
| `monetization_designer` | Monetization Designer | Design offers, pricing, and the in-game economy. |
| `player_support` | Player Support | Resolve player issues and refunds. |
| `store_optimizer` | Store Optimizer | App-store optimization and conversion. |
| `user_acquisition` | User Acquisition (orchestrator) | Run paid and organic UA campaigns. |

`user_acquisition` (User Acquisition) is this company's orchestrator: it holds the routing picture (`BRIEF.md`, `CLAIMS.md`, `THREADS.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **monetization and growth strategy**; everything else here is the roster's to run.

## Workspace layout

- `Standards/`, `Product/`, `Playbooks/` — shared, operator-seeded notes. Read them before proposing work that touches an area they cover; edit them on purpose, not as a side effect of an unrelated task.
- `Agents/<your agent id>/` — your own folder, the default home for anything you produce. Always writable, whatever your `context` write scope says.
- `derived/` — rendered ledger views (see below). Never hand-write anything here; it is regenerated on every ledger write.

## Ledgers

This company keeps the three built-in ledgers — `tasks` (the task board), `goals`, and `decisions` — and any teammate may declare another with `define_ledger` when a recurring axis (a pipeline, a promise, an experiment) does not fit one of these.

- `user_acquisition` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

