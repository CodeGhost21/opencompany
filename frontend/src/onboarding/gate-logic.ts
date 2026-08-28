// The pure half of the onboarding gate (issue #1844): whether to render
// `OnboardingGate` instead of the ordinary shell.
//
// Pinned here rather than left as an inline `if` in `app-shell.tsx`, for the
// same reason `shouldOfferSetup` lives in `lib/company-setup.ts` instead of
// inside `SetupController`: getting this wrong is expensive in a specific
// direction — showing the blocking gate to an operator who already cleared
// it, or over a company still being staffed — and a pure function of state is
// what makes that a unit test instead of a manual click-through.

import type { ActivationStatus } from "@/api/activation";

export interface GateDecisionInput {
  /** The funnel's last successful read, or `null` before the first one lands. */
  status: ActivationStatus | null;
  /** Whether that first read has landed (distinct from `status` — see the hook). */
  checked: boolean;
  /**
   * Whether first-run setup (staffing) is still on screen or the company is
   * unstaffed — `AppShell`'s own `setupOpen`, the same signal `TourController`
   * holds on. Setup runs first: an operator with nobody on the roster yet has
   * no workflow to run, so the activation gate must wait for it exactly the
   * way the tour already does.
   */
  setupOpen: boolean;
  /** Whether "skip for now" was clicked earlier in this tab's session. */
  skippedThisSession: boolean;
  /**
   * Whether the signed-in user is this company's admin — `null` before that
   * read has landed (PR #1878 review finding).
   *
   * None of the three steps this gate blocks on can be cleared by anyone
   * else: naming the company (`PATCH {scope}`) is `require_admin`-gated on
   * the host, and the Composio connect routes use `AdminScopedCompany`. An
   * invited member's only way past an unconditional gate was "Skip for now"
   * — session-scoped by design (see `onboarding/state.ts`), so it re-traps
   * them every new tab — which made this screen a dead end for exactly the
   * people it cannot ask anything of.
   */
  isAdmin: boolean | null;
}

/**
 * Whether the blocking first-run gate should be on screen right now.
 *
 * Order matters and is deliberate: a session-scoped skip and an in-progress
 * setup both win over an unread or incomplete funnel, because rendering the
 * gate over either would be rendering it wrong rather than merely early — see
 * each guard's own reasoning below.
 */
export function shouldShowOnboardingGate(input: GateDecisionInput): boolean {
  // "Skip for now" must always win. A hard lock behind a broken Composio
  // connect is worse than the blank app this gate replaces (the issue's own
  // words) — an operator who dismissed it must never be trapped back in it
  // until they navigate again, even if a poll landed in between.
  if (input.skippedThisSession) return false;

  // Staffing runs first. A company with nobody on the roster has no workflow
  // an operator authored to run, so asking them to clear step 3 here would be
  // asking for something `SetupController` has not offered them yet.
  if (input.setupOpen) return false;

  // Before the first read lands there is nothing to gate on — rendering the
  // gate here would flash it open for every company, activated or not, for
  // the one round trip it takes to learn which.
  if (!input.checked || !input.status) return false;

  // PR #1878 review finding: an invited member cannot act on any of the
  // three steps below (see `isAdmin`'s own doc comment), so the gate must
  // never be their dead end. `null` — the admin read has not landed yet — is
  // held here the same way `checked`/`status` are just above: an admin who
  // would otherwise see the gate immediately now waits one extra round trip
  // rather than this ever flashing open for a member who cannot clear it.
  if (input.isAdmin === null || !input.isAdmin) return false;

  return !input.status.isActivated;
}
