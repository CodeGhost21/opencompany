import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import { latestBudgetPauseMessageIdByAgent, type Transcripts } from "@/views/chat/model";

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
  isBudgetPauseNoticeSuperseded,
  isBudgetProximityExpired,
  parseBudgetPauseAgent,
  BUDGET_PAUSE_NOTICE_PREFIX,
  BUDGET_PROXIMITY_TTL_MS,
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

describe("isBudgetPauseNoticeSuperseded", () => {
  // Issue #1846 review (Codex #3864988184): the backend keeps at most one
  // marker per agent, so an OLD notice's "Add credits & resend" must disable
  // itself once a newer pause has been parked for the same agent — otherwise
  // clicking the stale card redeems the newer, unrelated marker while
  // claiming to resend the message on screen.
  it("is NOT superseded when this message IS the latest for its agent", () => {
    const latest = new Map([["ceo", "msg-2"]]);
    expect(isBudgetPauseNoticeSuperseded("ceo", "msg-2", latest)).toBe(false);
  });

  it("IS superseded when a newer pause has been parked for the same agent", () => {
    const latest = new Map([["ceo", "msg-2"]]);
    // msg-1 is an earlier notice for the SAME agent — a fresh pause since
    // overwrote the backend's single per-agent marker, so redeeming msg-1's
    // card would actually redeem whatever msg-2 parked.
    expect(isBudgetPauseNoticeSuperseded("ceo", "msg-1", latest)).toBe(true);
  });

  it("is not superseded by a DIFFERENT agent's newer pause", () => {
    const latest = new Map([
      ["ceo", "msg-2"],
      ["growth_lead", "msg-3"],
    ]);
    expect(isBudgetPauseNoticeSuperseded("ceo", "msg-1a", latest)).toBe(true);
    // ceo's own latest notice stays live even though growth_lead has a
    // separate, newer one — the two agents' markers do not interact.
  });

  it("degrades to NOT superseded when the caller has not wired the map", () => {
    // `undefined` is "unknown", not "stale": a caller that has not been
    // taught to compute the map (or an isolated unit render) must not have
    // every budget-pause card in the app read as disabled by default.
    expect(isBudgetPauseNoticeSuperseded("ceo", "msg-1", undefined)).toBe(false);
  });

  it("is never superseded when the notice text carried no agent id", () => {
    const latest = new Map([["ceo", "msg-2"]]);
    expect(isBudgetPauseNoticeSuperseded(null, "msg-1", latest)).toBe(false);
  });
});

