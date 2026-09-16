# Target flow

The redesign, step by step. Every "reuses" claim here is expanded with exact
file:line evidence in [reuse-mapping.md](reuse-mapping.md) — read that before
implementing any step below.

```
                     ┌───────────────────────────────────┐
                     │  STEP 0 — Choose your setup way    │
                     │  ○ Managed with TinyHumans         │
                     │  ○ Set it up yourself              │
                     └──────────────────┬──────────────────┘
                                         │
                 ┌───────────────────────┴───────────────────────┐
                 ▼ MANAGED                             SELF-MANAGED ▼
   ┌───────────────────────────────┐         ┌───────────────────────────────┐
   │ 1. Login to TinyHumans         │         │ 1. Provider  (skip for later)  │
   │    "Login with TinyHumans"     │         │    Composio  (skip for later)  │
   │    button — or paste a key     │         │    simplified views, reused    │
   │                                 │         │    from Connections > LLM /    │
   │    sets tinyhumans/key, then    │         │    Connections > Composio      │
   │    cascades:                   │         │                                 │
   │      provider/tinyhumans/key   │         │    each independently          │
   │      composio/managed/key      │         │    skippable                   │
   │      search/managed/key (#2342)│         │                                 │
   └────────────────┬────────────────┘         └────────────────┬────────────────┘
                     └─────────────────────┬──────────────────────┘
                                            ▼
                       STEP 2 — Name the company + pick a template
                                            ▼
                       STEP 3 — How should people sign in?
                                            ▼
                       STEP 4 — Review the roster → "Build my company"
                                            ▼
                              COMPANY OPENS (straight into the console)
```

## Step 0 — Choose your setup way (new)

New screen, no equivalent today. Two options:

- **Managed with TinyHumans** — sign in (or paste a key), TinyHumans handles
  login/credentials/model.
- **Set it up yourself** — configure provider and Composio manually.

No network call. Purely picks which step-1 component renders next. New
component `SetupWayStep`, added to `STEPS` ahead of `power`
(`SetupWizard.tsx:105-111`).

## Managed · step 1 — Login to TinyHumans

Reuses the real Connections → Account mechanism (`ApiKeyView.tsx`), not the
wizard's own `inference::store_key`. See reuse-mapping.md §1 for the two
gaps this exposes (no working "Login with TinyHumans" button anywhere yet;
the grant-landing stash is scoped to wherever `useRedeemKeyGrant` mounts).

Two entry points, matching what `ApiKeyView.tsx`/`AccountKeyDialog` already
offer:

- Paste a key → `setCompanyCredential` (`PUT …/credential`) → on
  `needsModel`, `setCompanyCredentialModel` (`PUT …/credential/model`).
- "Login with TinyHumans" (grant) → **new work**, not reuse: needs a real
  `link/start` caller wired into this step, and the redirect landing needs to
  reach the same in-memory stash `ApiKeyView` reads from.

Either path fires the same fan-out: `provider/tinyhumans/key`,
`composio/managed/key`, and (once #2342 lands) `search/managed/key` — never
overwriting a slot that already holds its own key.

## Self-managed · step 1 — Provider + Composio

Simplified views of the real Connections → LLM and Connections → Composio
pages, each independently skippable via "set this up later." Not a cascade —
two separate credentials, set (or deferred) independently. See
reuse-mapping.md §2.

## Step 2 — Name the company + pick a template

Same template/industry choice `BusinessStep` already has, plus a company-name
field moved here from Review. See reuse-mapping.md §3 for exactly what moves.

## Step 3 — How should people sign in?

Unchanged: today's `SignInStep`, verbatim.

## Step 4 — Review the roster → Build my company

Unchanged mechanics: `design()` on entry, `POST /api/v1/setup/roster`, the
roster shown, "Build my company" → `POST /api/v1/setup`. The only difference
from today: no editable name field here anymore (moved to step 2), and the
key-save from step 1 (Managed branch) should already have rebuilt the runtime
in place by the time this step's submit runs — see reuse-mapping.md §4.

## Company opens

No post-build checklist. `submit()`'s success path goes straight to the
handoff (mailed / unmailable / link / arranging / default → "Open the
console") exactly as today, then straight into `AppShell` — the checklist
screen between them is deleted, not skipped. See [removed.md](removed.md).
