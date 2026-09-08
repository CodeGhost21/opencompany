// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { DiscussionMessage } from "@/api/tasks";

/**
 * Withdrawing a discussion message.
 *
 * `redactTaskDiscussion` is an append-only tombstone — the journal keeps the
 * original event, and the host substitutes its fixed placeholder on every
 * later read. `task-detail-discussion-limits.test.ts` pins the in-flight
 * state a repeat click cannot double-fire, but no test ever checks the thing
 * the withdrawal is *for*: that once the row lands, the original text this
 * tab itself was just rendering is gone from the screen, not just replaced by
 * something sitting beside it. Nor does anything pin what happens when the
 * `DELETE` is refused — a silent no-op there would leave the operator
 * believing a leaked credential was pulled when it was not.
 *
 * `redactTaskDiscussion` is `ScopedCompany` in `server::ops::tasks` — any
 * member, the same authority `DELETE …/tasks/{id}` carries — and `DiscussionTab`
 * matches that: it reads no role before rendering the Remove control.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

const { DiscussionTab } = await import("@/views/TaskDetailView");

const SECRET = "sk-live-not-a-real-credential-abc123";

const MESSAGE: DiscussionMessage = {
  seq: 7,
  author: "ops",
  atMillis: new Date("2026-03-02T10:00:00Z").getTime(),
  text: `rotate this key please: ${SECRET}`,
};

const TOMBSTONE: DiscussionMessage = {
  seq: 7,
  author: "ops",
  atMillis: MESSAGE.atMillis,
  text: "This message was removed.",
  redacted: true,
  redactedBy: "finance",
};

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(DiscussionTab, {
        messages: [MESSAGE],
        hasMore: false,
        taskId: "task-1",
        client,
        company: "acme",
        onPosted: async () => {},
      }),
    );
  });
}

/** Presses Remove and confirms it in the portalled dialog. */
async function withdraw() {
  const trigger = container.querySelector(
    '[data-testid="discussion-redact"]',
  ) as HTMLButtonElement;
  await act(async () => trigger.click());
  const confirm = [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === "Remove it",
  ) as HTMLButtonElement;
  await act(async () => confirm.click());
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("a successful withdrawal leaves nothing client-side to read", () => {
  it("replaces the secret with the host's tombstone once the DELETE lands", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      del: async () => TOMBSTONE,
    } as unknown as OpenCompanyClient);

    expect(container.textContent).toContain(SECRET);
    await withdraw();

    // Not "also shows a tombstone" — the original text must be gone, because
    // this thread is the read surface the withdrawal exists to close.
    expect(container.textContent).not.toContain(SECRET);
    expect(container.textContent).toContain("This message was removed.");
    expect(container.textContent).toContain("Removed by finance.");
  });

  it("lands on the row's own seq, so it does not need the next 4s poll", async () => {
    // `absorb([row])` runs straight off the DELETE's response, not off a
    // re-read — a polled surface withdrawing late is the risk this proves is
    // not also true of the row that requested it.
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      del: async () => TOMBSTONE,
    } as unknown as OpenCompanyClient);
    await withdraw();
    expect(
      container.querySelector('[data-testid="discussion-message"]')?.getAttribute(
        "data-redacted",
      ),
    ).toBe("true");
  });
});

describe("a withdrawal the host refuses", () => {
  it("leaves the secret on screen and says so, rather than a silent success", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      del: async () => {
        throw new Error("could not reach the host");
      },
    } as unknown as OpenCompanyClient);

    await withdraw();

    expect(container.textContent).toContain(SECRET);
    expect(container.textContent).not.toContain("This message was removed.");
    expect(toasts.error).toHaveBeenCalledTimes(1);
    expect(toasts.success).not.toHaveBeenCalled();
    // The control comes back rather than staying stuck mid-request.
    expect(
      (container.querySelector('[data-testid="discussion-redact"]') as HTMLButtonElement).disabled,
    ).toBe(false);
  });
});

describe("Remove is offered with no role read at all", () => {
  it("renders live for a client that would fail on any role read", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      get: async () => {
        throw new Error("DiscussionTab must not read a role before offering Remove");
      },
    } as unknown as OpenCompanyClient);

    const trigger = container.querySelector(
      '[data-testid="discussion-redact"]',
    ) as HTMLButtonElement;
    expect(trigger).not.toBeNull();
    expect(trigger.disabled).toBe(false);
  });
});
