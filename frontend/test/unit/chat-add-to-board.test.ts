// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import { MessageRow } from "@/views/chat/MessageRow";
import type { TimelineEntry } from "@/views/chat/model";

/**
 * "Add to board" (issue #246) on Room's message row.
 *
 * The affordance and its chip lived only on the legacy `#/conversation`
 * transcript, which had no nav row — so the one way to reach the console's
 * only chat-to-card path was to type the address by hand. Room is where the
 * operator actually is, and this is the guard that the action is offered
 * there: a render assertion rather than a source scan, because "the file
 * mentions the string" was true of the retired surface right up until it was
 * deleted.
 */

const NOW = 1_700_000_300_000;
const here = dirname(fileURLToPath(import.meta.url));
const chatView = readFileSync(resolve(here, "../../src/views/ChatView.tsx"), "utf8");

function entryFor(message: Partial<ChatMessage>): TimelineEntry {
  return {
    message: {
      id: "m1",
      from: "you",
      text: "ship the launch checklist",
      at: NOW,
      ...message,
    } as ChatMessage,
    sender: { key: "you", name: "You", kind: "you" },
    continuation: false,
    replies: [],
    replySenders: [],
  };
}

let container: HTMLDivElement;
let root: Root;

function render(entry: TimelineEntry, addingCardId: string | null = null) {
  act(() => {
    root.render(
      createElement(MessageRow, {
        entry,
        threadOpen: false,
        onOpenThread: () => {},
        onReact: () => {},
        onDismissCard: () => {},
        dismissingCardId: null,
        onAddToBoard: () => {},
        addingCardId,
        now: NOW,
      }),
    );
  });
}

const addButton = () => container.querySelector('button[aria-label="Add to board"]');

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the Add to board action on a Room message", () => {
  it("is offered on an ordinary line", () => {
    render(entryFor({}));
    expect(addButton()).not.toBeNull();
  });

  it("is offered on a teammate's line too, not only your own", () => {
    // Through REST rather than the responder's toolbelt, which is what makes
    // the action true on every line in every channel.
    render(entryFor({ from: "company", channel: "engineering" }));
    expect(addButton()).not.toBeNull();
  });

  it("is withdrawn once the line already has a card", () => {
    // Withdrawn, not disabled: a second press must not be able to open a
    // duplicate, and the chip is what says a card exists.
    render(entryFor({ taskId: "task-7" }));
    expect(addButton()).toBeNull();
  });

  it("is withdrawn on a line with no text to title a card from", () => {
    render(entryFor({ text: "   " }));
    expect(addButton()).toBeNull();
  });

  it("goes busy for the line whose create is in flight", () => {
    render(entryFor({}), "m1");
    expect(addButton()?.hasAttribute("disabled")).toBe(true);
  });
});

describe("the chip names which provenance opened the card", () => {
  it("says the operator added it on their own line", () => {
    render(entryFor({ taskId: "task-7" }));
    expect(container.textContent).toContain("Added to the board");
  });

  it("says the turn opened it on a company line", () => {
    render(entryFor({ from: "company", channel: "engineering", taskId: "task-7" }));
    expect(container.textContent).toContain("Card opened");
  });
});

describe("ChatView's create", () => {
  it("addresses the card's origin by the host thread, not the console channel id", () => {
    // `originChatId` is what the card's origin row resolves back through
    // `chatChannelByThread` to offer the jump to Room. A DM's channel id is
    // console-local and names no host thread, so filing it here would leave
    // the card unable to say where it came from.
    expect(chatView).toContain("originChatId: activeThreadId");
  });

  it("leaves the column and the assignee to the host's intake default", () => {
    // Dropping a card into `in_progress` is what dispatches a turn, so the
    // human drag stays the only thing that spends money.
    const at = chatView.indexOf("async function addToBoard");
    const body = chatView.slice(at, chatView.indexOf("finally", at));
    expect(body).not.toContain("column:");
    expect(body).not.toContain("assignee:");
  });
});
