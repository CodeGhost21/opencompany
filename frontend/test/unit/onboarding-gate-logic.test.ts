import { describe, expect, it } from "vitest";

import type { ActivationStatus } from "@/api/activation";
import { ApiError } from "@/api/types";
import {
  type GateDecisionInput,
  resolveActivationReadError,
  resolveGateAdminCheckError,
  shouldPollActivation,
  shouldPollActivationForRole,
  shouldShowOnboardingGate,
} from "@/onboarding/gate-logic";

const complete: ActivationStatus = {
  nameConfirmed: true,
  integrationConnected: true,
  workflowRunSucceeded: true,
  isActivated: true,
  activationCompletedAtMillis: 1_700_000_000_000,
};

const incomplete: ActivationStatus = {
  nameConfirmed: true,
  integrationConnected: false,
  workflowRunSucceeded: false,
  isActivated: false,
};

const base: GateDecisionInput = {
  status: null,
  checked: false,
  setupOpen: false,
  skippedThisSession: false,
  // Every pre-existing case below is written from an admin's point of view
  // (the only role the gate could originally distinguish), so the shared
  // fixture defaults to `true` and the admin-specific guard gets its own
  // cases below.
  isAdmin: true,
};

describe("shouldShowOnboardingGate", () => {
  it("does not block before the first read has landed", () => {
    expect(shouldShowOnboardingGate({ ...base, checked: false, status: null })).toBe(false);
  });

  it("blocks once the funnel reads incomplete", () => {
    expect(shouldShowOnboardingGate({ ...base, checked: true, status: incomplete })).toBe(true);
  });

  it("does not block once the funnel reads activated", () => {
    expect(shouldShowOnboardingGate({ ...base, checked: true, status: complete })).toBe(false);
  });

  it("blocks on isActivated alone, regardless of which individual steps still read false", () => {
    // The gate's only question is the latch, not a re-derivation of the three
    // steps — a status object with every individual flag false but
    // `isActivated: true` (the latch carrying an operator through a later
    // regression, per `ActivationStatus::is_activated` on the host) must still
    // read as "let them in".
    const latchedDespiteRegressedSteps: ActivationStatus = {
      nameConfirmed: false,
      integrationConnected: false,
      workflowRunSucceeded: false,
      isActivated: true,
      activationCompletedAtMillis: 1_700_000_000_000,
    };
    expect(
      shouldShowOnboardingGate({ ...base, checked: true, status: latchedDespiteRegressedSteps }),
    ).toBe(false);
  });

  it("holds while first-run setup (staffing) is still on screen, even with an incomplete funnel", () => {
    expect(
      shouldShowOnboardingGate({
        ...base,
        checked: true,
        status: incomplete,
        setupOpen: true,
      }),
    ).toBe(false);
  });

  it("never blocks once the operator skipped it this session", () => {
    expect(
      shouldShowOnboardingGate({
        ...base,
        checked: true,
        status: incomplete,
        skippedThisSession: true,
      }),
    ).toBe(false);
  });

  it("skip wins even over an in-progress setup dialog", () => {
    expect(
      shouldShowOnboardingGate({
        ...base,
        checked: true,
        status: incomplete,
        setupOpen: true,
        skippedThisSession: true,
      }),
    ).toBe(false);
  });

  // PR #1875 review finding: an invited member cannot clear any of the three
  // steps this gate blocks on — naming the company is `require_admin`-gated
  // on the host, and `OAuthView` disables every connect control unless
  // `/auth/me` reports `role === "admin"`. Their only way past an
  // unconditional gate was "Skip for now", which is deliberately
  // session-scoped (`onboarding/state.ts`) so it re-traps them on every new
  // tab. These three cases are what closes that dead end.
  describe("the admin-only guard (PR #1875)", () => {
    it("does not block before the admin read has landed, even with an incomplete funnel", () => {
      expect(
        shouldShowOnboardingGate({ ...base, checked: true, status: incomplete, isAdmin: null }),
      ).toBe(false);
    });

    it("never blocks a non-admin member — they cannot act on any step", () => {
      expect(
        shouldShowOnboardingGate({ ...base, checked: true, status: incomplete, isAdmin: false }),
      ).toBe(false);
    });

    it("still blocks an admin exactly as before once the read confirms the role", () => {
      expect(
        shouldShowOnboardingGate({ ...base, checked: true, status: incomplete, isAdmin: true }),
      ).toBe(true);
    });
  });
});

