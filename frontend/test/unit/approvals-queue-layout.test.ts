// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ApprovalSummary, StandingGrant } from "@/api/types";
import type { CompanyFeed } from "@/hooks/use-company";
import { ApprovalsView } from "@/views/ApprovalsView";

const NOW = new Date("2026-08-23T10:00:00Z").getTime();

function approval(id: string, expires_at_millis: number): ApprovalSummary {
  return {
    id,
    kind: "web_fetch",
    amount_usd: null,
    at_millis: NOW,
    expires_at_millis,
  };
}

const GRANT: StandingGrant = {
  id: "grant-1",
  agent: "ops",
  tool: "shell",
  granted_by: { kind: "user", id: "operator" },
  verdict: "approve",
  at_millis: NOW,
  expires_at_millis: NOW + 60 * 60 * 1000,
};

const client = {
  get: async <T,>(path: string): Promise<T> => (path.endsWith("/users") ? [] : null) as T,
  listGrants: async () => [GRANT],
  listTeam: async () => [],
  revokeGrant: async () => undefined,
  scopeFor: () => "/api/v1/company",
} as unknown as OpenCompanyClient;

const feed: CompanyFeed = {
  status: {} as CompanyFeed["status"],
  approvals: [
    approval("tomorrow", NOW + 24 * 60 * 60 * 1000),
    approval("soon", NOW + 4 * 60 * 1000),
  ],
  queue: "ready",
  now: NOW,
  refresh: async () => undefined,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("the approvals backlog queue (#1427)", () => {
  it("keeps orientation sticky, orders cards by deadline, and puts revocation first", async () => {
    await act(async () => {
      root.render(
        createElement(ApprovalsView, {
          client,
          company: null,
          feed,
          onResolved: () => {},
          onGoToConversation: () => {},
        }),
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    const queueHeading = [...container.querySelectorAll("h2")].find(
      (heading) => heading.textContent === "2 things need your approval",
    );
    const permissionsHeading = [...container.querySelectorAll("h2")].find(
      (heading) => heading.textContent === "Standing permissions",
    );
    expect(queueHeading).toBeDefined();
    expect(permissionsHeading).toBeDefined();
    expect(permissionsHeading!.compareDocumentPosition(queueHeading!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(queueHeading!.parentElement?.className).toContain("sticky");
    expect(queueHeading!.parentElement?.textContent).toContain("Each one has a deadline.");
    expect([...container.querySelectorAll("[data-approval-id]")].map((row) => row.getAttribute("data-approval-id"))).toEqual([
      "soon",
      "tomorrow",
    ]);
  });

  it("holds the whole permissions-and-queue region while revoking a grant (#1593)", async () => {
    vi.useFakeTimers();
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => "visible",
    });

    let granted = true;
    const revokingClient = {
      ...client,
      listGrants: async () => (granted ? [GRANT] : []),
    } as unknown as OpenCompanyClient;

    await act(async () => {
      root.render(
        createElement(ApprovalsView, {
          client: revokingClient,
          company: null,
          feed,
          onResolved: () => {},
          onGoToConversation: () => {},
        }),
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    const permissionHeading = () =>
      [...container.querySelectorAll("h2")].find(
        (heading) => heading.textContent === "Standing permissions",
      );
    expect(permissionHeading()).toBeDefined();

    const revoke = [...container.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Remove"),
    )!;
    await act(async () => revoke.focus());

    // Another tab revokes it while the operator is still aiming at this control.
    // The section must not disappear and pull the approval buttons upward.
    granted = false;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(permissionHeading()).toBeDefined();

    await act(async () => revoke.blur());
    expect(permissionHeading()).toBeUndefined();
  });
});
