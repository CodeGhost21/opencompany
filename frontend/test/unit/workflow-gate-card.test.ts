import { describe, expect, it } from "vitest";

import { approvalAction, payloadLines } from "@/lib/language";
import { awaitingCount, runTone } from "@/views/workflows/run-health";
import type { ApprovalSummary } from "@/api/types";
import type { WorkflowRunOutcome } from "@/api/workflows";

/**
 * Issue #846, the two console-side halves.
 *
 * **The card** said "Continue a paused workflow" and showed the engine's resume
 * payload — a label true of every one of these cards, over a description that
 * describes nothing. #372 made exactly this complaint about the chat surface and
 * #375 fixed it there; this is the same treatment for the workflow-originated
 * card, now that the host names the call on it.
 *
 * **The run** said "Finished — this run routed no reports" while its gate sat
 * undecided on the Approvals page. A run waiting on a person is not finished,
 * and a scheduled run that reports success while doing none of its work is the
 * failure that makes it matter.
 */

function approval(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "kind">): ApprovalSummary {
  return { id: "a1", amount_usd: null, at_millis: 1_000, agent: null, ...over };
}

/** A paused gate's card as the host now parks it. */
function gateCard(payload: Record<string, unknown>): ApprovalSummary {
  return approval({ kind: "workflow.approve", payload });
}

/** The card the reproduction in #846 actually produced. */
const AS_REPORTED = {
  workflow_id: "daily-sports-news-blog",
  node_id: "fetch_bbc",
  input: { items: [{ json: {} }], port: null },
};

/** The same gate, as the host describes it after this change. */
const DESCRIBED = {
  ...AS_REPORTED,
  tool: "web_fetch",
  args: { url: "https://www.bbc.com/sport" },
  target: "www.bbc.com",
  delivered: [],
  performed: [],
  note: "Approving this re-runs the whole workflow from the start…",
};

function run(over: Partial<WorkflowRunOutcome> = {}): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_000,
    scheduled: false,
    running: false,
    cancelled: false,
    error: null,
    deliveries: [],
    pendingApprovals: [],
    nodes: [],
    ...over,
  } as WorkflowRunOutcome;
}

describe("a paused workflow gate says what it is approving (#846)", () => {
  it("names the tool rather than the mechanism", () => {
    expect(approvalAction(gateCard(DESCRIBED))).toBe("Fetch a web page");
  });

  it("shows the call's arguments and its destination, not the resume payload", () => {
    const lines = payloadLines(gateCard(DESCRIBED));
    const byLabel = Object.fromEntries(lines.map((l) => [l.label, l.value]));

    // What the operator is deciding about — the complaint in the issue was that
    // neither the tool nor the host appeared anywhere on this card.
    expect(byLabel.url).toBe("https://www.bbc.com/sport");
    expect(byLabel.target).toBe("www.bbc.com");
    // Where to find it afterwards.
    expect(byLabel.node_id).toBe("fetch_bbc");
    expect(byLabel.workflow_id).toBe("daily-sports-news-blog");

    // The arguments lead. An operator scanning a clamped block must see the
    // call before the bookkeeping.
    expect(lines[0].label).toBe("url");
  });

  it("hides the engine's resume payload and the mechanism's ledgers", () => {
    const labels = payloadLines(gateCard(DESCRIBED)).map((l) => l.label);

    // `input` is the seed payload that READ as a description and was not one.
    // It also carries the accumulating `approvals` list, which an operator
    // reasonably reads as one card covering several gates — it is not, and
    // consolidating them is #842's job, not something to imply by accident.
    expect(labels).not.toContain("input");
    expect(labels).not.toContain("delivered");
    expect(labels).not.toContain("performed");
    expect(labels).not.toContain("note");
    // Unwrapped one level up, so it must not also appear as a JSON blob.
    expect(labels).not.toContain("args");
  });

  it("keeps a key a newer host adds that this console has never heard of", () => {
    const labels = payloadLines(gateCard({ ...DESCRIBED, whodunnit: "a future field" })).map(
      (l) => l.label,
    );
    expect(labels).toContain("whodunnit");
  });

  it("falls back to the old line for an older host that names no tool", () => {
    // Absence is meaningful: a host from before this change omits `tool`, and
    // the card must read exactly as it did rather than showing a blank label.
    expect(approvalAction(gateCard(AS_REPORTED))).toBe("Continue a paused workflow");
  });

  it("leaves every other kind's card alone", () => {
    const shell = approval({
      kind: "shell",
      agent: "ceo",
      payload: { command: "ls -la", cwd: "/tmp" },
    });
    expect(approvalAction(shell)).toBe("Run a terminal command");
    expect(payloadLines(shell).map((l) => l.label)).toEqual(["command", "cwd"]);
  });
});

describe("a run waiting on a person is not finished (#846)", () => {
  it("counts a parked gate as awaiting, not as a clean run", () => {
    const parked = run({ pendingApprovals: ["fetch_bbc"] });

    // The reproduction's exact shape: no error, not cancelled, not running,
    // and no deliveries — because it never reached an output node. Every one of
    // those reads as "fine" on its own, which is how it scored green.
    expect(awaitingCount(parked)).toBe(1);
    expect(runTone(parked).label).toBe("awaiting approval");
  });

  it("still reads as ok when nothing is waiting", () => {
    expect(runTone(run()).label).toBe("ok");
    expect(awaitingCount(run())).toBe(0);
  });

  it("counts parked gates and parked reports together", () => {
    const both = run({
      pendingApprovals: ["gate_a", "gate_b"],
      deliveries: [
        {
          node: "summary",
          kind: "owner",
          status: "pending",
          detail: "waiting",
          target: null,
          reason: null,
        },
      ],
    } as Partial<WorkflowRunOutcome>);
    expect(awaitingCount(both)).toBe(3);
  });

  it("does not let a still-running run be read as awaiting", () => {
    // `runTone` checks in-flight first, and it must keep doing so: a run that
    // has not finished has neither succeeded nor stopped for a human.
    expect(runTone(run({ running: true, pendingApprovals: ["gate"] })).label).toBe("running");
  });

  it("keeps a failure louder than a pending approval", () => {
    expect(runTone(run({ error: "boom", pendingApprovals: ["gate"] })).label).toBe("failed");
  });
});
