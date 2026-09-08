// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { listInflight, type InflightRun } from "@/api/tasks";
import { InflightRunBar } from "@/views/chat/InflightRunBar";

/**
 * Reading `GET …/tasks/inflight` and reaching the steer controls it feeds.
 *
 * `listInflight` itself has zero prior coverage — `inflight-run-cancel.test.ts`
 * and `inflight-run-surface.test.ts` are thorough on `InflightRunBar`/
 * `InflightRunRow`, but every one of them hands `runs` in as a prop and never
 * calls the actual read. `GET …/tasks/inflight` is `ScopedCompany`
 * (`src/server/ops/tasks.rs`, `list_inflight`) — the same "any member" guard
 * `POST …/steer` carries, not `AdminScopedCompany` — and this is the AUTH
 * claim worth pinning: the read uses no elevated credential a member's session
 * does not already have, and its response reaches a steerable control with no
 * role check in between.
 *
 * Polling interval, page size and the read's own failure handling (the rest
 * of the ledger's gap text) are the shell's concern (`app-shell.tsx`'s
 * `refreshTaskStatuses`, which already treats a failed inflight poll as
 * best-effort) — informational here, not forced into this AUTH-only cell.
 */

const DELEGATION: InflightRun = {
  taskId: null,
  key: "run_7f3c9a",
  kind: "delegation",
  title: "Research the competitor pricing",
  agentId: "analyst",
  startedAt: 1_700_000_000_000,
  pendingAction: null,
};

function clientAs(get: (path: string) => Promise<unknown>): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company ?? "acme"}`,
    get: async (path: string) => get(path),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("listInflight — the same session a member already carries", () => {
  it("reads the company-scoped inflight route, addressing nothing an ordinary session lacks", async () => {
    const get = vi.fn(async (_path: string) => [DELEGATION]);
    const runs = await listInflight(clientAs(get), "acme");

    expect(get).toHaveBeenCalledTimes(1);
    expect(get.mock.calls[0][0]).toBe("/api/v1/companies/acme/tasks/inflight");
    expect(runs).toEqual([DELEGATION]);
  });
});

describe("listInflight's response reaches steerable controls with no role gate", () => {
  it("renders a Cancel control for a run listInflight actually returned", async () => {
    const get = vi.fn(async (_path: string) => [DELEGATION]);
    const client = clientAs(get);
    const runs = await listInflight(client, "acme");

    await act(async () => {
      root.render(
        createElement(InflightRunBar, {
          client,
          company: "acme",
          runs,
          onSteered: () => {},
        }),
      );
    });

    // No admin check stands between the read and the control: the same
    // client that fetched the list is offered the cancel it feeds.
    const cancel = container.querySelector(
      `button[aria-label="Cancel ${DELEGATION.title}"]`,
    );
    expect(cancel).not.toBeNull();
    expect((cancel as HTMLButtonElement).disabled).toBe(false);
  });

  it("renders nothing when the read comes back empty, rather than a stale or default control", async () => {
    const client = clientAs(async () => []);
    const runs = await listInflight(client, "acme");

    await act(async () => {
      root.render(
        createElement(InflightRunBar, {
          client,
          company: "acme",
          runs,
          onSteered: () => {},
        }),
      );
    });

    expect(container.querySelector('[data-testid="inflight-run-bar"]')).toBeNull();
  });
});
