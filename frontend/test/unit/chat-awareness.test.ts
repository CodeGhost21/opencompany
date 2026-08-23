import { describe, expect, it } from "vitest";

import {
  applyPresence,
  IDLE_AWAY_MS,
  livePeers,
  presenceToAnnounce,
  PRESENCE_HEARTBEAT_MS,
  PRESENCE_TTL_MS,
  pruneTypers,
  shouldShowTyping,
  typersIn,
  typingExpiry,
  typingLabel,
  TYPING_RECEIVE_TTL_MS,
  type Peer,
  type Typer,
} from "@/lib/awareness";

/**
 * Presence and typing are the two features where being slightly wrong is worse
 * than being absent: a dot that flickers, or a "still typing…" for somebody who
 * finished a paragraph ago, teaches people to ignore the indicator entirely.
 * Every rule below is a case that produced exactly that.
 */

describe("presenceToAnnounce", () => {
  it("is online while the human is at the machine", () => {
    expect(presenceToAnnounce("auto", 0)).toBe("online");
    expect(presenceToAnnounce("auto", IDLE_AWAY_MS - 1)).toBe("online");
  });

  it("goes away only after a real idle stretch", () => {
    expect(presenceToAnnounce("auto", IDLE_AWAY_MS)).toBe("away");
  });

  it("lets the viewer override in both directions", () => {
    expect(presenceToAnnounce("away", 0)).toBe("away");
    expect(presenceToAnnounce("offline", 0)).toBe("offline");
    // An override wins even when they are plainly at the machine.
    expect(presenceToAnnounce("offline", IDLE_AWAY_MS)).toBe("offline");
  });
});

describe("applyPresence", () => {
  const peers = (): Map<string, Peer> =>
    new Map([["u1", { status: "online" as const, atMillis: 1_000 }]]);

  it("adds somebody who arrives", () => {
    const next = applyPresence(new Map(), {
      userId: "u1",
      status: "online",
      atMillis: 1,
    });
    expect(next?.get("u1")?.status).toBe("online");
  });

  it("removes somebody who goes offline", () => {
    const next = applyPresence(peers(), {
      userId: "u1",
      status: "offline",
      atMillis: 2_000,
    });
    expect(next?.has("u1")).toBe(false);
  });

  /** No change means no new map, so nothing downstream re-renders. */
  it("reports no change for a repeat of what it already knows", () => {
    expect(
      applyPresence(peers(), { userId: "u1", status: "online", atMillis: 2_000 }),
    ).toBeNull();
  });

  /**
   * A reconnect can replay frames out of order, and an older one must not undo
   * a newer one — otherwise somebody who just went away flips back to online.
   */
  it("ignores a frame older than what it already has", () => {
    expect(
      applyPresence(peers(), { userId: "u1", status: "away", atMillis: 500 }),
    ).toBeNull();
  });

  it("applies a newer status change", () => {
    const next = applyPresence(peers(), {
      userId: "u1",
      status: "away",
      atMillis: 2_000,
    });
    expect(next?.get("u1")?.status).toBe("away");
  });

  it("ignores an offline frame for somebody it never had", () => {
    expect(
      applyPresence(new Map(), { userId: "ghost", status: "offline", atMillis: 1 }),
    ).toBeNull();
  });
});

describe("livePeers", () => {
  const peers = new Map<string, Peer>([["u1", { status: "online", atMillis: 0 }]]);

  /** The 3x margin: a missed heartbeat must not blink somebody out. */
  it("keeps somebody through a missed heartbeat", () => {
    expect(livePeers(peers, PRESENCE_HEARTBEAT_MS * 2).size).toBe(1);
  });

  it("drops somebody whose lease lapsed", () => {
    expect(livePeers(peers, PRESENCE_TTL_MS + 1).size).toBe(0);
  });
});

describe("shouldShowTyping", () => {
  it("shows a fresh frame", () => {
    expect(shouldShowTyping({ atMillis: 1_000 }, 1_100, undefined)).toBe(true);
  });

  /** Rule 1: a delayed or replayed frame must not resurrect an indicator. */
  it("drops a frame that is already older than its own TTL", () => {
    expect(
      shouldShowTyping({ atMillis: 0 }, TYPING_RECEIVE_TTL_MS, undefined),
    ).toBe(false);
  });

  /** Rule 2: somebody who has just spoken is not still typing. */
  it("drops a frame stamped before that author's own message", () => {
    expect(shouldShowTyping({ atMillis: 1_000 }, 1_100, 1_500)).toBe(false);
  });

  /** Rule 3: the in-flight ping that lands just after the message. */
  it("suppresses a frame stamped just after that author's message", () => {
    expect(shouldShowTyping({ atMillis: 1_600 }, 1_700, 1_500)).toBe(false);
  });

  it("shows them again once they genuinely start typing after it", () => {
    expect(shouldShowTyping({ atMillis: 5_000 }, 5_100, 1_500)).toBe(true);
  });
});

describe("typingExpiry", () => {
  /**
   * A frame delayed in transit must not extend the bubble past its own
   * validity — otherwise a slow network makes indicators linger.
   */
  it("never extends past the frame's own lifetime", () => {
    const delayed = typingExpiry({ atMillis: 0 }, 5_000);
    expect(delayed).toBe(TYPING_RECEIVE_TTL_MS);
  });

  it("uses now for a frame that arrived promptly", () => {
    expect(typingExpiry({ atMillis: 1_000 }, 1_000)).toBe(
      1_000 + TYPING_RECEIVE_TTL_MS,
    );
  });
});

describe("typers", () => {
  const typer = (over: Partial<Typer> & Pick<Typer, "userId">): Typer => ({
    chatId: "eng",
    expiresAt: 10_000,
    firstSeenAt: 0,
    ...over,
  });

  it("prunes the expired", () => {
    const kept = pruneTypers([typer({ userId: "a", expiresAt: 5 })], 10);
    expect(kept).toEqual([]);
  });

  it("keeps each channel's typers to itself", () => {
    const all = [typer({ userId: "a" }), typer({ userId: "b", chatId: "design" })];
    expect(typersIn(all, "eng", undefined, 0).map((t) => t.userId)).toEqual(["a"]);
  });

  it("keeps a thread's typers out of the channel", () => {
    const all = [typer({ userId: "a" }), typer({ userId: "b", parentId: "42" })];
    expect(typersIn(all, "eng", undefined, 0).map((t) => t.userId)).toEqual(["a"]);
    expect(typersIn(all, "eng", "42", 0).map((t) => t.userId)).toEqual(["b"]);
  });

  /**
   * Sorted by first-seen, not by latest frame: renewals arrive every few
   * seconds, and re-sorting on them reshuffles the line although nobody
   * started or stopped.
   */
  it("orders by who started first, so renewals do not reshuffle it", () => {
    const all = [
      typer({ userId: "b", firstSeenAt: 200 }),
      typer({ userId: "a", firstSeenAt: 100 }),
    ];
    expect(typersIn(all, "eng", undefined, 0).map((t) => t.userId)).toEqual([
      "a",
      "b",
    ]);
  });
});

describe("typingLabel", () => {
  it("names one and two, and counts past that", () => {
    expect(typingLabel([])).toBeNull();
    expect(typingLabel(["Jane"])).toBe("Jane is typing…");
    expect(typingLabel(["Jane", "Ada"])).toBe("Jane and Ada are typing…");
    expect(typingLabel(["Jane", "Ada", "Rae"])).toBe("Several people are typing…");
  });
});
