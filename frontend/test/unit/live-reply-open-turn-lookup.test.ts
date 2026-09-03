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
 * conversation: a busy channel makes it say "yes, work remains" for a thread
 * that has none, so the settled thread's rows and receipt are never cleared and
 * accumulate for the life of the session.
 *
 * It does NOT erase a running turn's rows, and it is worth being exact about
 * why, because the opposite is the easy assumption. The desk-addressed version
 * reads the guard and writes the clear under the *same* key, so it only ever
 * clears rows belonging to turns it also checked — it is self-consistent, and
 * fails by omission rather than by over-clearing. What it drops on the floor is
 * every entry filed under a composite key.
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
    // Addressed by the desk the thread's list is invisible, so the guard reports
    // an idle thread. The clear that follows is addressed by the desk too, so
    // what it empties is the channel's (absent) rows and never this thread's —
    // the running sibling is not harmed, and the settled turn is not cleaned up.
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
