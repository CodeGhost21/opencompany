import { describe, expect, it } from "vitest";

import { originConversation } from "@/lib/task-origin";

/**
 * "Open the conversation" on a card used to land on `#/conversation` — the
 * legacy two-pane surface with its own thread rail — and select the thread
 * with a state setter the address knows nothing about. Chats are the Room tab
 * now (`#/chat/<channelId>`), so the card's host thread id is not an address
 * on its own: it has to be resolved through the shell's thread → channel map,
 * and that resolution can come back empty.
 *
 * The pure half of that decision lives in `lib/task-origin.ts` so the three
 * outcomes can be named here rather than inferred from a rendered button, and
 * so the "no channel carries it" case is a branch with a test rather than a
 * button that navigates nowhere.
 */
describe("where a card's origin conversation lives", () => {
  it("is nowhere for a card that was never opened from one", () => {
    expect(originConversation(undefined, { t1: "desk-eng" })).toEqual({ kind: "none" });
    expect(originConversation("", { t1: "desk-eng" })).toEqual({ kind: "none" });
  });

  it("is the channel that carries the origin thread", () => {
    expect(originConversation("t1", { t1: "desk-eng" })).toEqual({
      kind: "channel",
      channelId: "desk-eng",
    });
  });

  it("is unreachable when no channel carries the thread", () => {
    // A desk retired since the card was opened. The row must state the origin
    // and offer no jump — the pre-fix handler offered one unconditionally and
    // navigated to a surface that had nothing to show.
    expect(originConversation("gone", { t1: "desk-eng" })).toEqual({ kind: "unreachable" });
  });

  it("is unreachable before the channel map has loaded", () => {
    // The shell starts with an empty map and fills it once `/desks` answers.
    expect(originConversation("t1", {})).toEqual({ kind: "unreachable" });
    expect(originConversation("t1", undefined)).toEqual({ kind: "unreachable" });
  });

  it("folds the General spellings the host echoes back", () => {
    // Resolved through `channelForThread`, not a bare `map[originChatId]`: a
    // card opened from a line addressed `MAIN` carries that casing, and a
    // direct index misses it while the conversation plainly exists.
    expect(originConversation("MAIN", { main: "general" })).toEqual({
      kind: "channel",
      channelId: "general",
    });
  });
});
