import { describe, expect, it } from "vitest";

import {
  applyPresence,
  IDLE_AWAY_MS,
  livePeers,
  presenceToAnnounce,
  reconcilePresenceSnapshot,
  PRESENCE_HEARTBEAT_MS,
  PRESENCE_TTL_MS,
  pruneTypers,
  shouldShowTyping,
  typersIn,
  typingExpiry,
  typingLabel,
  TYPING_RECEIVE_TTL_MS,
  type Peer,
  type PresenceStatus,
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

/**
 * Every one of these iterates something the host supplied. The types promise an
 * array; a host promises nothing. A stub answering `[]` for unrecognised routes
 * made `res.people` `undefined`, and iterating it threw during render — blanking
 * the entire console and failing every test in an unrelated spec file. Presence
 * is the least important thing on the screen and has to fail like it.
 */
describe("malformed host responses", () => {
  it("reconcilePresenceSnapshot treats a missing snapshot as no news", () => {
    const peers = new Map([["u1", { status: "online" as const, atMillis: 1 }]]);
    for (const bad of [undefined, null, "nope", 7, {}]) {
      const out = reconcilePresenceSnapshot(
        peers,
        new Map(),
        bad as unknown as Array<{ userId: string; status: PresenceStatus; atMillis: number }>,
        1_000,
      );
      // Unchanged, not emptied: absent news must not evict somebody we know of.
      expect(out.get("u1")?.status).toBe("online");
    }
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

describe("reconcilePresenceSnapshot", () => {
  /**
   * Regression coverage for the race `usePresence`'s periodic snapshot
   * refresh introduced: a `GET /presence` in flight racing a newer SSE frame
   * must never let the (now stale) response win.
   */
  it("keeps a newer local status over an older snapshot row for the same peer", () => {
    const peers = new Map<string, Peer>([["u1", { status: "away", atMillis: 2_000 }]]);
    const next = reconcilePresenceSnapshot(
      peers,
      new Map(),
      [{ userId: "u1", status: "online", atMillis: 1_000 }],
      500,
    );
    expect(next.get("u1")).toEqual({ status: "away", atMillis: 2_000 });
  });

  it("applies a snapshot row newer than what is locally held", () => {
    const peers = new Map<string, Peer>([["u1", { status: "away", atMillis: 1_000 }]]);
    const next = reconcilePresenceSnapshot(
      peers,
      new Map(),
      [{ userId: "u1", status: "online", atMillis: 2_000 }],
      500,
    );
    expect(next.get("u1")).toEqual({ status: "online", atMillis: 2_000 });
  });

  it("does not resurrect somebody an offline tombstone already removed", () => {
    // No entry in `peers` for u1 — a live "offline" frame already deleted it.
    const tombstones = new Map([["u1", 3_000]]);
    const next = reconcilePresenceSnapshot(
      new Map(),
      tombstones,
      [{ userId: "u1", status: "online", atMillis: 2_000 }],
      500,
    );
    expect(next.has("u1")).toBe(false);
  });

  it("applies a snapshot row newer than a stale offline tombstone", () => {
    const tombstones = new Map([["u1", 1_000]]);
    const next = reconcilePresenceSnapshot(
      new Map(),
      tombstones,
      [{ userId: "u1", status: "online", atMillis: 2_000 }],
      500,
    );
    expect(next.get("u1")).toEqual({ status: "online", atMillis: 2_000 });
  });

  it("does not drop a peer the snapshot omits if a live frame raced ahead of the request", () => {
    // u2 arrived (via a live frame) *after* the snapshot request was sent —
    // the snapshot's silence about them predates that arrival.
    const peers = new Map<string, Peer>([["u2", { status: "online", atMillis: 5_000 }]]);
    const next = reconcilePresenceSnapshot(peers, new Map(), [], 1_000);
    expect(next.get("u2")).toEqual({ status: "online", atMillis: 5_000 });
  });

  it("drops a peer the snapshot omits when what we knew predates the request", () => {
    const peers = new Map<string, Peer>([["u2", { status: "online", atMillis: 500 }]]);
    const next = reconcilePresenceSnapshot(peers, new Map(), [], 1_000);
    expect(next.has("u2")).toBe(false);
  });
});

describe("shouldShowTyping", () => {
  it("shows a fresh frame", () => {
    expect(shouldShowTyping({ atMillis: 1_000 }, undefined)).toBe(true);
  });

  /**
   * There is deliberately no "frame older than its own TTL" case here anymore.
   * That rule compared the frame's *server*-stamped `atMillis` against the
   * *browser's* `now`, so a viewer whose workstation clock ran fast by more
   * than the TTL saw every typing frame read as already-expired on arrival —
   * typing was silently broken for them. Freshness is `typingExpiry`'s job
   * now, anchored to local receipt time instead of a comparison across clock
   * domains. See both functions' doc comments.
   */
  it("does not depend on the receiving console's own clock at all", () => {
    // A frame nominally many minutes "old" by a naive `now - atMillis` check
    // must still be shown: nothing here reads wall-clock time.
    expect(shouldShowTyping({ atMillis: 0 }, undefined)).toBe(true);
  });

  /** Rule 1: somebody who has just spoken is not still typing. */
  it("drops a frame stamped before that author's own message", () => {
    expect(shouldShowTyping({ atMillis: 1_000 }, 1_500)).toBe(false);
  });

  /** Rule 2: the in-flight ping that lands just after the message. */
  it("suppresses a frame stamped just after that author's message", () => {
    expect(shouldShowTyping({ atMillis: 1_600 }, 1_500)).toBe(false);
  });

  it("shows them again once they genuinely start typing after it", () => {
    expect(shouldShowTyping({ atMillis: 5_000 }, 1_500)).toBe(true);
  });
});

describe("typingExpiry", () => {
  /**
   * Anchored purely to the receipt time the caller passes in — never to the
   * frame's own (server-clock) timestamp, which is exactly what fixes the
   * clock-skew bug: a workstation clock running arbitrarily far ahead of or
   * behind the host no longer changes when the indicator disappears.
   */
  it("is a TTL from the given now, regardless of any server timestamp", () => {
    expect(typingExpiry(1_000)).toBe(1_000 + TYPING_RECEIVE_TTL_MS);
    expect(typingExpiry(5_000)).toBe(5_000 + TYPING_RECEIVE_TTL_MS);
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
