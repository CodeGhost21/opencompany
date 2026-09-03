import { describe, expect, it } from "vitest";

import { openTurnsFromRuns, turnStateKey, type OpenRunRow } from "@/lib/live-reply";

/**
 * Live-turn state is keyed per **thread**, not per channel.
 *
 * A channel has held many threads since #1890, and every run row names only its
 * channel (`chatId`). Keying the shell's open-turn / live-step / receipt maps on
 * that meant two concurrent turns in one channel shared a slot, and — because
 * the console could not then tell whose turn was running — `ChatView` gave up
 * and suppressed the working indicator for the whole channel whenever any
 * thread was open.
 *
 * That was not a cosmetic mix-up. Driving a real host with a thread open, the
 * runs API reported `{"chatId":"engineering","status":"running"}` while the
 * console showed no working row anywhere: a turn in flight, and a UI that said
 * nothing about it.
 */
describe("openTurnsFromRuns keys on the thread, not the channel", () => {
  const run = (over: Partial<OpenRunRow> & { id: string }): OpenRunRow => ({
    chatId: "engineering",
    status: "running",
    ...over,
  });

  it("gives two threads of one channel their own slots", () => {
    const open = openTurnsFromRuns([
      run({ id: "t1", threadRoot: 11 }),
      run({ id: "t2", threadRoot: 22 }),
    ]);

    expect(Object.keys(open).sort()).toEqual(["engineering#11", "engineering#22"]);
    // `chatId` rides alongside the composite key so the settle poll can re-read
    // the real desk (Codex review on #2042).
    expect(open["engineering#11"]).toEqual([
      { turnId: "t1", queued: false, chatId: "engineering" },
    ]);
    expect(open["engineering#22"]).toEqual([
      { turnId: "t2", queued: false, chatId: "engineering" },
    ]);
  });

  it("keeps an unrooted turn on the channel, exactly where it was before", () => {
    // A message sent from the channel composer is deliberately unrooted: its
    // turn is the channel's own. This is also every row written before the host
    // carried a root, so an un-upgraded host keeps its indicator rather than
    // losing it.
    const open = openTurnsFromRuns([run({ id: "t1" })]);

    expect(Object.keys(open)).toEqual(["engineering"]);
    expect(turnStateKey("engineering", undefined)).toBe("engineering");
  });

  it("does not merge a thread's turn into the channel's slot", () => {
    // The collision the old keying produced: both rows landed under
    // `engineering`, and `[0]` — what the indicator reads — was whichever
    // sorted first.
    const open = openTurnsFromRuns([run({ id: "channel" }), run({ id: "threaded", threadRoot: 11 })]);

    expect(open["engineering"]).toEqual([
      { turnId: "channel", queued: false, chatId: "engineering" },
    ]);
    expect(open["engineering#11"]).toEqual([
      { turnId: "threaded", queued: false, chatId: "engineering" },
    ]);
  });

  it("still keeps a running turn ahead of one queued behind it in the same thread", () => {
    // The per-thread ordering #1000 established has to survive the re-key: the
    // head is what the indicator reads, and "queued" and "working" are
    // different things to tell an operator.
    const open = openTurnsFromRuns([
      run({ id: "queued", threadRoot: 11, status: "pending" }),
      run({ id: "running", threadRoot: 11 }),
    ]);

    expect(open["engineering#11"]).toEqual([
      { turnId: "running", queued: false, chatId: "engineering" },
      { turnId: "queued", queued: true, chatId: "engineering" },
    ]);
  });

  it("skips a run with no conversation at all", () => {
    // A card dispatch and a workflow node own no thread's indicator.
    expect(openTurnsFromRuns([{ id: "dispatch", status: "running" }])).toEqual({});
  });
});