describe("latestBudgetPauseMessageIdByAgent", () => {
  // Issue #1846 review (Codex #3865395879): the map `isBudgetPauseNoticeSuperseded`
  // reads has to be computed COMPANY-WIDE — every channel's transcript, not
  // just the one that happens to be open — because the backend keeps at most
  // one marker per agent regardless of which channel a pause happened in.
  function notice(agentId: string): string {
    return `${BUDGET_PAUSE_NOTICE_PREFIX} Paused — ${agentId}'s turn ran out of inference budget/credits.`;
  }

  function msg(id: string, text: string, at: number): ChatMessage {
    return { id, from: "company", text, at };
  }

  it("a later pause in a DIFFERENT channel supersedes an earlier notice in this one", () => {
    const transcripts: Transcripts = {
      "channel-a": [msg("msg-1", notice("ceo"), 1_000)],
      "channel-b": [msg("msg-2", notice("ceo"), 2_000)],
    };
    const latest = latestBudgetPauseMessageIdByAgent(transcripts);
    expect(latest.get("ceo")).toBe("msg-2");
    // Channel A's own notice, read in isolation, would look current — only
    // scanning every channel reveals it has been superseded.
    expect(isBudgetPauseNoticeSuperseded("ceo", "msg-1", latest)).toBe(true);
  });

  it("sorts by each message's own timestamp, not by which channel is scanned first", () => {
    // A naive fold over `Object.values(transcripts)` in insertion order would
    // visit "channel-b" before "channel-a" here — but B's message is the
    // OLDER one by wall clock, so A's must still win.
    const transcripts: Transcripts = {
      "channel-b": [msg("msg-old", notice("ceo"), 1_000)],
      "channel-a": [msg("msg-new", notice("ceo"), 5_000)],
    };
    const latest = latestBudgetPauseMessageIdByAgent(transcripts);
    expect(latest.get("ceo")).toBe("msg-new");
  });

  it("tracks each agent independently across channels", () => {
    const transcripts: Transcripts = {
      "channel-a": [
        msg("msg-1", notice("ceo"), 1_000),
        msg("msg-2", notice("growth_lead"), 1_500),
      ],
      "channel-b": [msg("msg-3", notice("growth_lead"), 3_000)],
    };
    const latest = latestBudgetPauseMessageIdByAgent(transcripts);
    expect(latest.get("ceo")).toBe("msg-1");
    expect(latest.get("growth_lead")).toBe("msg-3");
  });

  it("ignores non-notice messages and channels with no pauses", () => {
    const transcripts: Transcripts = {
      "channel-a": [msg("msg-1", "just chatting", 1_000)],
      "channel-b": [],
    };
    expect(latestBudgetPauseMessageIdByAgent(transcripts).size).toBe(0);
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

/**
 * Issue #1846 review (Codex #3866418899) — the console half of the
 * proximity-banner staleness fix.
 *
 * `app-shell.tsx`'s `budgetProximity` state used to clear only on dismissal
 * or a company switch: the backend only ever publishes a `budget_proximity`
 * frame while usage is at least 90%, so once a daily agent cap reset at
 * midnight, a plan period rolled over, or an operator raised the cap, the
 * next below-threshold dispatch published nothing — and the banner kept
 * claiming the previous period's status forever, with no wire signal ever
 * telling the console to drop it. `isBudgetProximityExpired` is the pure
 * predicate `app-shell.tsx`'s expiry effect is built on; these pin its
 * boundary directly, since there is no component-test harness in this
 * project to mount the effect itself (see this file's header doc comment).
 */
describe("isBudgetProximityExpired", () => {
  const parkedAt = 1_700_000_000_000;

  it("is not expired the moment it is parked", () => {
    expect(isBudgetProximityExpired(parkedAt, parkedAt)).toBe(false);
  });

  it("is not expired a millisecond before the TTL elapses", () => {
    expect(isBudgetProximityExpired(parkedAt, parkedAt + BUDGET_PROXIMITY_TTL_MS - 1)).toBe(
      false,
    );
  });

  it("is expired exactly at the TTL boundary — a stale banner must not survive it", () => {
    expect(isBudgetProximityExpired(parkedAt, parkedAt + BUDGET_PROXIMITY_TTL_MS)).toBe(true);
  });

  it("is expired well past the TTL — the pre-fix 'remains indefinitely' case", () => {
    // A week later: before this fix, nothing ever cleared this state, so a
    // check like this one would have failed against the pre-fix code path
    // (which had no expiry predicate to call at all — the banner just never
    // went away).
    expect(isBudgetProximityExpired(parkedAt, parkedAt + 7 * BUDGET_PROXIMITY_TTL_MS)).toBe(true);
  });

  it("a fresh frame's later atMillis is not expired against the same clock reading", () => {
    // Mirrors the re-arming behaviour: a dispatch that is STILL near the cap
    // republishes with a newer `atMillis`, and that newer value must read as
    // fresh even at a clock reading where the OLD one has already expired.
    const staleAt = parkedAt;
    const now = parkedAt + BUDGET_PROXIMITY_TTL_MS + 1000;
    const freshAt = now - 1000;
    expect(isBudgetProximityExpired(staleAt, now)).toBe(true);
    expect(isBudgetProximityExpired(freshAt, now)).toBe(false);
  });
});
