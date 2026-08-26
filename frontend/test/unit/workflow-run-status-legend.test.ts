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
