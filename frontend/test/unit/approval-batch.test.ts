import { describe, expect, it } from "vitest";

import type { ApprovalSummary, Verdict } from "@/api/types";
import { buildTimelineItems, type TimelineItem } from "@/views/chat/model";

/**
 * One turn asks once (issue #842).
 *
 * A research turn that reached espn.com, bbc.com and theguardian.com parked
 * three approvals and interrupted the conversation three times — three
 * sign-offs, and three re-dispatch cycles (#561) for one piece of work. The
 * host already knew they belonged together: #469 records the parking turn so it
 * can continue that turn exactly once. `batch` carries that key out to the
 * console, and this is where it is turned into one card.
 *
 * What must **not** happen is the reason this suite is worth its length. The
 * grouping is presentation over records that stay individually decidable, and
 * two mistakes would each be invisible in a screenshot:
 *
 *  * folding approvals the host did *not* say belonged together — an operator
 *    approving something they were never shown;
 *  * a card that keeps claiming three things are pending after one was decided
 *    on the Approvals page, which is the two surfaces drifting.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function approval(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "id">): ApprovalSummary {
  return {
    kind: "web_fetch",
    amount_usd: null,
    at_millis: T0,
    agent: "seo",
    thread: "desk-marketing",
    ...over,
  };
}

/** Every approval card in the timeline, in the order it renders. */
function cards(items: TimelineItem[]) {
  return items.filter((i): i is Extract<TimelineItem, { kind: "approval" }> => i.kind === "approval");
}

function decidedMap(entries: [ApprovalSummary, Verdict][]) {
  return Object.fromEntries(entries.map(([approval, verdict]) => [approval.id, { verdict, approval }]));
}

describe("buildTimelineItems — batching a turn's gated calls (#842)", () => {
  it("raises one card carrying every call the same turn parked", () => {
    const espn = approval({ id: "a1", batch: "turn-1", at_millis: T0 });
    const bbc = approval({ id: "a2", batch: "turn-1", at_millis: T0 + 900 });

    const items = cards(buildTimelineItems([], [espn, bbc]));

    expect(items).toHaveLength(1);
    expect(items[0].approvals.map((a) => a.id)).toEqual(["a1", "a2"]);
    // At the moment the turn first asked, so the card sits beside the message
    // that provoked it rather than beside whichever call was gated last.
    expect(items[0].at).toBe(T0);
  });

  it("keeps two turns apart, however close together they parked", () => {
    // The failure this guards is not hypothetical: grouping by "same agent,
    // same thread, close together" — the shape a console would reach for
    // without a host key — folds exactly this pair, and the operator approves a
    // second turn's work with one click meant for the first.
    const first = approval({ id: "a1", batch: "turn-1" });
    const second = approval({ id: "a2", batch: "turn-2", at_millis: T0 + 1 });

    const items = cards(buildTimelineItems([], [first, second]));

    expect(items).toHaveLength(2);
    expect(items.map((i) => i.approvals.map((a) => a.id))).toEqual([["a1"], ["a2"]]);
  });

  it("never groups approvals the host gave no batch for", () => {
    // A workflow node, a scheduler tick, an older host: `batch` is absent, and
    // absent means "unknown", not "the same one". Two unknowns are not a batch.
    const workflow = approval({ id: "a1", batch: null, thread: null });
    const tick = approval({ id: "a2", thread: null });

    const items = cards(buildTimelineItems([], [workflow, tick]));

    expect(items).toHaveLength(2);
    expect(items.every((i) => i.approvals.length === 1)).toBe(true);
  });

  it("reports a batch's verdicts per item, so a partly-decided card can say so", () => {
    const espn = approval({ id: "a1", batch: "turn-1" });
    const bbc = approval({ id: "a2", batch: "turn-1", at_millis: T0 + 1 });
    const guardian = approval({ id: "a3", batch: "turn-1", at_millis: T0 + 2 });

    // What the shell holds after one row was approved on the Approvals page:
    // the host has dropped it from the feed, and the witnessed map is the only
    // thing that still knows it existed.
    const items = cards(
      buildTimelineItems([], [bbc, guardian, espn], decidedMap([[espn, "approve"]])),
    );

    expect(items).toHaveLength(1);
    // The settled item rejoins the card it was raised in rather than opening a
    // second one below it — and the batch is still ordered by when each call
    // was gated, not by the order the feed happened to hand them over.
    expect(items[0].approvals.map((a) => a.id)).toEqual(["a1", "a2", "a3"]);
    expect(items[0].decided).toEqual({ a1: "approve" });
  });

  it("carries a mixed batch's verdicts without collapsing them into one", () => {
    const espn = approval({ id: "a1", batch: "turn-1" });
    const bbc = approval({ id: "a2", batch: "turn-1", at_millis: T0 + 1 });

    const items = cards(
      buildTimelineItems([], [espn, bbc], decidedMap([[espn, "approve"], [bbc, "deny"]])),
    );

    expect(items[0].decided).toEqual({ a1: "approve", a2: "deny" });
  });

  it("keeps a fully settled turn as one transcript entry", () => {
    const espn = approval({ id: "a1", batch: "turn-1" });
    const bbc = approval({ id: "a2", batch: "turn-1", at_millis: T0 + 1 });
    const guardian = approval({ id: "a3", batch: "turn-1", at_millis: T0 + 2 });

    const items = cards(
      buildTimelineItems(
        [],
        [espn, bbc, guardian],
        decidedMap([
          [espn, "approve"],
          [bbc, "approve"],
          [guardian, "approve"],
        ]),
      ),
    );

    expect(items).toHaveLength(1);
    expect(items[0].approvals).toHaveLength(3);
    expect(items[0].decided).toEqual({ a1: "approve", a2: "approve", a3: "approve" });
  });
});
