// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { CompanyFeed } from "@/hooks/use-company";
import { ApprovalsView } from "@/views/ApprovalsView";

/**
 * Bulk resolve reuses the same admin-scoped route as a single card's footer,
 * once per row — so it must be withheld on the same basis: any row this
 * viewer cannot read must also not be swept into "Approve all" / "Decline
 * all".
 */

const NOW = new Date("2026-08-23T10:00:00Z").getTime();

function approval(id: string, contents_hidden?: boolean): ApprovalSummary {
  return {
    id,
    kind: "web_fetch",
    amount_usd: contents_hidden ? null : 0,
    at_millis: NOW,
    expires_at_millis: NOW + 60 * 60 * 1000,
    agent: "seo",
    thread: "desk-marketing",
    ...(contents_hidden ? { contents_hidden: true } : {}),
  };
}

const client = {
  get: async <T>(path: string): Promise<T> => (path.endsWith("/users") ? [] : null) as T,
  listGrants: async () => [],
  listTeam: async () => [],
  listDesks: async () => [],
  revokeGrant: async () => undefined,
  scopeFor: () => "/api/v1/company",
} as unknown as OpenCompanyClient;

function feedWith(approvals: ApprovalSummary[]): CompanyFeed {
  return {
    status: {} as CompanyFeed["status"],
    approvals,
    queue: "ready",
    now: NOW,
    refresh: async () => undefined,
  };
}

let container: HTMLDivElement;
let root: Root;

async function show(approvals: ApprovalSummary[]) {
  await act(async () => {
    root.render(
      createElement(ApprovalsView, {
        client,
        company: null,
        feed: feedWith(approvals),
        onResolved: () => {},
        onGoToConversation: () => {},
      }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });
}

function buttons(): string[] {
  return [...container.querySelectorAll("button")].map((b) => b.textContent ?? "");
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the queue header's bulk resolve, gated on contents_hidden", () => {
  it("offers Approve all / Decline all when every row in the batch is readable", async () => {
    await show([approval("a1"), approval("a2")]);

    expect(buttons().some((t) => t.includes("Approve all"))).toBe(true);
    expect(buttons().some((t) => t.includes("Decline all"))).toBe(true);
  });

  it("withholds bulk resolve when any row in the batch has hidden contents", async () => {
    await show([approval("a1"), approval("a2", true)]);

    expect(buttons().some((t) => t.includes("Approve all"))).toBe(false);
    expect(buttons().some((t) => t.includes("Decline all"))).toBe(false);
  });

  it("still offers each readable row's own decide footer even while bulk is withheld", async () => {
    await show([approval("a1"), approval("a2", true)]);

    const readableRow = container.querySelector('[data-approval-id="a1"]');
    expect(readableRow?.textContent).toContain("Approve");
  });
});
