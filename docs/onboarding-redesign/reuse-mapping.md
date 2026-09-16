# Reuse mapping

The point of this redesign is that most of it is not new code. This file
names, for every piece of the target flow, the exact existing
function/component/endpoint it must call — and separately, honestly, the
pieces that are not reuse no matter how they're described, because the thing
being "reused" doesn't actually exist yet. Conflating the two is the failure
mode this file exists to prevent.

## §1 Managed step 1 — the login/key mechanism

**Reuse, real and available today:**

- `setCompanyCredential(client, company, key, model?)` — `PUT …/credential`
  (`frontend/src/api/credential.ts`). This is what `ApiKeyView.tsx`'s
  paste-a-key dialog (`AccountKeyDialog`) calls today (`write()`,
  `ApiKeyView.tsx:309-402`).
- `setCompanyCredentialModel(client, company, model)` — `PUT
  …/credential/model` (`credential.ts`). Finishes the row off a *stored* key
  when there's no `pendingKey` — this is what a redeemed grant uses
  (`writeModel()`, `ApiKeyView.tsx:411-458`).
- The fan-out these trigger server-side, `company_key/fan_out.rs`'s `Slot`
  enum (`Composio, Inference, Provider, Default, Health`, plus the new
  `Search` from #2342). Confirmed real: `ApiKeyView.tsx:564-567`'s own copy —
  "Saving copies it to the LLM and Composio pages wherever they hold no key
  of their own" — and it genuinely does not overwrite a slot that already has
  its own key (`:556-563`).
- The rebuild-in-place mechanism: `set_key`, `set_model`, `finish_link` in
  `company_key.rs` all call `rebuild_if_pending` (`company_key.rs:1851`) when
  the fan-out filled/rotated the provider or default slot and
  `restart_required` holds. This is issue #290's mechanism, extended to the
  key-save path by #2338.

**Required change, not an assumption:** the wizard's `PowerStep` today does
**not** call any of the above. It calls `company::inference::store_key()`
(`inference.rs:952-967`), a completely separate mechanism that writes exactly
one secret (`inference/key`) plus the manifest's `inference` block
(`server/setup.rs:962-965`) — no Composio, no Search, no fan-out at all. Slice
4a (README.md's order-of-work) is this exact swap: Managed step 1 must call
`setCompanyCredential`/`setCompanyCredentialModel`, not
`inference::store_key`. Until this swap happens, "connecting TinyHumans in
onboarding" and "connecting TinyHumans on the Account page" are two different
features that happen to look similar.

**Not reuse — does not exist yet:** a real one-click "Login with TinyHumans"
button. Traced directly: `ApiKeyView.tsx` has exactly one entry point today,
"Connect to TinyHumans," which opens the paste-a-key dialog. The actual
"Sign in with TinyHumans" button was removed from that page on 2026-09-14
(`account.ts:119-120`). The **only** place that phrase exists anywhere in the
app is a plain external link in the *current* wizard's `PowerStep`
(`SetupWizard.tsx:1886-1894`, `<a href={keySource.url}>`) — and its own
comment says what it is: "the operator signs in as themselves, creates the
key on their own dashboard, and brings it back to the field above." That's a
link-out to copy-paste a key manually, not an OAuth grant.

The grant machinery is real and working — `POST /api/v1/company/credential
/link/start` returns a real `authorizeUrl`, the hub's consent page is real,
`finishCredentialLink`/`useRedeemKeyGrant` genuinely redeem it (proven by
`frontend/test/e2e/tinyhumans-key-link.spec.ts`, added by #2338). But
**nothing in the console calls `link/start`.** Grepped the whole frontend:
zero call sites. So "reuse the login button" is not possible — there is no
button to reuse, only a redemption path waiting for one. Building the button
is slice 4a's real scope, not a subtraction from it.

**The landing-page constraint, if the button is built:** the redeemed grant's
code is stashed in a **module-level JS variable**
(`frontend/src/lib/pending-key-link.ts:20`, `let pending: PendingKeyLink |
null = null`), deliberately not `sessionStorage` — a redeemable credential
sitting in browser storage is exactly what the URL-stripping dance exists to
avoid. `App.tsx:109-119` parses the redirect params, `:148-159`
(`clearKeyLinkFromUrl`) strips them via `history.replaceState` (not a reload,
which would reset the module and lose the code), and `:513` writes the stash
once at boot. **Only one reader exists today**: `useRedeemKeyGrant`, called
from `ApiKeyView.tsx` alone. If the wizard needs to catch this same
redirect, it has two options, both real work: mount inside the same
`App.tsx` boot sequence so it shares the stash, or relocate the stash
somewhere both the wizard and `ApiKeyView` can reach. Pick one before writing
slice 4a's grant-landing code — see [open-questions.md](open-questions.md).

## §2 Self-managed step 1 — Provider + Composio

**Reuse:** the real Connections → LLM page's provider list and the real
Connections → Composio page, both simplified/condensed for the wizard
context, each with its own "set this up later" skip. Not a cascade — no
fan-out involved, since neither credential comes from a TinyHumans key here.

**Scope note:** "simplified view" is a UI-layer decision (fewer fields shown,
fewer affordances), not a new backend surface — the same `PUT
/inference/providers`-style endpoints and Composio's own connect flow back
both, unchanged. If the simplified view turns out to need a capability the
real pages don't expose (e.g. a combined "connect and skip" single action),
that is a stop point, not something to build ad hoc in the wizard.

## §3 Step 2 — Name the company + pick a template

**Reuse:** `BusinessStep`'s existing template/industry logic, verbatim.

**New:** a company-name field, moved here from `ReviewStep`. Today
`ReviewStep` shows an **editable company name** alongside the designed roster
(`SetupWizard.tsx:1989` area, traced in current-flow.md). D-name-once
(README.md) requires this field to be deleted from Review, not duplicated —
naming happens exactly once. `submit()`'s `SetupInput.name` field is
unchanged; only which step populates it moves.

## §4 Rebuild-in-place for the wizard's key-save

**Reuse:** `rebuild_if_pending` (`company_key.rs:1851`), same as §1.

Once Managed step 1 calls the real fan-out (§1's required change), it gets
this for free — `set_key`/`set_model`/`finish_link` already call
`rebuild_if_pending` themselves. The only thing to verify in slice 7 is that
the wizard doesn't *also* need its own rebuild trigger for whatever happens
at final submit (`apply_inner` → `seed_generated_company` → `register()`),
since that path boots the runtime fresh rather than rebuilding an existing
one — confirm at implementation time whether a freshly-registered company
ever needs `rebuild_if_pending` at all, or whether `register()`'s own boot
already resolves inference correctly the first time. This is a real open
question, not assumed either way — see [open-questions.md](open-questions.md).

## §5 The search tier (#2342)

**Not reuse — a dependency.** Issue #2342 is what makes `search/managed/key`
exist as a fan-out slot at all. Today there is no `Search` variant in
`company_key/fan_out.rs`'s `Slot` enum, and Search's "Managed" row is not a
real provider (`catalogue.rs:23-25`: "`managed` is not in this table"). Slice
4a (Managed step 1 calling the real fan-out) will silently *not* set up
search until #2342 ships — the wizard doesn't need to do anything extra for
search once #2342 lands, but it cannot claim to set up search before then.
Do not implement a wizard-side search step to compensate; the fan-out is the
right layer for this, per #2342's own "no new fallback complexity" framing.
