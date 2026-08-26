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
});
