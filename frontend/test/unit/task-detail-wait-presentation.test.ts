import { describe, expect, it } from "vitest";

import type { TimelineEntry } from "@/api/tasks";
import { groupTimeline } from "@/views/TaskDetailView";

function entry(over: Partial<TimelineEntry> = {}): TimelineEntry {
  return {
    seq: 1,
    atMillis: 1_000,
    kind: "approval",
    label: "Approved launch copy",
    ...over,
  };
}

describe("task-detail wait presentation", () => {
  it("keeps only completed approval waits in the timeline", () => {
    expect(
      groupTimeline([
        entry({ seq: 4, waitedMillis: 8_000 }),
        entry({ seq: 5, kind: "dispatched", label: "Resumed work" }),
      ]),
    ).toMatchObject([
      { row: "wait", key: "wait-4", millis: 8_000 },
      { row: "group", key: "4" },
      { row: "group", key: "5" },
    ]);
  });
});