// PR #1875 review finding: `AppShell`'s `isGateAdmin` effect caught every
// `fetchMe` failure the same way — treat as non-admin — mirroring every
// other admin-gated view's `catch { admin = false }` pattern. That pattern is
// safe on a read-only view (a button stays disabled one round trip longer).
// It is wrong here: `isAdmin: false` fails the *blocking* gate open, so a
// transient failure (a dropped connection, a 5xx) would let an actual admin
// past onboarding for the rest of that mount. A definitive `401` — no
// session at all — is the one answer that genuinely means non-admin and must
// still settle immediately; everything else must be retried instead.
describe("resolveGateAdminCheckError", () => {
  it("settles to non-admin on a definitive 401 — no session on this host", () => {
    const outcome = resolveGateAdminCheckError(new ApiError(401, "no_session", "no session"));
    expect(outcome).toEqual({ settled: true, isAdmin: false });
  });

  it("does not settle on a network failure — retry instead of failing the gate open", () => {
    const outcome = resolveGateAdminCheckError(new ApiError(0, "network_error", "offline"));
    expect(outcome.settled).toBe(false);
  });

  it("does not settle on a 5xx — the host, not the session, is the problem", () => {
    const outcome = resolveGateAdminCheckError(new ApiError(503, "unavailable", "quiescing"));
    expect(outcome.settled).toBe(false);
  });

  it("does not settle on a non-ApiError throw (e.g. a raw fetch TypeError)", () => {
    const outcome = resolveGateAdminCheckError(new TypeError("Failed to fetch"));
    expect(outcome.settled).toBe(false);
  });
});

// PR #1875 review finding, round 3: `useActivationGate`'s first-read catch
// treated every failure the same — settle `checked` with `status` left
// `null`. That is correct for a legacy host that will never have this route
// (a `404`), but wrong for a transient failure (a dropped connection, a
// 5xx): it fails the gate open for up to a full poll interval, on the very
// read that decides whether a real, non-activated company gets blocked.
describe("resolveActivationReadError", () => {
  it("settles on a definitive 404 — this host has no such route", () => {
    const outcome = resolveActivationReadError(new ApiError(404, "not_found", "no route"));
    expect(outcome).toEqual({ settled: true });
  });

  it("does not settle on a network failure — retry instead of failing the gate open", () => {
    const outcome = resolveActivationReadError(new ApiError(0, "network_error", "offline"));
    expect(outcome.settled).toBe(false);
  });

  it("does not settle on a 5xx — the host, not the route, is the problem", () => {
    const outcome = resolveActivationReadError(new ApiError(503, "unavailable", "quiescing"));
    expect(outcome.settled).toBe(false);
  });

  it("does not settle on a non-ApiError throw (e.g. a raw fetch TypeError)", () => {
    const outcome = resolveActivationReadError(new TypeError("Failed to fetch"));
    expect(outcome.settled).toBe(false);
  });
});

// PR #1875 review finding, round 4: the shell wired the poll's `enabled` to
// `!gateSkipped`, so "skip for now" stopped `useActivationGate` from ever
// reading again. `GET {scope}/activation` is the only production caller of
// `compute_and_latch`, so a funnel that genuinely completes after a skip
// (an admin connects an integration and runs a workflow from the ordinary
// shell) never got its `activation_completed_at` persisted at all.
describe("shouldPollActivation", () => {
  it("keeps polling before the first read lands", () => {
    expect(shouldPollActivation(null)).toBe(true);
  });

  it("keeps polling while the funnel reads incomplete — this is what a skip must not silence", () => {
    expect(shouldPollActivation(incomplete)).toBe(true);
  });

  it("stops once the latch is set — nothing left for the poll to notice", () => {
    expect(shouldPollActivation(complete)).toBe(false);
  });
});

// PR #1875 review finding, round 5: the shell wired the poll's `enabled` to
// a bare `true`, so every invited member's tab kept polling `compute_and_latch`
// for as long as the company stayed unactivated — even though none of the
// three funnel steps can ever be cleared by a non-admin, so that poll can
// never be the read that observes the funnel complete.
describe("shouldPollActivationForRole", () => {
  it("keeps polling before the admin check has landed — must not cost a real admin their fast first read", () => {
    expect(shouldPollActivationForRole(null)).toBe(true);
  });

  it("keeps polling for a confirmed admin", () => {
    expect(shouldPollActivationForRole(true)).toBe(true);
  });

  it("stops for a confirmed non-admin — this is what round 5 was filed about", () => {
    expect(shouldPollActivationForRole(false)).toBe(false);
  });
});
