// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { makeMessage } from "@/lib/chat";
import type { Channel } from "@/views/chat/model";
import { ThreadPanel } from "@/views/chat/ThreadPanel";

/**
 * `ThreadPanel`'s own inline Approve — the settle pill's twin for a card
 * whose review folded into an open thread. It reaches the same `POST {scope}/chat/review`
 * (`review_card`, `operator.rs`) as `MessageRow`'s pill
 * (`cov-chat-review-pill.test.ts`), which is `scoped(…)` — any company
 * member, not admin-only. `ThreadPanel` carries no role check of its own:
 * `onReviewCard` is wired unconditionally whenever `reviewing` is true, so
 * this pins Approve is offered to whichever viewer opened the thread.
 */

const CHANNEL: Channel = { id: "main", name: "main", kind: "channel", purpose: "The main channel" };
const PARENT = makeMessage("company", "Ship the thing?", { taskId: "t1" });

function baseProps(overrides: Record<string, unknown> = {}) {
  return {
    channel: CHANNEL,
    members: [],
    parent: PARENT,
    replies: [],
    sending: false,
    onSend: vi.fn(),
    onClose: vi.fn(),
    reviewing: true,
    reviewTaskId: "t1",
    onReviewCard: vi.fn(),
    reviewInFlight: false,
    ...overrides,
  };
}

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
  vi.restoreAllMocks();
});

function approveButton(): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent === "Approve" || b.textContent === "Approving…",
  );
}

describe("ThreadPanel's inline review Approve, by viewer", () => {
  it("offers a live Approve with no role check of its own", async () => {
    await act(async () => {
      root.render(createElement(ThreadPanel, baseProps()));
    });

    const btn = approveButton();
    expect(btn).not.toBeUndefined();
    expect(btn?.disabled).toBe(false);
  });

  it("wires a click straight to onReviewCard with the approve decision", async () => {
    const onReviewCard = vi.fn();
    await act(async () => {
      root.render(createElement(ThreadPanel, baseProps({ onReviewCard })));
    });
    await act(async () => approveButton()?.click());

    expect(onReviewCard).toHaveBeenCalledWith("t1", "approve");
  });

  it("withdraws Approve once nothing is reviewing — the only gate this panel enforces", async () => {
    await act(async () => {
      root.render(createElement(ThreadPanel, baseProps({ reviewing: false, reviewTaskId: undefined })));
    });

    expect(approveButton()).toBeUndefined();
  });
});
