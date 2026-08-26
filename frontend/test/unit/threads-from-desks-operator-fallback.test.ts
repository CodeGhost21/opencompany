// Codex review finding on #1757 (PR #1781): `threadsFromDesks` (`@/lib/threads`)
// backs the still-supported `#/conversation` route, and used to fall back to
// `defaultThreads()` only when the host's `/desks` answer was literally `[]`.
// The always-present Operator system channel (issue #1757) means a desk-less
// company's `/desks` answer is never actually empty — so that fallback never
// fired for a desk-less company, and `#/conversation` lost its
// main/strategy/creative/front-desk lines (and anything already journaled
// under them), while the Chat route's `resolveDesks` (`views/chat/model.ts`)
// stayed correct because issue #370 had already keyed *its* fallback on
// non-system desk count. This is the second time this route has lagged the
// Chat view's desk handling — `782772aad` had to bring `readOnly` here after
// the enumeration sweep missed it — so these pin the fallback threshold
// itself, not just the read-only flag `conversation-operator-readonly.test.ts`
// already covers.

import { describe, expect, it } from "vitest";

import type { DeskDto } from "@/api/types";
import { defaultThreads, threadsFromDesks } from "@/lib/threads";

function operatorDesk(): DeskDto {
  return {
    id: "operator",
    name: "Operator",
    description: "Workflow reports and notifications",
    members: [],
    system: true,
  };
}

function realDesk(id: string, name: string): DeskDto {
  return { id, name, members: [] };
}

describe("threadsFromDesks operator-only fallback (issue #1757, PR #1781)", () => {
  it("falls back to the default threads when /desks answers with only the Operator system channel", () => {
    const threads = threadsFromDesks([operatorDesk()]);
    const ids = threads.map((t) => t.id);

    // Every default thread survives the fallback — this is the exact
    // regression: pre-fix, a desk-less company's non-empty (Operator-only)
    // `/desks` response skipped `defaultThreads()` entirely.
    expect(ids).toEqual([...defaultThreads().map((t) => t.id), "operator"]);
  });

  it("still marks the fallback's Operator thread read-only", () => {
    const threads = threadsFromDesks([operatorDesk()]);
    const operator = threads.find((t) => t.id === "operator");
    expect(operator?.readOnly).toBe(true);
  });

  it("still falls back on a literally empty /desks answer", () => {
    const threads = threadsFromDesks([]);
    expect(threads.map((t) => t.id)).toEqual(defaultThreads().map((t) => t.id));
  });

  it("uses the real desks, not the fallback, once the company has any", () => {
    const threads = threadsFromDesks([realDesk("growth", "Growth"), operatorDesk()]);
    const ids = threads.map((t) => t.id);

    expect(ids).toEqual(["main", "growth", "operator"]);
    expect(threads.find((t) => t.id === "growth")?.readOnly).toBeFalsy();
    expect(threads.find((t) => t.id === "operator")?.readOnly).toBe(true);
  });
});
