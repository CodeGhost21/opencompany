# Open questions and risks

Not decided. Each needs a real answer before or during its slice, not an
assumption baked into the implementation.

## Deferred: the grant-landing relocation problem

A one-click "Login with TinyHumans" OAuth grant button was considered for
Managed step 1 and explicitly descoped from this implementation — the
existing "Connect to TinyHumans" paste-a-key dialog is Managed step 1
instead, verbatim. The relocation problem this would have raised (the
redeemed grant's one-time code lives in a module-level variable,
`pending-key-link.ts:20`, read today by exactly one caller,
`useRedeemKeyGrant` inside `ApiKeyView.tsx` — a new caller in the wizard
would need to either share `App.tsx`'s boot sequence or a relocated stash)
is not this implementation's concern. Recorded here only so it isn't
re-discovered from scratch if a login button is built later.

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

## Answered: Composio's skip-for-later state, self-managed branch

**Skipping records nothing, on either half of the step.** There is no
"explicitly deferred" state to store and none is added.

Composio's card already has the resting state, and it is the honest one:
`composioRows(null)` returns both rows for a company with no status at all —
`modeOf` reads a missing mode as `managed`, which is where a company with
nothing configured genuinely is — with a token to add on the managed route and
the own-account route offering to be chosen. That is the same card Connections
draws, so the wizard mounts it rather than inventing an empty state for it.

What is *not* stored is the distinction between "deferred" and "never tried".
`null` is the truth about a company minutes old, and nothing downstream could
act on the difference: no surface reads it, no later prompt is gated on it, and
a company that skipped and a company that has not got there yet want exactly the
same thing offered next. So the reassurance is shown while the operator is
standing on the step — "you can add this later under Connections" — and
forgotten when they press Next, which is what the step component unmounting does
for free.

One shape was deliberately not reused: the old model step's `tested =
{kind:"skipped"}`. That is a single verdict slot read by the step gate *and* by
the design pass's `modelless`, so recording a Composio skip in it would have
suppressed the roster design brief — a real bug, from an operator saying "later"
to their integrations.

## Whether `visibleSteps`' hide-conditions still make sense with a branch point

Today's `visibleSteps` filtering (`SetupWizard.tsx:665-674`) hides `power`
when the host already supplies inference. With `power` replaced by the
step-0 branch + two step-1s, the equivalent condition ("this host already has
inference — skip both step-1 branches entirely and go straight to step 2") is
new logic, not a direct port of the old one. Needs its own explicit handling
in slice 3, not an assumption that hiding a step generalizes cleanly to
hiding a branch.
