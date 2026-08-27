import { describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import {
  approvalsForTask,
  blockingTaskApprovals,
  decidingForTask,
  taskApprovalBlock,
  taskApprovalRows,
  taskApprovalVerdicts,
} from "@/lib/task-approvals";
import type { Verdict } from "@/api/types";

/**
 * What a paused card is blocked on (issue #883).
 *
 * The board reads `…/tasks`, whose card projection carries no approvals, so
 * before this a paused card could only show a Resume button and no reason —
 * and Resume is the wrong click from that state: the turn continues on its own
 * when the last decision it parked lands (#469), so re-dispatching re-runs the
 * work and parks the same calls again.
 *
 * These pin the join the card is derived from. Each case below is a way the
 * card could look completely normal while saying something false, which is why
 * they are here rather than left to the rendered board.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function approval(
  id: string,
  at: number,
  task: ApprovalSummary["task"],
): ApprovalSummary {
  return {
    id,
    kind: "web_fetch",
    amount_usd: null,
    at_millis: at,
    agent: "seo",
    task,
    payload: { url: `https://example.com/${id}` },
  };
}

const MINE = (id: string, at: number) => approval(id, at, { link: "task", id: "task-1" });
const THEIRS = (id: string, at: number) => approval(id, at, { link: "task", id: "task-2" });

describe("approvalsForTask", () => {
  it("takes only the approvals whose park named this card", () => {
    const feed = [MINE("a1", T0), THEIRS("b1", T0), MINE("a2", T0 + 1_000)];
    expect(approvalsForTask(feed, "task-1").map((a) => a.id)).toEqual(["a1", "a2"]);
  });

  /**
   * `{link: "unlinked"}` is a park the runtime performed for no card — a
   * workflow delivery, an operator-chat turn, a scheduler tick. Counting one
   * would put "blocked on 1 approval" on a card that is not blocked at all, and
   * then disable its Resume button forever, since deciding that approval would
   * never be something the operator connects to this card.
   */
  it("ignores an approval that belongs to no card", () => {
    const feed = [approval("u1", T0, { link: "unlinked" }), MINE("a1", T0)];
    expect(approvalsForTask(feed, "task-1").map((a) => a.id)).toEqual(["a1"]);
  });

  /**
   * An absent link is a park written before #333 stamped one. The host keeps a
   * run-window heuristic for exactly this case; the board has no window to
   * apply it against, so it must skip rather than guess — attributing an
   * unrecorded park to whichever card happened to be read would block a card
   * for a reason that is not its own.
   */
  it("ignores an approval whose park recorded no link at all", () => {
    const feed = [approval("old", T0, undefined), MINE("a1", T0)];
    expect(approvalsForTask(feed, "task-1").map((a) => a.id)).toEqual(["a1"]);
  });
});

describe("taskApprovalBlock", () => {
  it("is null when nothing in the queue names this card", () => {
    expect(taskApprovalBlock([THEIRS("b1", T0)], "task-1")).toBeNull();
    expect(taskApprovalBlock([], "task-1")).toBeNull();
  });

  it("counts every approval parked for this card", () => {
    const feed = [MINE("a1", T0), THEIRS("b1", T0), MINE("a2", T0), MINE("a3", T0)];
    expect(taskApprovalBlock(feed, "task-1")?.count).toBe(3);
  });

  /**
   * The oldest park, not the newest — the thing the card reports is how long it
   * has really been stopped. Taking the newest would reset a climbing clock
   * every time a second effect parked behind the first, so a card wedged for an
   * hour would read as freshly blocked and nothing about it would look wrong.
   */
  it("anchors the wait to the oldest park, whatever order the feed is in", () => {
    const feed = [MINE("late", T0 + 600_000), MINE("early", T0), MINE("mid", T0 + 60_000)];
    expect(taskApprovalBlock(feed, "task-1")?.since).toBe(T0);
  });

  /**
   * Oldest first, because the card names the *first* blocked call when there is
   * only one to name and the detail row reads the same list. An order that
   * varied with the host's response order would make the card's sentence change
   * between two polls that carry identical facts.
   */
  it("orders the approvals oldest park first", () => {
    const feed = [MINE("late", T0 + 600_000), MINE("early", T0), MINE("mid", T0 + 60_000)];
    expect(taskApprovalBlock(feed, "task-1")?.approvals.map((a) => a.id)).toEqual([
      "early",
      "mid",
      "late",
    ]);
  });
});

