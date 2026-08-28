import { describe, expect, it } from "vitest";

import type { ActivationStatus } from "@/api/activation";
import { type GateDecisionInput, shouldShowOnboardingGate } from "@/onboarding/gate-logic";

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

  // PR #1878 review finding: an invited member cannot clear any of the three
  // steps this gate blocks on — naming the company is `require_admin`-gated
  // on the host, and the Composio connect routes use `AdminScopedCompany`.
  // Their only way past an unconditional gate was "Skip for now", which is
  // deliberately session-scoped (`onboarding/state.ts`) so it re-traps them
  // on every new tab. These three cases are what closes that dead end.
  describe("the admin-only guard (PR #1878)", () => {
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
