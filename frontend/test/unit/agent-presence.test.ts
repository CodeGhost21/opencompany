import { describe, expect, it } from "vitest";

import {
  STALE_TURN_MS,
  approvalAgentCounts,
  derivePresence,
  dropTurnMeta,
  presenceIn,
  presenceOf,
  sameCounts,
  strongerPresence,
  type PresenceInputs,
} from "@/lib/agent-presence";

/**
 * `lib/agent-presence.ts`: the six-state answer to "what is this agent doing".
 * Pure, so every rule is asserted on plain objects: the precedence matrix, an
 * approval outliving its turn, queued, the DM-thread fallback, and the age-out.
 */

const NOW = 1_000_000_000;
const inputs = (over: Partial<PresenceInputs> = {}): PresenceInputs => ({
  openTurns: {},
  liveStepsByThread: {},
  liveStepsByMessage: {},
  liveAgentByTurn: {},
  turnMeta: {},
  ledgerTurns: [],
  approvalAgents: {},
  threadAgents: {},
  now: NOW,
  ...over,
});
const meta = (chatId: string, over: { replying?: boolean; lastFrameAt?: number } = {}) => ({
  chatId,
  replying: false,
  lastFrameAt: NOW - 1000,
  ...over,
});

describe("strongerPresence", () => {
  it("orders approval > working > typing > thinking > queued > inactive", () => {
    const order = ["approval", "working", "typing", "thinking", "queued", "inactive"] as const;
    for (let i = 0; i < order.length; i++) {
      for (let j = i + 1; j < order.length; j++) {
        expect(strongerPresence(order[j], order[i])).toBe(order[i]);
        expect(strongerPresence(order[i], order[j])).toBe(order[i]);
      }
    }
  });
});

