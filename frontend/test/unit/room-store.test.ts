import { beforeEach, describe, expect, it } from "vitest";

import {
  currentScope,
  enterScope,
  readRoom,
  resetStore,
  setLiveStepsByThread,
  setOpenTurns,
  setTranscripts,
  setUnreadSince,
  writersForScope,
} from "@/room/store";

/**
 * The Room's state, out of the shell.
 *
 * Two properties are the whole reason this store is worth having, and both fail
 * silently when broken:
 *
 * - **Scope isolation.** A cache keyed on anything less than (connection,
 *   company) is how two hosts' data gets mixed — the warning
 *   `connections/registry.ts` opens with. Here it would show as another
 *   company's transcript appearing under this one's channel.
 * - **Snapshot identity per field.** `useSyncExternalStore` compares by
 *   identity, so a reader handing back a fresh object every call re-renders on
 *   every frame from every field. That does not error; it just makes the store
 *   slower than the prop drill it replaced.
 */

beforeEach(() => {
  resetStore();
});

describe("scope", () => {
  it("clears everything when the scope changes", () => {
    enterScope("conn-a::acme");
    setTranscripts({ general: [{ id: "h1", from: "you", text: "hi", at: 1 }] });
    expect(readRoom().transcripts.general).toHaveLength(1);

    enterScope("conn-a::other");
    expect(readRoom().transcripts).toEqual({});
    expect(currentScope()).toBe("conn-a::other");
  });

  it("separates two connections holding the same company id", () => {
    // The mixing bug in its exact shape: same company slug, different host.
    enterScope("conn-a::acme");
    setTranscripts({ general: [{ id: "h1", from: "you", text: "from A", at: 1 }] });
    enterScope("conn-b::acme");
    expect(readRoom().transcripts).toEqual({});
  });

  it("is idempotent, so re-entering does not wipe a live conversation", () => {
    // An effect that runs on every render must not be able to clear a
    // transcript mid-turn.
    enterScope("conn-a::acme");
    setTranscripts({ general: [{ id: "h1", from: "you", text: "hi", at: 1 }] });
    enterScope("conn-a::acme");
    expect(readRoom().transcripts.general).toHaveLength(1);
  });

  it("gives a fresh scope its own unread floor", () => {
    enterScope("conn-a::acme");
    setUnreadSince(1);
    enterScope("conn-a::other");
    // Not the previous company's reading point.
    expect(readRoom().unreadSince).toBeGreaterThan(1);
  });

  it("drops a late writer retained by an unmounted scope", () => {
    enterScope("conn-a::acme");
    const oldScopeWrites = writersForScope("conn-a::acme");

    enterScope("conn-a::other");
    oldScopeWrites.setTranscripts({
      general: [{ id: "late", from: "company", text: "from acme", at: 1 }],
    });

    expect(readRoom().transcripts).toEqual({});
  });
});

describe("snapshot identity", () => {
  it("leaves one field's value untouched when another is written", () => {
    enterScope("s");
    const before = readRoom().transcripts;
    setLiveStepsByThread({ t1: [] });
    expect(readRoom().transcripts).toBe(before);
  });

  it("leaves an untouched channel's array identical when a neighbour changes", () => {
    // What makes the narrow readers worth having: a busy channel must not
    // re-render every quiet one.
    enterScope("s");
    setTranscripts({
      a: [{ id: "h1", from: "you", text: "a", at: 1 }],
      b: [{ id: "h2", from: "you", text: "b", at: 2 }],
    });
    const quiet = readRoom().transcripts.a;
    setTranscripts((prev) => ({
      ...prev,
      b: [...prev.b, { id: "h3", from: "you", text: "b2", at: 3 }],
    }));
    expect(readRoom().transcripts.a).toBe(quiet);
  });

  it("does not notify when a write changes nothing", () => {
    // `setX(sameValue)` is common in reducers that bail out; emitting anyway
    // would re-render the whole Room for no reason.
    enterScope("s");
    const before = readRoom();
    const same = before.openTurns;
    setOpenTurns(same);
    expect(readRoom()).toBe(before);
  });
});

describe("useState semantics", () => {
  it("takes a value or an updater, exactly as the setter it replaces did", () => {
    // The property the migration rests on: ~50 shell call sites move across
    // unchanged because this contract is identical.
    enterScope("s");
    setOpenTurns({ t: [] });
    expect(readRoom().openTurns).toEqual({ t: [] });
    setOpenTurns((prev) => ({ ...prev, u: [] }));
    expect(Object.keys(readRoom().openTurns).sort()).toEqual(["t", "u"]);
  });

  it("hands the updater the current value", () => {
    enterScope("s");
    setUnreadSince(10);
    setUnreadSince((prev) => prev + 5);
    expect(readRoom().unreadSince).toBe(15);
  });
});
