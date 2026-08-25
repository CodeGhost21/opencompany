// @vitest-environment jsdom
// Temporary diagnostic: the full error-then-retry cycle with a client that
// rejects on call 1 and succeeds on call 2 (both promises already settled
// when the component attaches handlers — the workflow-create pattern).

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

function completedRun(runId: string): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_000,
    workflowId: "launch",
    scheduled: false,
    runId,
    deliveries: [],
    pendingApprovals: [],
  };
}

async function renderPanel(run: WorkflowRunOutcome, client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(RunHistoryPanel, {
        client,
        company: "acme",
        runs: [run],
        graph: null,
        workflowName: "Launch",
        onClose: () => {},
        selectedRunSeq: null,
        onSelectRun: () => {},
      }),
    );
  });
}

async function expandFiles() {
  const details = container.querySelector<HTMLDetailsElement>(
    '[data-testid="workflow-run-files"]',
  );
  await act(async () => {
    details!.open = true;
    details!.dispatchEvent(new Event("toggle", { bubbles: true }));
  });
  await act(async () => {});
}

async function collapseFiles() {
  const details = container.querySelector<HTMLDetailsElement>(
    '[data-testid="workflow-run-files"]',
  );
  await act(async () => {
    details!.open = false;
    details!.dispatchEvent(new Event("toggle", { bubbles: true }));
  });
}

function failThenSucceedClient(
  rows: RunArtifactRow[],
  sink: { calls: string[] },
): OpenCompanyClient {
  let calls = 0;
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      sink.calls.push(path);
      calls++;
      if (calls === 1) return Promise.reject(new Error("boom")) as never;
      return { files: rows, truncated: false } as T;
    },
  } as unknown as OpenCompanyClient;
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

it("diag: fail-then-succeed two-phase cycle reliability", async () => {
  const FILE: RunArtifactRow = {
    taskId: "t-a",
    artifactId: "art-a1",
    title: "Retried launch spec",
    kind: "markdown",
    source: "specs/launch.md",
    latestVersion: 2,
    updatedAtMillis: 30,
    taskTitle: "Draft the launch",
  };

  let misses = 0;
  for (let i = 0; i < 30; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const sink = { calls: [] as string[] };
    const client = failThenSucceedClient([FILE], sink);

    await renderPanel(completedRun("run-1"), client);
    await expandFiles();

    // Phase 1: error renders, latch reset.
    const err = container.querySelector(
      '[data-testid="workflow-run-files-error"]',
    );
    if (!err || sink.calls.length !== 1) {
      misses++;
      console.log(`iter ${i} phase1: MISS. calls=${sink.calls.length}`);
      continue;
    }

    await collapseFiles();
    await expandFiles();

    // Phase 2: retried fetch succeeds, files render, error gone.
    const entries = container.querySelectorAll(
      '[data-testid="workflow-run-file"]',
    );
    const errAfter = container.querySelector(
      '[data-testid="workflow-run-files-error"]',
    );
    if (sink.calls.length !== 2 || entries.length !== 1 || errAfter) {
      misses++;
      console.log(
        `iter ${i} phase2: MISS. calls=${sink.calls.length} entries=${entries.length} err=${!!errAfter}`,
      );
    }
  }
  console.log(`misses=${misses}/30`);
  expect(misses).toBe(0);
});
