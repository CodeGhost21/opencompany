import { describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import { approvalBatchKey, buildTimelineItems } from "@/views/chat/model";

/**
 * A blocker folds by its root cause (#1862): every card stalled on one broken
 * integration is one question, even across turns a batch would keep apart.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function blocker(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "id">): ApprovalSummary {
  return {
    kind: "blocker.infrastructure",
    amount_usd: null,
    at_millis: T0,
    agent: "eng",
    thread: "eng",
    ...over,
  };
}

describe("approvalBatchKey for blockers", () => {
  it("folds blockers sharing a group_key into one key, across different batches", () => {
    const a = blocker({ id: "a", batch: "turn-1", group_key: "connection:slack" });
    const b = blocker({ id: "b", batch: "turn-2", group_key: "connection:slack" });
    expect(approvalBatchKey(a)).toBe(approvalBatchKey(b));
  });

  it("keeps distinct connections apart", () => {
    const slack = blocker({ id: "a", group_key: "connection:slack" });
    const notion = blocker({ id: "b", group_key: "connection:notion" });
    expect(approvalBatchKey(slack)).not.toBe(approvalBatchKey(notion));
  });

  it("groups an ungrouped blocker alone", () => {
    const a = blocker({ id: "a" });
    const b = blocker({ id: "b" });
    expect(approvalBatchKey(a)).not.toBe(approvalBatchKey(b));
  });

  it("leaves an ordinary approval on its batch", () => {
    const a: ApprovalSummary = { ...blocker({ id: "a" }), kind: "web_fetch", batch: "turn-9" };
    expect(approvalBatchKey(a)).toBe("turn-9");
  });

  it("folds an ungrouped blocker onto an ordinary approval sharing its batch", () => {
    const call: ApprovalSummary = { ...blocker({ id: "a" }), kind: "web_fetch", batch: "turn-9" };
    const stuck = blocker({ id: "b", batch: "turn-9" });
    expect(approvalBatchKey(stuck)).toBe(approvalBatchKey(call));
  });
});

describe("buildTimelineItems groups connection blockers into one card", () => {
  it("renders one card for three parks on one broken connection", () => {
    const items = buildTimelineItems(
      [],
      [
        blocker({ id: "a", group_key: "connection:slack" }),
        blocker({ id: "b", group_key: "connection:slack" }),
        blocker({ id: "c", group_key: "connection:slack" }),
      ],
    );
    const cards = items.filter((i) => i.kind === "approval");
    expect(cards).toHaveLength(1);
    expect(cards[0].kind === "approval" && cards[0].approvals).toHaveLength(3);
  });

  it("mixes an ungrouped blocker into an ordinary call's card when they share a batch", () => {
    const call: ApprovalSummary = { ...blocker({ id: "a" }), kind: "web_fetch", batch: "turn-9" };
    const stuck = blocker({ id: "b", batch: "turn-9" });
    const items = buildTimelineItems([], [call, stuck]);

    const cards = items.filter((i) => i.kind === "approval");
    expect(cards).toHaveLength(1);
    expect(cards[0].kind === "approval" && cards[0].approvals.map((a) => a.id)).toEqual([
      "a",
      "b",
    ]);
  });
});
