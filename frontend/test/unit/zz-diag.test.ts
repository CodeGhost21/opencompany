// @vitest-environment jsdom
// Temporary diagnostic: why does the error element not render after reject?

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

function deferredFilesClient(
  deferred: {
    resolve: (rows: RunArtifactRow[]) => void;
    reject: (err: unknown) => void;
  }[],
): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      return new Promise<T>((resolve, reject) => {
        deferred.push({ resolve, reject });
        void path;
      });
    },
  } as unknown as OpenCompanyClient;
}

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

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

it("diag: dump DOM after reject, with delays", async () => {
  const deferred: {
    resolve: (rows: RunArtifactRow[]) => void;
    reject: (err: unknown) => void;
  }[] = [];
  const client = deferredFilesClient(deferred);
  await renderPanel(completedRun("run-1"), client);
  await expandFiles();
  expect(deferred.length).toBe(1);

  await act(async () => {
    deferred[0].reject(new Error("boom"));
  });

  const dump = () =>
    container.querySelector('[data-testid="workflow-run-files"]')?.innerHTML;
  console.log("after reject act:", JSON.stringify(dump()));

  await act(async () => {});
  console.log("after 2nd empty act:", JSON.stringify(dump()));

  await new Promise((r) => setTimeout(r, 50));
  console.log("after 50ms macrotask:", JSON.stringify(dump()));

  await act(async () => {});
  console.log("after 3rd empty act:", JSON.stringify(dump()));

  expect(true).toBe(true);
});
