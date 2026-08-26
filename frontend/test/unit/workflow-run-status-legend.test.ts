// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

/**
 * Issue #1798: a run row's at-a-glance signals — the coloured status dot, the
 * "not delivered" badge — carried no definition, so an operator could not act on
 * a status they could not read. Each now explains itself on hover, and the panel
 * header carries a standing legend for the operator who does not know a badge is
 * hoverable.
 *
 * A jsdom render because the claim is about what the panel PAINTS: a `title`
 * attribute on the right element and a legend affordance in the header. The
 * definitions themselves are a plain map and could be unit-tested directly, but
 * the wiring — which element carries which title — is the part that regresses.
 */

function baseRun(over: Partial<WorkflowRunOutcome> = {}): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_700_000_000_000,
    workflowId: "daily_digest",
    scheduled: false,
    deliveries: [],
    pendingApprovals: [],
    ...over,
  };
}

/** A run an operator stopped: `runTone` reads this as `stopped`. */
function stoppedRun(): WorkflowRunOutcome {
  return baseRun({ seq: 2, cancelled: true });
}

/** A run that failed before any node ran at all — a graph that would not
 * compile, or a capability that could not be built (`graph.ts`'s
 * `failureLocation`/`failedNodeOf` name this case explicitly: `nodes` is
 * empty and no node is at fault). `verdictOf` reads a run with `error` set as
 * `failed` regardless of whether any node ran. */
function failedRun(): WorkflowRunOutcome {
  return baseRun({ seq: 4, error: "graph failed to compile" });
}

/** A run blocked on a gated call that never got a card at all: `unparkable` is
 * set and `approvalIds` is absent, the shape `WorkflowBlockedNode.approvalIds`
 * documents as "Absent when every park failed." `isBlocked` reads `true` off
 * `blockedNodes.length`, and nothing here promotes the run to `stranded`
 * (that reading only folds `pendingApprovals`/`strandedApprovals` — the
 * gate shape, not this one — see `run-verdict.md`). */
function blockedUnparkableRun(): WorkflowRunOutcome {
  return baseRun({
    seq: 5,
    blockedNodes: [{ nodeId: "notify_ops", tools: ["send_slack_message"], unparkable: 1 }],
  });
}

/** A finished run whose one output report was refused — `undeliveredCount` is 1,
 * so the row falls to the delivery block and badges "1 not delivered". */
function undeliveredRun(): WorkflowRunOutcome {
  return baseRun({
    seq: 3,
    deliveries: [
      {
        node: "digest",
        kind: "channel",
        target: "engineering",
        status: "failed",
        detail: "channel not wired",
      },
    ],
  });
}

/** A dry run: its one delivery is `skipped` with reason `dry-run` —
 * `isUndelivered` (`run-health.ts`) deliberately exempts that reason, so
 * `undeliveredCount` is 0 and `verdictOf` reads this run as `ok`, the same as
 * a run that actually sent something. Nothing was attempted, on purpose. */
function dryRunOkRun(): WorkflowRunOutcome {
  return baseRun({
    seq: 6,
    deliveries: [
      {
        node: "digest",
        kind: "channel",
        target: "engineering",
        status: "skipped",
        reason: "dry-run",
        detail: "dry run — nothing sent",
      },
    ],
  });
}

let container: HTMLDivElement;
let root: Root;

async function renderHistory(run: WorkflowRunOutcome) {
  await act(async () => {
    root.render(
      createElement(RunHistoryPanel, {
        runs: [run],
        graph: null,
        workflowName: "Daily digest",
        onClose: () => {},
        selectedRunSeq: null,
        onSelectRun: () => {},
      }),
    );
  });
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the run-status legend affordance", () => {
  it("renders an accessible legend trigger in the panel header", async () => {
    await renderHistory(baseRun());
    const legend = container.querySelector(
      '[data-testid="workflow-run-legend"]',
    );
    expect(legend).not.toBeNull();
    // It must carry a name — the whole point is a discoverable key, and an icon
    // with no accessible name is not discoverable to a screen reader.
    expect(legend?.getAttribute("aria-label")).toBeTruthy();
  });
});

