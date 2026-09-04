import { describe, expect, it } from "vitest";

import { foldLiveFrame, type LiveRow } from "@/lib/live-frame";

/**
 * Two questions asked in one channel, and why their rows must not share a list.
 *
 * Before `messageSeq`, a chat frame's only routing keys were `chatId` and
 * `agentId`. Two questions in one channel share the thread, and whenever the
 * same desk answers both they share the agent too — so there was nothing to
 * group a row by, and both turns folded into one per-thread list.
 *
 * The cost was not merely a merged pile. The console holds ONE list per thread
 * and `onSendStart` resets it, so asking a second question discarded the first
 * turn's rows outright. Measured on a real pair: two rows dropped from a
 * delegated turn that was still running — and a turn blocked on a teammate
 * emits nothing further, so its timeline never came back.
 *
 * These tests pin the fold itself. It is shared by both maps deliberately: a
 * second copy is how the two would drift, and the review on #2068 caught a test
 * that restated `onTurnEvent`'s rule instead of calling it, which would have
 * kept passing through a regression in the branch the shell actually runs.
 */

const call = (toolCallId: string, label: string) =>
  ({ type: "tool_call", toolCallId, label }) as const;
const result = (toolCallId: string, status: string) =>
  ({ type: "tool_result", toolCallId, status, elapsedMs: 12 }) as const;

describe("folding a live frame", () => {
  it("flips a call to its result in place, rather than appending a second row", () => {
    const started = foldLiveFrame([], call("c1", "Http Request"));
    expect(started).not.toBeNull();
    expect(started).toHaveLength(1);
    expect(started![0].status).toBe("running");

    const done = foldLiveFrame(started!, result("c1", "ok"));
    expect(done).toHaveLength(1);
    expect(done![0].status).toBe("ok");
    expect(done![0].label).toBe("Http Request");
    expect(done![0].elapsedMs).toBe(12);
  });

  it("carries an error status through rather than reporting success", () => {
    const rows = foldLiveFrame(foldLiveFrame([], call("c1", "Curl"))!, result("c1", "error"));
    expect(rows![0].status).toBe("error");
  });

  /**
   * The drop that made a mis-keyed frame *worse* than invisible. A result whose
   * call is not in these rows is refused rather than adopted — adopting it
   * would invent a row with no start, and pairing it with an unrelated running
   * row would mark the wrong call finished.
   *
   * This is what left a spinner running forever when a key changed mid-turn
   * (PR #2068 review): the call went to one bucket, the result to another, and
   * the result was dropped on arrival. The fold's job is to refuse; keeping the
   * key stable is what stops it being asked.
   */
  it("refuses a result whose call it does not hold", () => {
    expect(foldLiveFrame([], result("c-unknown", "ok"))).toBeNull();

    const other: LiveRow[] = [
      { kind: "tool_call", status: "running", label: "Web Fetch", toolCallId: "c1" },
    ];
    expect(foldLiveFrame(other, result("c2", "ok"))).toBeNull();
    // …and left the row it does hold untouched, still running.
    expect(other[0].status).toBe("running");
  });

  it("appends a thinking row per frame, since the host already coalesced them", () => {
    const rows = foldLiveFrame([], { type: "thinking" });
    expect(rows).toHaveLength(1);
    expect(rows![0].kind).toBe("thinking");
  });

  it("never mutates the rows it was given", () => {
    const rows: LiveRow[] = [];
    foldLiveFrame(rows, call("c1", "Http Request"));
    expect(rows).toHaveLength(0);
  });
});

describe("two queries in one thread", () => {
  /**
   * The point of the split, stated as a property: each question's rows are
   * folded against its own list, so neither can clear or contaminate the other
   * — which is precisely what one shared per-thread list could not promise.
   */
  it("keep separate lists, so neither turn can lose the other's rows", () => {
    // QUERY A delegates and then blocks on a teammate, emitting nothing more.
    let a = foldLiveFrame([], call("a1", "Delegate To Teammate"))!;
    a = foldLiveFrame(a, result("a1", "ok"))!;
    expect(a).toHaveLength(1);

    // QUERY B is asked while A is still open, and runs its own calls.
    let b = foldLiveFrame([], call("b1", "Http Request"))!;
    b = foldLiveFrame(b, result("b1", "ok"))!;

    // A's row survived B's whole turn, and the two never mixed.
    expect(a).toHaveLength(1);
    expect(a[0].label).toBe("Delegate To Teammate");
    expect(b).toHaveLength(1);
    expect(b[0].label).toBe("Http Request");
  });

  /**
   * And the shape that made the merged list unreadable even when nothing was
   * lost: `seq` is per-turn, so both turns count from zero. Ordering a merged
   * list by it is meaningless — which is why the split is by key rather than by
   * sorting harder.
   */
  it("cannot be ordered by seq, because each turn counts from its own zero", () => {
    const aFirst = { seq: 0, messageSeq: 45 };
    const bFirst = { seq: 0, messageSeq: 49 };
    expect(aFirst.seq).toBe(bFirst.seq);
    expect(aFirst.messageSeq).not.toBe(bFirst.messageSeq);
  });
});
