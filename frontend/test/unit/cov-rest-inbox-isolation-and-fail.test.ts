// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InboxDto, InboxMessageDto } from "@/api/types";
import { InboxView } from "@/views/InboxView";

/**
 * Each teammate's inbox is its own scope — `client.inboxMessages(key, …)` —
 * and nothing else in this view narrows who can read it, so the property
 * worth pinning is isolation: switching the selector must never let a slow
 * response for the inbox the operator left land under the one they moved to.
 * Paired with the honest-failure half no test exercises either: a rejected
 * read must say so, not blank the pane or spin forever.
 */

function inbox(over: Partial<InboxDto> = {}): InboxDto {
  return { key: "alex", name: "Alex", address: "alex@acme.test", enabled: true, unread: 0, ...over };
}

function msg(over: Partial<InboxMessageDto> = {}): InboxMessageDto {
  return {
    id: "m1",
    inbox: "alex",
    fromEmail: "x@y.com",
    fromName: "X",
    subject: "hi",
    body: "hi there",
    read: true,
    outbound: false,
    atMillis: 1,
    ...over,
  };
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(InboxView, { client, company: "acme" }));
  });
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
  vi.restoreAllMocks();
});

describe("switching teammates never leaks a slower read across the boundary", () => {
  it("drops a late response for the inbox the operator has already left", async () => {
    let resolveAlex: ((rows: InboxMessageDto[]) => void) | null = null;
    const inboxMessages = vi.fn((key: string) => {
      if (key === "alex") {
        return new Promise<InboxMessageDto[]>((resolve) => {
          resolveAlex = resolve;
        });
      }
      return Promise.resolve([msg({ id: "b1", subject: "Blair's mail" })]);
    });

    const client = {
      listInboxes: vi.fn(() => Promise.resolve([inbox({ key: "alex" }), inbox({ key: "blair", name: "Blair" })])),
      inboxMessages,
    } as unknown as OpenCompanyClient;

    await show(client);
    // The view lands on the first enabled inbox ("alex"), whose read is still
    // pending — `resolveAlex` has not fired.

    await act(async () => {
      const select = container.querySelector('[data-testid="inbox-select"]') as HTMLElement;
      select.click();
    });
    const blairOption = Array.from(document.querySelectorAll('[role="option"]')).find((o) =>
      o.textContent?.includes("Blair"),
    ) as HTMLElement | undefined;
    await act(async () => {
      blairOption?.click();
    });

    expect(container.querySelector('[data-testid="inbox-message"]')?.textContent).toContain(
      "Blair's mail",
    );

    // Alex's slow read now resolves — after the operator has moved on.
    await act(async () => {
      resolveAlex?.([msg({ id: "a1", subject: "Alex's mail" })]);
      await Promise.resolve();
    });

    expect(container.textContent).not.toContain("Alex's mail");
    expect(container.querySelector('[data-testid="inbox-message"]')?.textContent).toContain(
      "Blair's mail",
    );
  });
});

describe("a failed mailbox read is never mistaken for an empty one", () => {
  it("says the read failed and offers a retry, instead of a blank list", async () => {
    const client = {
      listInboxes: vi.fn(() => Promise.resolve([inbox()])),
      inboxMessages: vi.fn(() => Promise.reject(new Error("mailbox host unreachable"))),
    } as unknown as OpenCompanyClient;

    await show(client);

    const errorPane = container.querySelector('[data-testid="inbox-messages-error"]');
    expect(errorPane).not.toBeNull();
    expect(errorPane?.textContent).toContain("mailbox host unreachable");
    expect(container.querySelector('[data-testid="inbox-empty"]')).toBeNull();
    const retry = Array.from(errorPane?.querySelectorAll("button") ?? []).find((b) =>
      b.textContent?.includes("Try again"),
    );
    expect(retry).toBeDefined();
  });
});