describe("the status dot defines the run's verdict on hover", () => {
  it("titles a stopped run's dot with the word and its meaning", async () => {
    await renderHistory(stoppedRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    // The word an operator sees the colour for…
    expect(title).toContain("stopped");
    // …and a plain-English definition, not just the word again.
    expect(title).toContain("stopped this run");
  });

  // Codex review on #1821: `RunCancel` (`src/ports/workflow_runner.rs`) stops
  // a run at the next node boundary — the node already executing normally
  // finishes and is journaled. Only a node wedged past the hard-abort grace
  // period is actually dropped mid-flight. The old wording claimed the
  // mid-flight step was *always* dropped, which misleads an operator into
  // thinking its work or side effects never completed.
  it("does not claim the mid-flight step was unconditionally dropped", async () => {
    await renderHistory(stoppedRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).toContain("normally ran to completion");
    expect(title).not.toContain("was dropped where it was");
  });

  // Codex review on #1821: `failureLocation`/`failedNodeOf` (`graph.ts`)
  // explicitly preserve the case where a run's `error` names no node at all —
  // a graph that would not compile, a capability that could not be built, or
  // an interrupted run the boot sweep recorded. The old wording said
  // unconditionally "a step errored", misdiagnosing those runs.
  it("does not claim a step errored when the run failed before any step ran", async () => {
    await renderHistory(failedRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).toContain("failed");
    expect(title).not.toContain("A step errored");
  });

  // Codex review on #1821 (second pass): the previous fix above stopped
  // claiming a step errored when the run failed with no node at fault, but
  // left the remedy — "correct the workflow" — unconditional. `workflow_outcome.rs`'s
  // `INTERRUPTED_BY_RESTART` is exactly this case and is deliberately worded
  // as a host fact, not a workflow fault ("nothing about the graph went
  // wrong, the process holding it went away... an operator reading this
  // should go looking at the deployment, not at their nodes"). Telling that
  // operator to correct the workflow sends them to fix something that was
  // never broken.
  it("does not unconditionally tell the operator to correct the workflow", async () => {
    await renderHistory(failedRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).toContain("failed");
    expect(title).not.toContain("correct the workflow, and run it again");
    // The hedge names a case where the failure isn't the workflow's fault.
    expect(title).toContain("host restart");
  });

  // Codex review on #1821: a blocked node whose gated call could not be
  // queued for approval at all (`unparkable`, not `stranded`) never gets a
  // card in Approvals — `BlockedNodeApprovals`/the row's own body text
  // already say so ("could not be queued for approval at all, so you will
  // not be asked about it"). The old wording unconditionally told the
  // operator to "decide it in Approvals" for every blocked run, which sends
  // this one to a queue with nothing in it.
  it("does not unconditionally promise a card in Approvals for a blocked run", async () => {
    await renderHistory(blockedUnparkableRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).toContain("blocked");
    expect(title).not.toContain("decide it in Approvals");
    // The hedge names the case that has no card, and what to do about it.
    expect(title).toContain("nothing there to decide");
  });

  // Codex review on #1821 (third pass, same site): `unparkable` is set both
  // when the workflow never wired an approvals queue AND when the store
  // itself refused the write (`docs/modules/server/workflow-routes.md`'s
  // `parkFailed`: "the store refused the write, or no approvals queue is
  // wired"). The frontend has no field naming which one happened, so telling
  // the operator this "needs a workflow or policy change" is only true for
  // one of the two causes and misdirects them for the other.
  it("does not unconditionally prescribe a workflow change for a call that could not be queued", async () => {
    await renderHistory(blockedUnparkableRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).not.toContain("that case needs a workflow or policy change");
    // The hedge names the infra cause a workflow edit can't fix.
    expect(title).toContain("approvals queue itself can refuse the write");
  });

  // Codex review on #1821: `isUndelivered` (`run-health.ts`) exempts a
  // `skipped` row whose reason is `dry-run` — a test run attempted nothing,
  // on purpose — so `verdictOf` reads it as `ok` the same as a run that
  // actually sent something. The old wording claimed "every report reached
  // its destination", which is false for a report that was never attempted.
  it("does not claim every report was delivered for a dry run read as ok", async () => {
    await renderHistory(dryRunOkRun());
    const dot = container.querySelector(
      '[data-testid="workflow-run-status-dot"]',
    );
    const title = dot?.getAttribute("title") ?? "";
    expect(title).toContain("ok");
    expect(title).not.toContain("every report reached its destination");
    // The hedge covers the report that was never attempted, not just refused.
    expect(title).toContain("didn't need to");
  });
});

describe("the 'not delivered' delivery badge explains itself", () => {
  it("carries a title defining a report that did not go out", async () => {
    await renderHistory(undeliveredRun());
    const badge = Array.from(container.querySelectorAll("[title]")).find((el) =>
      el.textContent?.includes("not delivered"),
    );
    expect(badge).toBeTruthy();
    expect(badge?.getAttribute("title")).toContain(
      "never reached its destination",
    );
  });
});
