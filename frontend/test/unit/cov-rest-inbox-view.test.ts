// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InboxDto, InboxMessageDto } from "@/api/types";
import { ApiError } from "@/api/types";
import { InboxView } from "@/views/InboxView";

/**
 * Both host routes behind this view (`GET …/inboxes`, `GET …/inboxes/{key}/
 * messages`) are `ScopedCompany` (`ops/mail.rs:30-32`), not admin-gated — a
 * member reads exactly what an admin reads, and the view offers nothing it
 * withholds by role. What is worth pinning here is the honesty half: a roster
 * read or a message read that fails must say so and offer a retry, never sit
 * on a blank list or an endless skeleton that looks the same as "no mail".
 */

const INBOX: InboxDto = {
  key: "sales",
  name: "Sales",
  address: "sales@acme.test",
  enabled: true,
  unread: 1,
};

const MESSAGE: InboxMessageDto = {
  id: "m1",
  inbox: "sales",
  fromName: "A Customer",
  fromEmail: "customer@example.com",
  subject: "Quote request",
  body: "Hi, can you send a quote?",
  atMillis: 1_700_000_000_000,
  read: false,
  outbound: false,
};

function clientAs(opts: {
  listInboxes?: () => Promise<InboxDto[]>;
  inboxMessages?: () => Promise<InboxMessageDto[]>;
}): OpenCompanyClient {
  return {
    listInboxes: vi.fn(opts.listInboxes ?? (() => Promise.resolve([INBOX]))),
    inboxMessages: vi.fn(opts.inboxMessages ?? (() => Promise.resolve([MESSAGE]))),
    markInboxRead: vi.fn(() => Promise.resolve({ unread: 0 })),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
  });
}

function at(testid: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
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

describe("InboxView, a surface with no role gate at all", () => {
  it("shows a plain member the same inbox and mail an admin would see — the reads carry no admin check", async () => {
    const client = clientAs({});
    await show(createElement(InboxView, { client, company: "acme" }));
    await act(async () => {});

    expect(at("inbox-list")).not.toBeNull();
    expect(container.textContent).toContain("Quote request");
    expect(at("inbox-select")).not.toBeNull();
  });
});

describe("InboxView, a failed read", () => {
  it("says the roster failed and offers a retry, rather than an empty inbox list", async () => {
    const client = clientAs({
      listInboxes: () => Promise.reject(new ApiError(0, "network_error", "cannot reach the company host")),
    });
    await show(createElement(InboxView, { client, company: "acme" }));
    await act(async () => {});

    expect(container.textContent).toContain("Inboxes unavailable");
    expect(container.textContent).toContain("cannot reach the company host");
    expect(at("inbox-list")).toBeNull();
    const retry = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Try again"),
    );
    expect(retry).toBeDefined();
  });

  it("says a message read failed and offers a retry, rather than an honest-looking empty mailbox", async () => {
    const client = clientAs({
      inboxMessages: () => Promise.reject(new ApiError(500, "server_error", "the store timed out")),
    });
    await show(createElement(InboxView, { client, company: "acme" }));
    await act(async () => {});

    expect(at("inbox-messages-error")).not.toBeNull();
    expect(at("inbox-messages-error")!.textContent).toContain("the store timed out");
    expect(at("inbox-empty")).toBeNull();
    const retry = Array.from(at("inbox-messages-error")!.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Try again"),
    );
    expect(retry).toBeDefined();
  });
});
