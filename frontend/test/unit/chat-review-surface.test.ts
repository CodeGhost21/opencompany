// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { TaskStatus } from "@/api/tasks";
import { makeMessage, type ChatMessage } from "@/lib/chat";
import { isTaskInReview, MessageRow } from "@/views/chat/MessageRow";
import { repliesInThread, reviewAnchorForThread, reviewCardIdForThread } from "@/views/chat/model";
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

  // Codex #3905031257: a card that finished, was revised, and is in_review
  // again mints a new settle pill for the same taskId while the old one stays
  // in history. Only the newest pill (or its relay) is a live review surface —
  // the same gate `buildTimeline` applies to the Approve control.
  it("declines a reply anchored on a stale settle pill from an earlier revise pass", () => {
    const oldPill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const newPill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const messages = [oldPill, newPill];
    expect(reviewCardIdForThread(oldPill, messages, IN_REVIEW)).toBeUndefined();
    expect(reviewCardIdForThread(newPill, messages, IN_REVIEW)).toBe("t-1");
  });

  it("declines a reply anchored on the relay bubble of a stale settle pill", () => {
    const oldPill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const oldRelay = makeMessage("company", "Here is the first draft.", {});
    const newPill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const messages = [oldPill, oldRelay, newPill];
    expect(reviewCardIdForThread(oldRelay, messages, IN_REVIEW)).toBeUndefined();
  });

  it("gates the Approve control on the linked card still being in review", () => {
    expect(isTaskInReview({ column: "in_review" })).toBe(true);
    expect(isTaskInReview({ column: "done" })).toBe(false);
    expect(isTaskInReview(undefined)).toBe(false);
  });
});

describe("the anchor a thread's review composer submits", () => {
  it("anchors on the root itself when it is the review surface", () => {
    const pill = makeMessage("system", "finished → In review", { taskId: "t-1" });
    const reply = makeMessage("you", "looks good", { parentId: pill.id });
    expect(reviewAnchorForThread(pill, [reply], [pill, reply], IN_REVIEW)).toEqual({
      taskId: "t-1",
      anchorId: pill.id,
    });
  });

  it("anchors on the relay among the thread's own replies when the card was dispatched from inside an existing thread", () => {
    const root = makeMessage("you", "kick off the writeup", {});
    const trigger = makeMessage("company", "starting on it", { parentId: root.id });
    const pill = makeMessage("system", "finished → In review", {
      taskId: "t-1",
      parentId: root.id,
    });
    const relay = makeMessage("company", "Here is the draft.", { parentId: root.id });
    const messages = [root, trigger, pill, relay];
    const replies = [trigger, pill, relay];

    expect(reviewAnchorForThread(root, replies, messages, IN_REVIEW)).toEqual({
      taskId: "t-1",
      anchorId: relay.id,
    });
  });

  it("resolves nothing when neither the root nor any reply is a review surface", () => {
    const root = makeMessage("you", "just chatting", {});
    const reply = makeMessage("company", "sure thing", { parentId: root.id });
    expect(reviewAnchorForThread(root, [reply], [root, reply], IN_REVIEW)).toBeUndefined();
  });
});

describe("the messages an open thread panel renders", () => {
  it("keeps an ordinary direct reply visible (unchanged from a same-level filter)", () => {
    const root = makeMessage("you", "kick off the writeup", {});
    const reply = makeMessage("company", "on it", { parentId: root.id });
    expect(repliesInThread(root, [root, reply])).toEqual([reply]);
  });

  // Codex #3906409961: a card dispatched from inside an already-open thread
  // lands its settle pill/relay as replies to that thread's ROOT (a same-desk
  // fact the backend's own review-anchor lookup relies on — see
  // `reviewAnchorForThread`'s doc). Review feedback the operator sends from
  // that thread is anchored on the relay, not the root, because that is the
  // id the host needs to find the card. That makes the operator's own message
  // a reply-to-the-relay, i.e. a grandchild of the thread root. Before this
  // fix the panel filtered on `m.parentId === parent.id` — a same-level
  // check — so the operator's message vanished from the thread the instant
  // it sent, in both the optimistic bubble and the persisted echo.
  it("still shows the operator's own message after it sends review feedback anchored on a nested relay", () => {
    const root = makeMessage("you", "kick off the writeup", {});
    const trigger = makeMessage("company", "starting on it", { parentId: root.id });
    const pill = makeMessage("system", "finished → In review", {
      taskId: "t-1",
      parentId: root.id,
    });
    const relay = makeMessage("company", "Here is the draft.", { parentId: root.id });
    const feedback = makeMessage("you", "make it punchier", { parentId: relay.id });
    const messages = [root, trigger, pill, relay, feedback];

    // Pre-fix behaviour, kept here as the regression proof: a same-level
    // filter drops `feedback` because its parentId is the relay, not the root.
    const sameLevelFilter = messages.filter((m) => m.parentId === root.id);
    expect(sameLevelFilter).not.toContain(feedback);

    expect(repliesInThread(root, messages)).toEqual([trigger, pill, relay, feedback]);
  });

  it("does not pull in a sibling thread's messages", () => {
    const root = makeMessage("you", "kick off the writeup", {});
    const reply = makeMessage("company", "on it", { parentId: root.id });
    const otherRoot = makeMessage("you", "unrelated question", {});
    const otherReply = makeMessage("company", "sure", { parentId: otherRoot.id });
    const messages = [root, reply, otherRoot, otherReply];
    expect(repliesInThread(root, messages)).toEqual([reply]);
  });
});

let container: HTMLDivElement;
let root: Root;

function render(element: ReturnType<typeof createElement>) {
  act(() => root.render(element));
}

function systemEntry(message: ChatMessage, isLatestSettlePill = true): TimelineEntry {
  const sender: Sender = { key: "system", name: "System", kind: "system" };
  return { message, sender, continuation: false, replies: [], replySenders: [], isLatestSettlePill };
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

  it("hides Approve on an earlier pass's pill once a later pass is the one in review", () => {
    render(
      createElement(MessageRow, {
        entry: systemEntry(pill, false),
        threadOpen: false,
        onOpenThread: () => {},
        onReact: () => {},
        onDismissCard: () => {},
        dismissingCardId: null,
        onReviewCard: () => {},
        taskStatusByTaskId: IN_REVIEW,
      }),
    );
    expect(approveButton()).toBeFalsy();
  });
});
