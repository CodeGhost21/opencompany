// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MessageRow } from "@/views/chat/MessageRow";
import { makeMessage } from "@/lib/chat";
import type { TimelineEntry } from "@/views/chat/model";
import type { TaskStatus } from "@/api/tasks";

/**
 * The settle pill's Approve control (`ChatView.reviewCard`, `POST
 * …/chat/review`). The backend route is `ScopedCompany` (`operator.rs`),
 * not admin-only — any company member may settle a card they are reviewing
 * — so the console rightly offers Approve to every viewer; what it must not
 * do is offer it once the card has left `in_review`, which is the only gate
 * `review_card` actually enforces (a 404 "no card is awaiting review").
 */

const SYSTEM_MESSAGE = makeMessage("system", "Task done — Ship the thing", { taskId: "t1" });

function entry(overrides: Partial<TimelineEntry> = {}): TimelineEntry {
  return {
    message: SYSTEM_MESSAGE,
    sender: { key: "system", name: "system", kind: "system" },
    continuation: false,
    replies: [],
    replySenders: [],
    ...overrides,
  } as TimelineEntry;
}

function baseProps(overrides: Record<string, unknown> = {}) {
  return {
    entry: entry(),
    threadOpen: false,
    onOpenThread: vi.fn(),
    onReact: vi.fn(),
    onDismissCard: vi.fn(),
    dismissingCardId: null,
    onReviewCard: vi.fn(),
    reviewingCardIds: new Set<string>(),
    taskStatusByTaskId: { t1: { column: "in_review" } as TaskStatus },
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

function approveButton(): HTMLButtonElement | null {
  return Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent === "Approve" || b.textContent === "Approving…",
  ) as HTMLButtonElement | undefined ?? null;
}

describe("chat settle pill Approve, by whether the card is still in review", () => {
  it("offers Approve to any viewer while the card is in review", async () => {
    await act(async () => {
      root.render(createElement(MessageRow, baseProps()));
    });
    const btn = approveButton();
    expect(btn).not.toBeNull();
    expect(btn?.disabled).toBe(false);
  });

  it("wires a click straight to onReviewCard with the approve decision", async () => {
    const onReviewCard = vi.fn();
    await act(async () => {
      root.render(createElement(MessageRow, baseProps({ onReviewCard })));
    });
    await act(async () => approveButton()?.click());

    expect(onReviewCard).toHaveBeenCalledWith("t1", "approve");
  });

  it("withdraws Approve once the card has left review — the only gate the host enforces", async () => {
    await act(async () => {
      root.render(
        createElement(
          MessageRow,
          baseProps({ taskStatusByTaskId: { t1: { column: "done" } as TaskStatus } }),
        ),
      );
    });

    expect(approveButton()).toBeNull();
  });

  it("disables Approve and shows it in flight while a verdict is already submitting", async () => {
    await act(async () => {
      root.render(
        createElement(MessageRow, baseProps({ reviewingCardIds: new Set(["t1"]) })),
      );
    });

    const btn = approveButton();
    expect(btn?.textContent).toBe("Approving…");
    expect(btn?.disabled).toBe(true);
  });
});
