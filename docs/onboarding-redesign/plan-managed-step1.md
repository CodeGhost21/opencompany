# Slice 4a — Managed step 1: reuse the real fan-out

The highest-risk slice in this plan — it's two different pieces of work
wearing one name. Split them explicitly rather than estimating them together.

## Architecture Impact

Two independent changes, only one of which is "reuse":

**4a-i (reuse, mechanical):** the wizard's paste-a-key path stops calling
`company::inference::store_key` and instead calls
`setCompanyCredential`/`setCompanyCredentialModel` — the exact functions
`ApiKeyView.tsx`'s `write()`/`writeModel()` already call. This is a straight
swap of which endpoint a form submit hits; no new UI shape.

**4a-ii (new, not reuse):** a real "Login with TinyHumans" button that calls
`link/start` and catches the redirect. Nothing today calls `link/start` from
inside the console — reuse-mapping.md §1 confirmed zero call sites. This is
net-new frontend work plus the grant-landing relocation decision from
open-questions.md.

## Files to Modify

- `frontend/src/views/setup/SetupWizard.tsx` — the Managed branch's step-1
  component (new, replacing `PowerStep`'s TinyHumans-specific path on this
  branch): paste-a-key submit calls `setCompanyCredential`/
  `setCompanyCredentialModel` (`@/api/credential`) instead of whatever
  currently posts to the wizard's own inference-key endpoint.
- Backend: confirm `PUT …/credential` and `PUT …/credential/model` are
  reachable pre-company-creation. They're scoped to an existing company
  today (`ApiKeyView` always has a `company` prop from a mounted Connections
  page); the wizard runs *before* a company exists. **This is not addressed
  anywhere in the docs and needs resolving before writing code**: either
  these endpoints already support an unscoped/pending-company call shape (if
  `client.scopeFor(null)` — referenced in `credential.ts`'s
  `setCompanyCredentialModel` — resolves to something valid pre-creation,
  check what), or the wizard needs to stage the key/model choice locally and
  defer the actual `setCompanyCredential` call to just after
  `seed_generated_company` succeeds in submit. Check `client.scopeFor`'s
  behavior with `company: null` before assuming either path.
- For 4a-ii: a new button + `link/start` caller, wired into the Managed
  step-1 component. Grant-landing: pick one of open-questions.md's two
  options (mount inside `App.tsx`'s boot sequence, or relocate
  `pending-key-link.ts`'s stash) — this is a real architecture decision, not
  an implementation detail, and needs sign-off before code is written against
  it.

## New Files

- The Managed step-1 component itself (new file or new case in
  `SetupWizard.tsx`, matching whatever convention slice 3 established).
- Whatever the grant-landing decision requires — a shared context/provider
  if relocating the stash (open-questions.md option 2), or nothing new if
  mounting inside `App.tsx`'s existing sequence (option 1).

## Dependencies

- Slice 3 (the branch point needs to exist to mount this inside it).
- Slice 2a (`Slot::Search`) for the fan-out to actually be complete — but
  4a-i (the reuse swap) does not need to *wait* for 2a to land; it correctly
  calls the real fan-out either way, and search simply won't be filled until
  #2342 ships. Do not block 4a-i on 2a; do note in the PR description that
  search coverage is pending.

## Implementation Steps

**4a-i:**
1. Resolve the pre-company-existence scoping question above — this
   determines whether steps 2-6 below happen "live" during the wizard or are
   staged and flushed at submit time.
2. Swap the paste-a-key submit handler to `setCompanyCredential`.
3. Wire the `needsModel` response into a model-picker step, matching
   `ApiKeyView.tsx`'s existing two-step UI shape (reuse-mapping.md §1).
4. On model selection, call `setCompanyCredentialModel` (or
   `setCompanyCredential` with both key+model if there's no staged
   intermediate state — match whichever of `write()`/`writeModel()`'s shape
   applies).
5. Delete the now-dead `company::inference::store_key` call site in the
   wizard's submit path (`server/setup.rs:962-965`-adjacent) — confirm
   nothing else still depends on it before removing (grep for other
   callers).

**4a-ii:**
6. Make the architecture call on grant-landing (open-questions.md) — get
   this reviewed before step 7.
7. Add the "Login with TinyHumans" button, calling `link/start`.
8. Wire the redirect-landing to read the stash per the decision in step 6.
9. On redemption, follow the same `needsModel` → model-picker path as 4a-i.

## Testing Strategy

- An e2e spec parallel to `tinyhumans-account-key.spec.ts` but starting from
  the wizard instead of Connections → Account, asserting the same end state
  (fan-out filled, `cognition: harness`, no restart banner).
- A spec for the grant path, parallel to `tinyhumans-key-link.spec.ts`,
  confirming the redirect lands correctly inside the wizard specifically
  (not just that the mechanism works in isolation — the wizard's mount
  timing relative to `App.tsx`'s boot is exactly what's unverified today).
- A unit test on whichever scoping resolution was chosen in step 1 — pending-
  company vs. staged-then-flushed each need their own failure-mode coverage
  (e.g.: operator closes the wizard mid-step-1 after a key was already sent
  live — is the key now stranded attached to nothing? Does staged-then-flush
  risk losing the tested key if submit fails for an unrelated reason later?).

## Risks and Edge Cases

- **The pre-company scoping question is the real risk in this whole slice.**
  Everything else is mechanical once it's answered. Do not start 4a-ii until
  4a-i's answer is settled, since the button's landing behavior depends on
  it too.
- **4a-ii is easy to underscope as "just add a button."** It's the grant
  machinery's first real caller in the entire app — treat it with the same
  care the original grant implementation (#2338) got, not as wizard glue
  code.
- **`ReuseAccountKeyBanner`** (`frontend/src/inference/ReuseAccountKeyBanner.tsx`)
  is an existing, shipped pattern for "you have a TinyHumans key, use it
  here too?" on the Connections pages. It is not reused directly by this
  slice — the wizard's cascade fills slots outright rather than asking — but
  its existence means there's already a team-established tone/UX pattern for
  this kind of prompt. Worth a look before designing the wizard's own copy
  from scratch, purely for consistency.

## Developer Handoff

Do not start writing UI for this slice until the pre-company scoping
question has a written answer from whoever owns the credential endpoints —
that answer changes the shape of every subsequent step. Split 4a-i and
4a-ii into separate PRs; 4a-i alone is already a complete, shippable
improvement (a company set up in onboarding today doesn't even get a
Provider row — 4a-i alone fixes that regardless of whether the login button
ever ships).
