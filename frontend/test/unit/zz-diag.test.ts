// @vitest-environment jsdom
// Temporary diagnostic: does flushSync (or a macrotask + act) recover the
// stuck error render after a plain reject?

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { beforeEach, expect, it } from "vitest";

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

type Variant = "plain" | "flushSync" | "macroAct";
const VARIANTS: Variant[] = ["plain", "flushSync", "macroAct"];
const ITERATIONS = 25;

async function runVariant(variant: Variant): Promise<number> {
  let misses = 0;
  for (let i = 0; i < ITERATIONS; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const deferred: {
      resolve: (rows: RunArtifactRow[]) => void;
      reject: (err: unknown) => void;
    }[] = [];
    const client = deferredFilesClient(deferred);
    await renderPanel(completedRun("run-1"), client);
    await expandFiles();
    if (deferred.length !== 1) continue;

    await act(async () => {
      deferred[0].reject(new Error("boom"));
    });

    if (variant === "flushSync") {
      try {
        flushSync();
      } catch {
        /* flushSync may throw on re-entrance; ignore */
      }
    } else if (variant === "macroAct") {
      await new Promise((resolve) => setTimeout(resolve, 0));
      await act(async () => {});
    }

    if (
      !container.querySelector('[data-testid="workflow-run-files-error"]')
    ) {
      misses++;
    }
  }
  return misses;
}

it("diag: recovery-strategy miss rates", async () => {
  for (const v of VARIANTS) {
    const misses = await runVariant(v);
    console.log(`variant=${v} misses=${misses}/${ITERATIONS}`);
  }
  expect(true).toBe(true);
});
