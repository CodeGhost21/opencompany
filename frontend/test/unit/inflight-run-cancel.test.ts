// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { InflightRun } from "@/api/tasks";

/**
 * Cancelling a run that has no card.
 *
 * A `kind: "delegation"` run carries `taskId: null`, and `POST …/tasks/{key}/steer`
 * resolves `key` against the in-flight registry. These tests pin the control to
 * that key: addressing a run by `taskId` cannot reach a delegation at all.
 */

const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), toasts),
}));

const steerTask = vi.hoisted(() => vi.fn());

vi.mock("@/api/tasks", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/tasks")>()),
  steerTask,
}));

const { InflightRunBar } = await import("@/views/chat/InflightRunBar");

const CLIENT = {
  scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
} as unknown as OpenCompanyClient;

/** A delegation: running, and with no card behind it. */
function delegation(over: Partial<InflightRun> = {}): InflightRun {
  return {
    taskId: null,
    key: "run_7f3c9a",
    kind: "delegation",
    title: "Research the competitor pricing",
    agentId: "analyst",
    startedAt: 1_700_000_000_000,
    pendingAction: null,
    ...over,
  };
}

function task(over: Partial<InflightRun> = {}): InflightRun {
  return { ...delegation(), taskId: "t-42", key: "t-42", kind: "task", ...over };
}

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  steerTask.mockResolvedValue(undefined);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

function render(runs: readonly InflightRun[], onSteered = vi.fn()) {
  act(() => {
    root.render(
      createElement(InflightRunBar, {
        client: CLIENT,
        company: "acme",
        runs,
        onSteered,
      }),
    );
  });
  return onSteered;
}

/** The cancel control for a run, found by its accessible name. */
function cancelButton(title: string): HTMLButtonElement | null {
  return host.querySelector<HTMLButtonElement>(
    `button[aria-label="Cancel ${title}"]`,
  );
}

function click(el: HTMLElement) {
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Let the click's async handler settle. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("a delegation with no card can be cancelled", () => {
  beforeEach(() => vi.stubGlobal("confirm", vi.fn(() => true)));

  it("offers Cancel for a run whose taskId is null", () => {
    render([delegation()]);
    expect(cancelButton("Research the competitor pricing")).not.toBeNull();
  });

  it("steers by the run key, never by taskId", async () => {
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();

    expect(steerTask).toHaveBeenCalledTimes(1);
    const [, company, key, body] = steerTask.mock.calls[0];
    // The whole point: a null taskId cannot address anything, and the key can.
    expect(key).toBe("run_7f3c9a");
    expect(company).toBe("acme");
    expect(body).toEqual({ action: "cancel", confirm: true });
  });

  it("sends confirm, which the host requires for a cancel", async () => {
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();
    expect(steerTask.mock.calls[0][3]).toMatchObject({ confirm: true });
  });

  it("re-reads the in-flight list so the cancelled run leaves the bar", async () => {
    const onSteered = render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();
    expect(onSteered).toHaveBeenCalled();
  });

  it("renders nothing when nothing is in flight", () => {
    render([]);
    expect(host.querySelector("[data-testid='inflight-run-bar']")).toBeNull();
  });

  it("gives every run of many its own control", () => {
    render([
      delegation(),
      delegation({ key: "run_b", title: "Draft the brief" }),
      task({ title: "Ship the changelog" }),
    ]);
    expect(host.querySelectorAll("[data-testid='inflight-run-row']")).toHaveLength(3);
    expect(cancelButton("Draft the brief")).not.toBeNull();
    expect(cancelButton("Ship the changelog")).not.toBeNull();
  });

  it("cancels one of several without touching the others", async () => {
    render([delegation(), delegation({ key: "run_b", title: "Draft the brief" })]);
    click(cancelButton("Draft the brief")!);
    await settle();
    expect(steerTask).toHaveBeenCalledTimes(1);
    expect(steerTask.mock.calls[0][2]).toBe("run_b");
  });
});

describe("the operator is asked before a run is stopped", () => {
  it("does not steer when the confirmation is declined", async () => {
    vi.stubGlobal("confirm", vi.fn(() => false));
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();
    expect(steerTask).not.toHaveBeenCalled();
  });
});

describe("a run that is already going away", () => {
  beforeEach(() => vi.stubGlobal("confirm", vi.fn(() => true)));

  it("reads a host 404 as the run having finished, not as a failure", async () => {
    // The run settled between the render and the click: its key is no longer
    // in the registry. Telling the operator the cancel failed would be false —
    // the work they wanted stopped is over.
    steerTask.mockRejectedValue(new ApiError(404, "not_found", "task run_7f3c9a", true));
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();

    expect(toasts.info).toHaveBeenCalled();
    expect(toasts.error).not.toHaveBeenCalled();
  });

  it("reads a host 409 the same way", async () => {
    steerTask.mockRejectedValue(
      new ApiError(409, "conflict", "task t-42 is not in flight", true),
    );
    render([task()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();

    expect(toasts.info).toHaveBeenCalled();
    expect(toasts.error).not.toHaveBeenCalled();
  });

  it("still refreshes after a rejected steer, so a stale row is cleared", async () => {
    steerTask.mockRejectedValue(new ApiError(404, "not_found", "gone", true));
    const onSteered = render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();
    expect(onSteered).toHaveBeenCalled();
  });

  it("surfaces a real failure as an error", async () => {
    steerTask.mockRejectedValue(new ApiError(500, "internal", "the host fell over", true));
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();

    expect(toasts.error).toHaveBeenCalled();
    expect(toasts.info).not.toHaveBeenCalled();
  });

  it("does not claim a run finished when it was a proxy that 404'd", async () => {
    // `fromHost: false` means nothing between here and the registry answered
    // about this run. Reporting "already finished" would be an invention.
    steerTask.mockRejectedValue(new ApiError(404, "http_error", "HTTP 404", false));
    render([delegation()]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();

    expect(toasts.error).toHaveBeenCalled();
    expect(toasts.info).not.toHaveBeenCalled();
  });

  it("freezes a row whose steer the host is already applying", () => {
    render([delegation({ pendingAction: "cancel" })]);
    expect(cancelButton("Research the competitor pricing")).toBeNull();
    expect(host.textContent).toContain("cancelling…");
  });
});

describe("a delegation that gains a card mid-run stays cancellable", () => {
  beforeEach(() => vi.stubGlobal("confirm", vi.fn(() => true)));

  it("addresses the same key after a taskId appears", async () => {
    // The hand-off path opens a card while the delegated turn is already
    // registered. The key is minted once and does not move, so a row that
    // acquires a taskId under the operator still cancels the same run.
    render([delegation({ taskId: "t-99" })]);
    click(cancelButton("Research the competitor pricing")!);
    await settle();
    expect(steerTask.mock.calls[0][2]).toBe("run_7f3c9a");
  });
});
