// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { TaskStatus } from "@/api/tasks";
import { makeMessage, type ChatMessage } from "@/lib/chat";
import { isTaskInReview, MessageRow } from "@/views/chat/MessageRow";
import { reviewCardIdForThread } from "@/views/chat/model";
import type { Sender, TimelineEntry } from "@/views/chat/model";

const IN_REVIEW: Readonly<Record<string, TaskStatus>> = {
  "t-1": { column: "in_review" },
};

describe("the in-review card a chat thread reviews", () => {
  it("resolves a settle pill to its in-review card", () => {
    const pill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    expect(reviewCardIdForThread(pill, [pill], IN_REVIEW)).toBe("t-1");
  });

  it("resolves a relay bubble via the settle pill before it", () => {
    const pill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const relay = makeMessage("company", "Here is the draft.", {});
    expect(reviewCardIdForThread(relay, [pill, relay], IN_REVIEW)).toBe("t-1");
  });

  it("resolves nothing when the card has left review", () => {
    const pill = makeMessage("system", "finished → Done", { taskId: "t-1" });
    const done: Readonly<Record<string, TaskStatus>> = { "t-1": { column: "done" } };
    expect(reviewCardIdForThread(pill, [pill], done)).toBeUndefined();
  });

  it("resolves nothing for an ordinary line with no settle pill before it", () => {
    const chatter = makeMessage("company", "just chatting", {});
    expect(reviewCardIdForThread(chatter, [chatter], IN_REVIEW)).toBeUndefined();
  });

  it("gates the Approve control on the linked card still being in review", () => {
    expect(isTaskInReview({ column: "in_review" })).toBe(true);
    expect(isTaskInReview({ column: "done" })).toBe(false);
    expect(isTaskInReview(undefined)).toBe(false);
  });
});

let container: HTMLDivElement;
let root: Root;

function render(element: ReturnType<typeof createElement>) {
  act(() => root.render(element));
}

function systemEntry(message: ChatMessage): TimelineEntry {
  const sender: Sender = { key: "system", name: "System", kind: "system" };
  return { message, sender, continuation: false, replies: [], replySenders: [] };
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

function approveButton(): HTMLButtonElement | undefined {
  return [...container.querySelectorAll("button")].find((b) =>
    (b.textContent ?? "").includes("Approve"),
  ) as HTMLButtonElement | undefined;
}

describe("the settle pill's Approve control", () => {
  const pill = makeMessage("system", "finished → In review", { taskId: "t-1" });

  it("offers Approve only while the linked card is in review", () => {
    render(
      createElement(MessageRow, {
        entry: systemEntry(pill),
        threadOpen: false,
        onOpenThread: () => {},
        onReact: () => {},
        onDismissCard: () => {},
        dismissingCardId: null,
        onReviewCard: () => {},
        taskStatusByTaskId: IN_REVIEW,
      }),
    );
    expect(approveButton()).toBeTruthy();
  });

  it("hides Approve once the card has left review", () => {
    render(
      createElement(MessageRow, {
        entry: systemEntry(pill),
        threadOpen: false,
        onOpenThread: () => {},
        onReact: () => {},
        onDismissCard: () => {},
        dismissingCardId: null,
        onReviewCard: () => {},
        taskStatusByTaskId: { "t-1": { column: "done" } },
      }),
    );
    expect(approveButton()).toBeFalsy();
  });

  it("calls reviewCard with the pill's task id on click", () => {
    const calls: Array<[string, string]> = [];
    render(
      createElement(MessageRow, {
        entry: systemEntry(pill),
        threadOpen: false,
        onOpenThread: () => {},
        onReact: () => {},
        onDismissCard: () => {},
        dismissingCardId: null,
        onReviewCard: (taskId, decision) => calls.push([taskId, decision]),
        taskStatusByTaskId: IN_REVIEW,
      }),
    );
    act(() => {
      approveButton()?.click();
    });
    expect(calls).toEqual([["t-1", "approve"]]);
  });
});
