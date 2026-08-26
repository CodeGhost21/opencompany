import { beforeEach, describe, expect, it, vi } from "vitest";

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

const {
  handleEvent,
  isBudgetPauseNotice,
  parseBudgetPauseAgent,
  BUDGET_PAUSE_NOTICE_PREFIX,
} = await import("@/hooks/use-events");
type Ev = import("@/hooks/use-events").CompanyStreamEvent;
type Subs = import("@/hooks/use-events").Subscribers;

/**
 * Issue #1846 — the console half of the top-level budget-pause fix.
 *
 * There is no component-test harness in this project (no
 * `@testing-library/react` anywhere under `test/`, per
 * `workflow-invalid-problems.test.ts`'s doc comment), so the JSX in
 * `MessageRow.tsx`'s `SystemPill` that renders the "Add credits" card is
 * guarded by the compiler and by reading, not by a render test. What CAN be
 * pinned, and is pinned here, is the pure logic that JSX depends on: the
 * notice-detection prefix, the agent-id parse, and the live-frame routing.
 */
describe("isBudgetPauseNotice", () => {
  it("recognises the host's exact prefix", () => {
    expect(
      isBudgetPauseNotice(
        `${BUDGET_PAUSE_NOTICE_PREFIX} Paused — ceo's turn ran out of inference budget/credits.`,
      ),
    ).toBe(true);
  });

  it("does not misfire on an ordinary reply that happens to mention credits", () => {
    expect(isBudgetPauseNotice("You have 100 remaining credits this month.")).toBe(false);
    expect(isBudgetPauseNotice("Paused — waiting for your review.")).toBe(false);
    expect(isBudgetPauseNotice("")).toBe(false);
  });
});

describe("parseBudgetPauseAgent", () => {
  it("extracts the teammate id from the host's exact wording", () => {
    const text =
      `${BUDGET_PAUSE_NOTICE_PREFIX} Paused — ceo's turn ran out of inference budget/credits, ` +
      "so it stopped instead of failing silently. Add credits to your account, then resend " +
      "your message to continue. Details:\nhosted inference returned 400: insufficient budget";
    expect(parseBudgetPauseAgent(text)).toBe("ceo");
  });

  it("extracts a multi-word-safe agent id (underscored, not spaced)", () => {
    const text = `${BUDGET_PAUSE_NOTICE_PREFIX} Paused — growth_lead's turn ran out of inference budget/credits.`;
    expect(parseBudgetPauseAgent(text)).toBe("growth_lead");
  });

  it("returns null for text that is not this notice — the CTA must not guess", () => {
    expect(parseBudgetPauseAgent("Acknowledged.")).toBeNull();
    expect(parseBudgetPauseAgent("The reply above is where this turn stopped")).toBeNull();
    expect(parseBudgetPauseAgent("")).toBeNull();
  });
});

describe("budget_proximity routing", () => {
  function subscribers() {
    return {
      onAgentReply: vi.fn(),
      onTaskEvent: vi.fn(),
      onWorkspaceEvent: vi.fn(),
      onTurnEvent: vi.fn(),
      onBudgetProximityEvent: vi.fn(),
      onWorkflowRunEvent: vi.fn(),
      onWorkflowChanged: vi.fn(),
      onApprovalEvent: vi.fn(),
      onResync: vi.fn(),
    } satisfies Subs;
  }

  const frame = (agentId?: string): Ev => ({
    type: "budget_proximity",
    agentId,
    message: "This company is nearing its token budget for the current period.",
    atMillis: 1_700_000_000_000,
  });

  beforeEach(() => {
    for (const fn of Object.values(toasts)) fn.mockClear();
  });

  it("reaches the proximity subscriber and no other", () => {
    const subs = subscribers();
    handleEvent(frame("ceo"), subs);

    expect(subs.onBudgetProximityEvent).toHaveBeenCalledTimes(1);
    expect(subs.onBudgetProximityEvent).toHaveBeenCalledWith(frame("ceo"));
    expect(subs.onTurnEvent).not.toHaveBeenCalled();
    expect(subs.onAgentReply).not.toHaveBeenCalled();
    expect(subs.onTaskEvent).not.toHaveBeenCalled();
  });

  it("raises no toast — a coarse warning is a banner, not an interruption", () => {
    const subs = subscribers();
    handleEvent(frame(undefined), subs);
    for (const fn of Object.values(toasts)) expect(fn).not.toHaveBeenCalled();
  });

  it("carries the company-wide shape (no agentId) as well as the per-agent shape", () => {
    const subs = subscribers();
    handleEvent(frame(undefined), subs);
    const [received] = subs.onBudgetProximityEvent.mock.calls[0] as [Ev];
    expect(received).toMatchObject({ type: "budget_proximity", agentId: undefined });
  });
});
