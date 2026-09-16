# Open questions and risks

Not decided. Each needs a real answer before or during its slice, not an
assumption baked into the implementation.

## The grant-landing relocation problem

[reuse-mapping.md](reuse-mapping.md) §1 traces this in full: the redeemed
grant's one-time code lives in a module-level variable
(`pending-key-link.ts:20`) written once at `App.tsx` boot and read by exactly
one caller, `useRedeemKeyGrant` inside `ApiKeyView.tsx`. If Managed step 1
needs to catch a grant redirect (for the real "Login with TinyHumans" button
this redesign wants), it needs one of:

1. Mount the wizard's login step inside the same boot sequence `App.tsx`
   already runs, so it can call `takeKeyLink()`/`takeKeyLinkRefusal()`
   directly.
2. Relocate the stash to somewhere both `ApiKeyView` and the wizard can
   reach — a shared context/provider above both mount points.

Option 1 is smaller but couples the wizard's mount timing to `App.tsx`'s own
boot order in a way that isn't true today. Option 2 touches a mechanism
that's deliberately minimal (a plain module variable, chosen specifically to
avoid persistence) and any change to it needs the same care the original
design put into "why not `sessionStorage`." Neither is free. Pick one before
slice 4a's grant-landing code is written, not during.

## Does a freshly-registered company ever need `rebuild_if_pending`?

[reuse-mapping.md](reuse-mapping.md) §4: once Managed step 1 calls the real
fan-out, `set_key`/`set_model`/`finish_link` already trigger
`rebuild_if_pending` on their own. But the wizard's final submit
(`apply_inner` → `seed_generated_company` → `register()`) boots the company's
runtime **fresh**, not in-place — it's not clear whether a fresh boot can
ever be in the "resolved but stale" state `rebuild_if_pending` exists to fix,
or whether `register()` always resolves inference correctly on a first boot
by construction. If a fresh boot can race the key-save (key saved in step 1,
runtime registered later in submit, without re-reading the just-saved
key), that's a real bug this redesign could introduce. Verify the ordering
at implementation time; do not assume `register()` is unaffected just because
it's a different code path.

## The security-boundary shift for search's managed credential

Already called out explicitly in issue #2342 and in this folder's
D-boundary-change: today, `search_backend_from_env`'s doc comment
(`provider.rs:333-334`) states the managed-search credential resolver
consults only the environment, "so a company can never point search at a key
it controls." #2342 deliberately crosses this line for the company tier. This
folder inherits that decision rather than re-deciding it, but flags it again
here because onboarding is the surface that will make this common — most
companies that connect TinyHumans during setup will now be exercising the
crossed boundary from their very first session, not as an edge case reached
later. Worth a second look from whoever reviews #2342's implementation,
specifically asking: does making this the *default* path (via onboarding)
change the risk calculus versus it being an opt-in a company reaches later
via Connections → Account?

## Composio's skip-for-later state, self-managed branch

Self-managed step 1 offers "set this up later" independently for Provider
and Composio. Composio's own connect flow today is effectively all-or-nothing
per company (one key, one connection) — it's not confirmed whether Composio's
existing UI already has a clean "not connected yet, connect later" resting
state that the wizard's simplified view can just reuse, or whether it needs
new UI to represent "explicitly deferred" as distinct from "never tried."
Check against the real Connections → Composio page's current empty/disconnected
state before assuming it maps cleanly.

## Whether `visibleSteps`' hide-conditions still make sense with a branch point

Today's `visibleSteps` filtering (`SetupWizard.tsx:665-674`) hides `power`
when the host already supplies inference. With `power` replaced by the
step-0 branch + two step-1s, the equivalent condition ("this host already has
inference — skip both step-1 branches entirely and go straight to step 2") is
new logic, not a direct port of the old one. Needs its own explicit handling
in slice 3, not an assumption that hiding a step generalizes cleanly to
hiding a branch.
