import { describe, expect, it } from "vitest";

import type { WorkflowRunResult } from "@/api/workflows";
import { legacyRunVerdict } from "@/views/workflows/run-health";

/**
 * Codex review (PR #2053): a settled run response from a host predating
 * issue #981 carries no `verdict` key, but still carries enough of the other
 * fields — `cancelled`, `blockedNodes`, `deliveries`, `pendingApprovals`,
 * `nodes` — to say the run was NOT clean. Reading only `res.verdict` mapped
 * every one of those legacy shapes to the green "Workflow ran." fallback,
 * which is the exact contradiction B-039 exists to close, just for an older
 * host. `legacyRunVerdict` is the derivation `WorkflowsView.tsx` now calls
 * when `res.verdict` is absent.
 */

const BASE: WorkflowRunResult = {
  output: null,
  pendingApprovals: [],
};

describe("legacyRunVerdict", () => {
  it("reads a cancelled legacy run as stopped", () => {
    expect(legacyRunVerdict({ ...BASE, cancelled: true })).toBe("stopped");
  });

  it("reads a run with a blocked node as blocked", () => {
    expect(
      legacyRunVerdict({
        ...BASE,
        blockedNodes: [{ nodeId: "approve", tools: ["send_email"] }] as never,
      }),
    ).toBe("blocked");
  });

  it("reads a dropped report as undelivered", () => {
    expect(
      legacyRunVerdict({
        ...BASE,
        deliveries: [{ node: "out", status: "failed" }] as never,
      }),
    ).toBe("undelivered");
  });

  it("reads a run with pending approvals as awaiting-approval", () => {
    expect(legacyRunVerdict({ ...BASE, pendingApprovals: ["gate-1"] })).toBe(
      "awaiting-approval",
    );
  });

  it("reads a run with a pending delivery as awaiting-approval too", () => {
    expect(
      legacyRunVerdict({
        ...BASE,
        deliveries: [{ node: "out", status: "pending" }] as never,
      }),
    ).toBe("awaiting-approval");
  });

  it("reads an errored node, with nothing else wrong, as degraded", () => {
    expect(
      legacyRunVerdict({
        ...BASE,
        nodes: [{ nodeId: "step", status: "error" }] as never,
      }),
    ).toBe("degraded");
  });

  it("reads a run with none of the above as ok", () => {
    expect(legacyRunVerdict(BASE)).toBe("ok");
  });

  it("stopped outranks every other reading, the same precedence verdictOf uses", () => {
    expect(
      legacyRunVerdict({
        ...BASE,
        cancelled: true,
        blockedNodes: [{ nodeId: "x", tools: ["send_email"] }] as never,
        pendingApprovals: ["gate-1"],
      }),
    ).toBe("stopped");
  });
});
