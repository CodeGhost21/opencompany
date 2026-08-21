# Agentic Media Company — working agreement

> A BuzzFeed-shaped newsroom of agents that finds stories, verifies sources, writes, illustrates, translates, and distributes — under human editorial standards.

This file is routed into every teammate's system prompt alongside `method.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `illustrator` | Illustrator | Create article illustrations. |
| `publisher` | Publisher | Publish to the CMS. |
| `seo_optimizer` | SEO Optimizer | Optimize articles for search. |
| `social_distributor` | Social Distributor | Distribute across social channels. |
| `source_verifier` | Source Verifier | Verify sources and fact-check. |
| `story_scout` | Story Scout (orchestrator) | Find and pitch story ideas. |
| `translator` | Translator | Localize stories across languages. |
| `writer` | Writer | Write and edit articles. |

`story_scout` (Story Scout) is this company's orchestrator: it holds the routing picture (`brief.md`, `claims.md`, `threads.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **editorial standards**; everything else here is the roster's to run.

## Workspace layout

- `standards/`, `product/`, `playbooks/` — shared, operator-seeded notes. Read them before proposing work that touches an area they cover; edit them on purpose, not as a side effect of an unrelated task.
- `agents/<your agent id>/` — your own folder, the default home for anything you produce. Always writable, whatever your `context` write scope says.
- `derived/` — rendered ledger views (see below). Never hand-write anything here; it is regenerated on every ledger write.

## Ledgers

This company keeps the three built-in ledgers — `tasks` (the task board), `goals`, and `decisions` — and any teammate may declare another with `define_ledger` when a recurring axis (a pipeline, a promise, an experiment) does not fit one of these.

- `story_scout` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

## Write scope

Every specialist but `story_scout` declares an explicit `context` confining `workspace_write`/`workspace_create` to `stories/water-rights-investigation.md` — this company's one shared active-work document — plus its own `agents/<id>/` home, which stays writable regardless. `standards/` and `playbooks/` are left out of that grant: governance documents, read by everyone but reserved for the operator and `story_scout` (unconfined) to change.

