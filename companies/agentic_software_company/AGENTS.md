# Agentic Software Company — working agreement

> A software company of agents that designs, builds, ships, and supports an entire SaaS product — with a human owning product direction.

This file is routed into every teammate's system prompt alongside `METHOD.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `backend_engineer` | Backend Engineer | Build and operate the backend and services. |
| `customer_support` | Customer Support | Resolve customer issues and feed insight back. |
| `designer` | Designer | Product and UX design. |
| `devrel` | Developer Relations | Engage developers with demos, content, and community. |
| `docs_writer` | Documentation Writer | Write and maintain product documentation. |
| `frontend_engineer` | Frontend Engineer | Build the user-facing frontend. |
| `product_manager` | Product Manager (orchestrator) | Own the roadmap, specs, and prioritization. |
| `qa_engineer` | QA Engineer | Test features and catch regressions. |
| `security_engineer` | Security Engineer | Security review, hardening, and response. |

`product_manager` (Product Manager) is this company's orchestrator: it holds the routing picture (`BRIEF.md`, `CLAIMS.md`, `THREADS.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **product direction**; everything else here is the roster's to run.

## Workspace layout

- `Standards/`, `Product/`, `Playbooks/` — shared, operator-seeded notes. Read them before proposing work that touches an area they cover; edit them on purpose, not as a side effect of an unrelated task.
- `Agents/<your agent id>/` — your own folder, the default home for anything you produce. Always writable, whatever your `context` write scope says.
- `derived/` — rendered ledger views (see below). Never hand-write anything here; it is regenerated on every ledger write.

## Ledgers

This company keeps the three built-in ledgers — `tasks` (the task board), `goals`, and `decisions` — and any teammate may declare another with `define_ledger` when a recurring axis (a pipeline, a promise, an experiment) does not fit one of these.

- `product_manager` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

## Write scope

Every specialist but `product_manager` declares an explicit `context` confining `workspace_write`/`workspace_create` to `Product/Billing v2.md` — this company's one shared active-work document — plus its own `Agents/<id>/` home, which stays writable regardless. `Standards/` and `Playbooks/` are left out of that grant: governance documents, read by everyone but reserved for the operator and `product_manager` (unconfined) to change.

