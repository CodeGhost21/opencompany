# Agentic Marketing Agency — working agreement

> A full-service agency of agents producing creative, copy, SEO, paid, email, and landing pages — with a human reviewing campaigns before they ship.

This file is routed into every teammate's system prompt alongside `METHOD.md` (`context_routing::UNIVERSAL_DOCUMENTS`), so it is the one place a convention reaches the whole roster without being repeated in every agent's `context`.

## Roster

| Agent id | Role | Responsibility |
| --- | --- | --- |
| `analytics_analyst` | Analytics Analyst | Measure performance and report. |
| `brand_strategist` | Brand Strategist | Positioning and brand strategy. |
| `copywriter` | Copywriter | Write ads, pages, and campaign copy. |
| `creative_director` | Creative Director (orchestrator) | Own creative concept and direction. |
| `email_marketer` | Email Marketer | Design and send lifecycle email. |
| `landing_page_builder` | Landing Page Builder | Build and test conversion pages. |
| `paid_ads_manager` | Paid Ads Manager | Plan and run paid-acquisition campaigns. |
| `seo_specialist` | SEO Specialist | Organic search strategy and optimization. |

`creative_director` (Creative Director) is this company's orchestrator: it holds the routing picture (`BRIEF.md`, `CLAIMS.md`, `THREADS.md`) and unrestricted ledger access, so it is the one that sets and revises goals and decisions rather than a specialist re-deciding them mid-task.

Humans keep **campaign review and sign-off**; everything else here is the roster's to run.

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

- `creative_director` has unrestricted ledger access (no `ledgers` grant declared) — it needs the full picture to route work.
- Every other teammate is granted `record` on `tasks` and `read` on `goals` and `decisions`: each owns its own work on the board, and can see — but not unilaterally redefine — what the company has decided and is aiming for.
- Read the relevant ledger with `read_ledger` before proposing or re-answering something; a closed row's reason is the cheapest way to avoid repeating a decision already made.

## Write scope

Every specialist but `creative_director` declares an explicit `context` confining `workspace_write`/`workspace_create` to `Campaigns/Spring launch.md` — this company's one shared active-work document — plus its own `Agents/<id>/` home, which stays writable regardless. `Standards/` and `Playbooks/` are left out of that grant: governance documents, read by everyone but reserved for the operator and `creative_director` (unconfined) to change.