/**
 * The same join, plus what has become of each row (#1891).
 *
 * The card decides now, so it needs more than "is this blocked": it needs the
 * *set* it is deciding, and it needs a row the operator has just settled to
 * read as settled rather than offer its buttons a second time. That gap — the
 * resolve's answer and the feed's next poll are two moments — is what these
 * pin, and it is invisible on a rendered card that is polling every four
 * seconds.
 */
describe("taskApprovalRows", () => {
  it("carries this card's parked approvals, oldest park first, undecided", () => {
    const feed = [MINE("a2", T0 + 1_000), THEIRS("b1", T0), MINE("a1", T0)];
    const rows = taskApprovalRows(feed, {}, "task-1");
    expect(rows.map((r) => r.approval.id)).toEqual(["a1", "a2"]);
    expect(rows.every((r) => r.verdict === null)).toBe(true);
  });

  /** Two parks in the same millisecond still order the same way on every poll:
   *  a list that reordered under a pointer mid-click would be its own bug. */
  it("breaks a tie on id, so the order survives a refresh", () => {
    const rows = taskApprovalRows([MINE("b", T0), MINE("a", T0)], {}, "task-1");
    expect(rows.map((r) => r.approval.id)).toEqual(["a", "b"]);
  });

  /**
   * The witnessed half. The queue has already dropped this row, and without
   * folding it back in the operator's own decision would blink out from under
   * their cursor instead of settling in place.
   */
  it("keeps a decided approval the queue has already dropped", () => {
    const a1 = MINE("a1", T0);
    const rows = taskApprovalRows([], { a1: { verdict: "approve", approval: a1 } }, "task-1");
    expect(rows.map((r) => [r.approval.id, r.verdict])).toEqual([["a1", "approve"]]);
  });

  /** `decided` is console-wide. Another card's settled row is not this card's
   *  business, and folding it in would inflate this card's total. */
  it("ignores a decision taken on another card's approval", () => {
    const b1 = THEIRS("b1", T0);
    const rows = taskApprovalRows([], { b1: { verdict: "deny", approval: b1 } }, "task-1");
    expect(rows).toEqual([]);
  });

  it("marks a still-queued row with the verdict this console witnessed", () => {
    const a1 = MINE("a1", T0);
    const rows = taskApprovalRows([a1], { a1: { verdict: "deny", approval: a1 } }, "task-1");
    expect(rows[0].verdict).toBe("deny");
    expect(blockingTaskApprovals(rows)).toEqual([]);
  });
});

describe("taskApprovalVerdicts", () => {
  it("maps only the rows that have one", () => {
    const a1 = MINE("a1", T0);
    const rows = taskApprovalRows(
      [a1, MINE("a2", T0 + 1_000)],
      { a1: { verdict: "approve", approval: a1 } },
      "task-1",
    );
    expect(taskApprovalVerdicts(rows)).toEqual({ a1: "approve" });
  });
});

describe("decidingForTask", () => {
  /**
   * The shell's map covers every surface that resolves. `ApprovalRow` reads
   * `size > 0` as "I am busy", so handing it through whole would grey out every
   * blocked card on the board the moment one of them was clicked (#373, one
   * surface over).
   */
  it("keeps only the in-flight decisions belonging to this card", () => {
    const rows = taskApprovalRows([MINE("a1", T0), MINE("a2", T0 + 1_000)], {}, "task-1");
    const deciding = new Map<string, Verdict>([
      ["a1", "approve"],
      ["b1", "deny"],
    ]);
    expect([...decidingForTask(rows, deciding)]).toEqual([["a1", "approve"]]);
  });

  it("is empty when the console is deciding nothing of this card's", () => {
    const rows = taskApprovalRows([MINE("a1", T0)], {}, "task-1");
    expect(decidingForTask(rows, new Map([["b1", "deny"]])).size).toBe(0);
  });
});
