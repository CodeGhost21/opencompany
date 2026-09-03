import { describe, expect, it } from "vitest";

import { deskOfStateKey, hasOtherOpenTurns, turnStateKey, type OpenTurn } from "@/lib/live-reply";

/**
 * Why a settled turn's cleanup must be addressed by the **state key** and not
 * by the desk, even though the same call needs the desk to reach the host.
 *
 * `hasOtherOpenTurns` is the guard on a thread-wide clear: it answers "is there
 * still work here?", and the caller erases that thread's live rows when it says
 * no. Since #2042 the map it reads is keyed per thread (`engineering#41`), and
 * `ChatView` hands `onSendStart` that same key — so `liveStepsByThread` and
 * `receiptByThread` are keyed by it too.
 *
 * Look the **desk** up in that map and the guard answers about a different
 * conversation entirely, in both directions. Both are silent, and both are
 * wrong in a way an operator eventually sees:
 *
 *   * a busy channel makes it say "yes, work remains" for a thread that has
 *     none, so the settled thread's rows are never cleared; and
 *   * a quiet channel makes it say "no work here" while the thread is still
 *     running, which erases the rows of a turn that is currently working —
 *     the "teammate that stopped" appearance the guard exists to prevent
 *     (PR #1904 review).
 *
 * The desk is still what `chat/history` is addressed by, which is why the two
 * identities are passed separately rather than one being derived and used for
 * everything (Codex review on #2044).
 */
describe("open-turn lookups are addressed by the state key, not the desk", () => {
  const running: OpenTurn = { turnId: "t-running", queued: false, chatId: "engineering" };
  const settled: OpenTurn = { turnId: "t-settled", queued: false, chatId: "engineering" };

  it("sees a thread's own sibling only under the composite key", () => {
    const open = { [turnStateKey("engineering", 41)]: [settled, running] };

    // Addressed correctly: the sibling is found, so the clear is held back.
    expect(hasOtherOpenTurns(open, "engineering#41", "t-settled")).toBe(true);
    // Addressed by the desk: the thread's list is invisible, so the guard
    // reports an idle thread and the running sibling's rows get erased.
    expect(hasOtherOpenTurns(open, "engineering", "t-settled")).toBe(false);
  });

  it("does not let a busy channel speak for an idle thread", () => {
    // The mirror image: work in the channel, none left in the thread.
    const open = {
      engineering: [running],
      [turnStateKey("engineering", 41)]: [settled],
    };

    // The thread is done, so its rows should be cleared.
    expect(hasOtherOpenTurns(open, "engineering#41", "t-settled")).toBe(false);
    // Addressed by the desk, the channel's running turn blocks that clear and
    // the settled thread keeps its live rows indefinitely.
    expect(hasOtherOpenTurns(open, "engineering", "t-settled")).toBe(true);
  });

  it("collapses to one identity for an unthreaded send, which is why this hid", () => {
    // A channel send keys the map on the bare desk, so both addressings agree
    // and every existing test and spec passes either way. The threaded case is
    // the only one that can tell them apart.
    const key = turnStateKey("engineering", undefined);
    expect(key).toBe(deskOfStateKey(key));
    const open = { [key]: [settled, running] };
    expect(hasOtherOpenTurns(open, key, "t-settled")).toBe(
      hasOtherOpenTurns(open, deskOfStateKey(key), "t-settled"),
    );
  });
});