describe("derivePresence", () => {
  it("reads inactive when nothing is open", () => {
    const index = derivePresence(inputs());
    expect(presenceOf(index, "rae")).toBe("inactive");
    expect(presenceIn(index, "rae", "rae")).toBe("inactive");
  });

  it("reads thinking for an accepted turn on a DM before any frame, via the thread's teammate", () => {
    const index = derivePresence(
      inputs({
        openTurns: { rae: [{ queued: false, chatId: "rae" }] },
        threadAgents: { rae: "rae" },
      }),
    );
    expect(presenceIn(index, "rae", "rae")).toBe("thinking");
    expect(presenceOf(index, "rae")).toBe("thinking");
  });

  it("joins the dm:<id> thread of a teammate named like a general channel", () => {
    const index = derivePresence(
      inputs({
        openTurns: { "dm:general": [{ queued: false, chatId: "dm:general" }] },
        threadAgents: { "dm:general": "general" },
      }),
    );
    expect(presenceIn(index, "general", "dm:general")).toBe("thinking");
  });

  it("reads queued only when every open turn is queued, and thinking beats it", () => {
    const queued = { queued: true, chatId: "rae" };
    const running = { queued: false, chatId: "rae" };
    const all = derivePresence(inputs({ openTurns: { rae: [queued, queued] }, threadAgents: { rae: "rae" } }));
    expect(presenceOf(all, "rae")).toBe("queued");
    const mixed = derivePresence(inputs({ openTurns: { rae: [running, queued] }, threadAgents: { rae: "rae" } }));
    expect(presenceOf(mixed, "rae")).toBe("thinking");
  });

  it("prefers the agent the frames named over the one the host started the turn on", () => {
    const index = derivePresence(
      inputs({
        openTurns: { desk: [{ queued: false, chatId: "desk", agentId: "ceo" }] },
        liveAgentByTurn: { desk: "engineer" },
        turnMeta: { desk: meta("desk") },
      }),
    );
    expect(presenceOf(index, "engineer")).toBe("thinking");
    expect(presenceOf(index, "ceo")).toBe("inactive");
  });

  it("reads working while a tool row runs, and thinking again once it finishes", () => {
    const running = derivePresence(
      inputs({
        liveAgentByTurn: { rae: "rae" },
        turnMeta: { rae: meta("rae") },
        liveStepsByThread: { rae: [{ status: "running" }] },
      }),
    );
    expect(presenceOf(running, "rae")).toBe("working");
    const done = derivePresence(
      inputs({
        liveAgentByTurn: { rae: "rae" },
        turnMeta: { rae: meta("rae") },
        liveStepsByThread: { rae: [{ status: "ok" }] },
      }),
    );
    expect(presenceOf(done, "rae")).toBe("thinking");
  });

  it("reads typing after a replying frame, and working wins if a tool is still running", () => {
    const typing = derivePresence(
      inputs({ liveAgentByTurn: { "m:1": "rae" }, turnMeta: { "m:1": meta("rae", { replying: true }) } }),
    );
    expect(presenceOf(typing, "rae")).toBe("typing");
    const both = derivePresence(
      inputs({
        liveAgentByTurn: { "m:1": "rae" },
        turnMeta: { "m:1": meta("rae", { replying: true }) },
        liveStepsByMessage: { "m:1": [{ status: "running" }] },
      }),
    );
    expect(presenceOf(both, "rae")).toBe("working");
  });

  it("has no timer on typing: a long reply stays typing until a frame or settle resets it", () => {
    const index = derivePresence(
      inputs({
        liveAgentByTurn: { rae: "rae" },
        turnMeta: { rae: meta("rae", { replying: true, lastFrameAt: NOW - 60_000 }) },
      }),
    );
    expect(presenceOf(index, "rae")).toBe("typing");
  });

  it("scopes a chat lookup to that chat, but not an approval", () => {
    const index = derivePresence(
      inputs({ liveAgentByTurn: { desk: "rae" }, turnMeta: { desk: meta("desk") } }),
    );
    expect(presenceIn(index, "rae", "desk")).toBe("thinking");
    expect(presenceIn(index, "rae", "rae")).toBe("inactive");
    const approving = derivePresence(inputs({ approvalAgents: { rae: 1 } }));
    expect(presenceIn(approving, "rae", "rae")).toBe("approval");
  });

  it("keeps approval after the turn has settled and counts several as one state", () => {
    const index = derivePresence(inputs({ approvalAgents: { rae: 3 } }));
    expect(presenceOf(index, "rae")).toBe("approval");
    expect(presenceOf(derivePresence(inputs({ approvalAgents: { rae: 0 } })), "rae")).toBe("inactive");
  });

  it("wins for approval over a running tool, and reads a parked live row as approval", () => {
    const parked = derivePresence(
      inputs({
        liveAgentByTurn: { rae: "rae" },
        turnMeta: { rae: meta("rae") },
        liveStepsByThread: { rae: [{ status: "awaiting_approval" }, { status: "running" }] },
      }),
    );
    expect(presenceOf(parked, "rae")).toBe("approval");
    const listed = derivePresence(
      inputs({
        liveAgentByTurn: { rae: "rae" },
        turnMeta: { rae: meta("rae") },
        liveStepsByThread: { rae: [{ status: "running" }] },
        approvalAgents: { rae: 1 },
      }),
    );
    expect(presenceOf(listed, "rae")).toBe("approval");
  });

  it("reads a ledger-only seat as working, and defers to the frames when they speak", () => {
    const seat = derivePresence(
      inputs({ ledgerTurns: [{ agentId: "seat", chatId: "desk", startedAtMillis: NOW - 5000 }] }),
    );
    expect(presenceOf(seat, "seat")).toBe("working");
    const framed = derivePresence(
      inputs({
        ledgerTurns: [{ agentId: "rae", chatId: "desk", startedAtMillis: NOW - 5000 }],
        liveAgentByTurn: { desk: "rae" },
        turnMeta: { desk: meta("desk", { replying: true }) },
      }),
    );
    expect(presenceOf(framed, "rae")).toBe("typing");
  });

  it("ignores a ledger turn with no agent (a chat-route bracket)", () => {
    const index = derivePresence(inputs({ ledgerTurns: [{ chatId: "desk", startedAtMillis: NOW }] }));
    expect(index.byAgent.size).toBe(0);
  });

  it("ages out a turn nothing has heard from, in the ledger and in the frames", () => {
    const old = NOW - STALE_TURN_MS - 1;
    const index = derivePresence(
      inputs({
        ledgerTurns: [{ agentId: "seat", startedAtMillis: old }],
        liveAgentByTurn: { desk: "rae" },
        turnMeta: { desk: meta("desk", { lastFrameAt: old }) },
      }),
    );
    expect(presenceOf(index, "seat")).toBe("inactive");
    expect(presenceOf(index, "rae")).toBe("inactive");
  });

  it("lets a recent frame keep a long-running ledger turn alive", () => {
    const index = derivePresence(
      inputs({
        ledgerTurns: [{ agentId: "rae", startedAtMillis: NOW - STALE_TURN_MS - 1 }],
        liveAgentByTurn: { other: "rae" },
        turnMeta: { other: meta("other") },
      }),
    );
    expect(presenceOf(index, "rae")).toBe("thinking");
  });
});

describe("helpers", () => {
  it("dropTurnMeta returns the same object when nothing matches and prunes when it does", () => {
    const all = { a: meta("x"), b: meta("y") };
    expect(dropTurnMeta(all, () => false)).toBe(all);
    expect(Object.keys(dropTurnMeta(all, (_k, m) => m.chatId === "x"))).toEqual(["b"]);
  });

  it("counts approvals per asker and skips the ones no agent raised", () => {
    const counts = approvalAgentCounts([{ agent: "a" }, { agent: "a" }, { agent: null }, {}]);
    expect(counts).toEqual({ a: 2 });
    expect(sameCounts(counts, { a: 2 })).toBe(true);
    expect(sameCounts(counts, { a: 1 })).toBe(false);
    expect(sameCounts(counts, {})).toBe(false);
  });
});
