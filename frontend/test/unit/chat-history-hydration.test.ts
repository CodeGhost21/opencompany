import { describe, expect, it } from "vitest";

import {
  HISTORY_UNSTARTED,
  HISTORY_UNTRACKED,
  historyReady,
  type HistoryHydration,
} from "@/views/chat/model";

/**
 * The question issue #934 was filed over: may the timeline say this channel is
 * empty?
 *
 * The bug was not a rendering mistake — it was that nothing could answer this.
 * `Transcripts` maps a channel to `ChatMessage[]`, and `transcripts[id] ?? []`
 * turns "nobody has asked the host yet" into "the host says there is nothing",
 * so a reloaded DM with months of history rendered the copy that only belongs
 * on a channel that has never been used.
 *
 * These cases are the whole contract. Each is a state the console genuinely
 * reaches, and getting any of them wrong costs the operator something specific:
 * a false claim of emptiness (which reads as data loss) or a spinner that never
 * resolves (which reads as a hang).
 */

function hydration(over: Partial<HistoryHydration> = {}): HistoryHydration {
  return { discovered: false, byChannel: {}, ...over };
}

describe("whether a channel may be called empty", () => {
  it("holds while the channel's history request is in flight", () => {
    const h = hydration({ discovered: true, byChannel: { "dm:pm": "loading" } });
    expect(historyReady(h, "dm:pm")).toBe(false);
  });

  it("releases once the request has settled", () => {
    const h = hydration({ discovered: true, byChannel: { "dm:pm": "ready" } });
    expect(historyReady(h, "dm:pm")).toBe(true);
  });

  it("holds before the rehydration pass has reached the channel", () => {
    // The window that made the bug reachable at all: `ChatView` resolves its
    // own desk list independently of the shell's, so it can paint a channel
    // the shell's pass has not marked yet. No entry here does NOT mean
    // "nothing is coming" — it means "ask again in a moment".
    expect(historyReady(hydration({ byChannel: { other: "ready" } }), "dm:pm")).toBe(false);
  });

  it("releases an unknown channel once the pass has finished marking", () => {
    // A console-only teammate, or a host whose roster never named this channel:
    // the pass ran and did not claim it, so nothing will ever hydrate it.
    // Waiting forever would be a worse lie than the one this prevents.
    const h = hydration({ discovered: true, byChannel: { other: "ready" } });
    expect(historyReady(h, "dm:ghost")).toBe(true);
  });

  it("holds every channel before a company's pass begins", () => {
    expect(historyReady(HISTORY_UNSTARTED, "dm:pm")).toBe(false);
    expect(historyReady(HISTORY_UNSTARTED, "main")).toBe(false);
  });

  it("releases every channel when nothing is tracking hydration", () => {
    // A `ChatView` mounted without the shell behind it: there is no pass to
    // wait for, so it must render exactly as it did before this existed.
    expect(historyReady(HISTORY_UNTRACKED, "dm:pm")).toBe(true);
    expect(historyReady(HISTORY_UNTRACKED, "main")).toBe(true);
  });

  it("keeps a settled channel settled while a sibling is still loading", () => {
    // Per channel, not per pass — the operator should not wait on #general for
    // a DM that already answered.
    const h = hydration({
      discovered: true,
      byChannel: { main: "ready", "dm:pm": "loading" },
    });
    expect(historyReady(h, "main")).toBe(true);
    expect(historyReady(h, "dm:pm")).toBe(false);
  });
});
