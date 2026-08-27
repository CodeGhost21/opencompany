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
import { ApiError } from "@/api/types";

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
   * read has landed (PR #1875 review finding).
   *
   * None of the three steps this gate blocks on can be cleared by anyone
   * else: naming the company (`PATCH {scope}`) is `require_admin`-gated on
   * the host, and `OAuthView` disables every connect control unless
   * `/auth/me` reports `role === "admin"`. An invited member's only way past
   * an unconditional gate was "Skip for now" — session-scoped by design (see
   * `onboarding/state.ts`), so it re-traps them every new tab — which made
   * this screen a dead end for exactly the people it cannot ask anything of.
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

  // PR #1875 review finding: an invited member cannot act on any of the
  // three steps below (see `isAdmin`'s own doc comment), so the gate must
  // never be their dead end. `null` — the admin read has not landed yet — is
  // held here the same way `checked`/`status` are just above: an admin who
  // would otherwise see the gate immediately now waits one extra round trip
  // rather than this ever flashing open for a member who cannot clear it.
  if (input.isAdmin === null || !input.isAdmin) return false;

  return !input.status.isActivated;
}

/** What a failed `/auth/me` read (behind `isGateAdmin` in `AppShell`) resolves to. */
export type GateAdminCheckOutcome =
  | { settled: true; isAdmin: boolean }
  | { settled: false };

/**
 * Classifies a `fetchMe` failure for the gate's admin check (PR #1875 review
 * finding).
 *
 * Every other `fetchMe`-gated view in this app (`OAuthView`, `TeamView`, …)
 * catches every failure the same way — `admin = false` — and that is safe
 * there: the worst case is a connect button staying disabled one round trip
 * longer. It is the wrong direction here, because `isAdmin: false` is what
 * makes `shouldShowOnboardingGate` suppress the blocking gate — a transient
 * failure (a dropped connection, a proxy 5xx) would resolve to "not admin"
 * exactly like a real 401 does, and fail the gate open for an actual admin
 * for the rest of that mount.
 *
 * A definitive `401` — no session on this host at all, whether because there
 * is no user plane or the operator is signed out — is the one answer that
 * genuinely means non-admin, and settles immediately, same as before.
 * Anything else (`ApiError` with any other status, or a raw `fetch` throw
 * that never reached the host) is not an answer about *who this user is* —
 * `settled: false` tells the caller to retry rather than guess.
 */
export function resolveGateAdminCheckError(error: unknown): GateAdminCheckOutcome {
  if (error instanceof ApiError && error.status === 401) {
    return { settled: true, isAdmin: false };
  }
  return { settled: false };
}

/** What a failed `GET {scope}/activation` read (behind `useActivationGate`) resolves to. */
export type ActivationReadOutcome =
  | { settled: true }
  | { settled: false };

/**
 * Classifies a `getActivation` failure for `useActivationGate`'s first read
 * (PR #1875 review finding, round 3).
 *
 * The hook's old `catch { setChecked(true) }` treated every failure the same
 * — a legacy host predating this route, a dropped connection, a proxy 5xx —
 * which settles `checked` with `status` left `null`. `shouldShowOnboardingGate`
 * renders the ordinary shell for that combination exactly as it would for
 * "not checked yet", so a real (non-activated) company whose *first* read
 * merely glitched got a multi-second window of the ordinary, unblocked shell
 * before a later poll succeeded and the gate abruptly replaced it — worse
 * than the same "abrupt" appearance on a normal first read, because that one
 * lands before the operator has had any chance to start clicking around.
 *
 * A `404` is the one answer that is genuinely final: this host does not have
 * the route at all, and retrying will not change that, so it settles —
 * `status` stays `null` and the gate stays off, same as before this fix.
 * Anything else (`ApiError` with any other status, or a raw `fetch` throw
 * that never reached the host) is not an answer about *whether this company
 * is activated* — `settled: false` tells the caller to retry rather than
 * guess, sooner than the regular poll cadence.
 */
export function resolveActivationReadError(error: unknown): ActivationReadOutcome {
  if (error instanceof ApiError && error.status === 404) {
    return { settled: true };
  }
  return { settled: false };
}

/**
 * Whether `useActivationGate`'s poll should keep running (PR #1875 review
 * finding, round 4).
 *
 * `GET {scope}/activation` is the only production caller of
 * `compute_and_latch` on the host — nothing else notices when the funnel's
 * three steps have all gone true and persists `activation_completed_at`. The
 * poll must therefore keep running for as long as the funnel is incomplete,
 * regardless of "skip for now": an admin who skips, then connects an
 * integration and runs a workflow from the ordinary shell, has genuinely
 * completed the funnel — but if the poll stopped the moment they skipped,
 * nothing would ever observe that and persist it. Reloading the same tab
 * preserves the session skip, and the funnel reads incomplete forever even
 * though every step is actually done.
 *
 * Only `isActivated` — the one thing the poll exists to notice — stops it;
 * `status: null` (nothing read yet) must keep polling, not stop it.
 */
export function shouldPollActivation(status: ActivationStatus | null): boolean {
  return status?.isActivated !== true;
}
