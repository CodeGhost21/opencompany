// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MessageRow } from "@/views/chat/MessageRow";
import type { TimelineEntry } from "@/views/chat/model";

/** Issue #1396: adjacent reaction chips need a 24px target on touch layouts. */

let container: HTMLDivElement;
let root: Root;

const ENTRY: TimelineEntry = {
  message: {
    id: "h1",
    from: "you",
    text: "A message with a reaction.",
    at: 1_700_000_000_000,
    reactions: [{ emoji: "👍", by: "Mithil", mine: false }],
  },
  sender: { key: "you", name: "You", kind: "you" },
  continuation: false,
  replies: [],
  replySenders: [],
};

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

describe("reaction chips at a touch viewport", () => {
  it("reserve a 24px minimum height below md while retaining desktop density", () => {
    act(() => {
      root.render(
        createElement(MessageRow, {
          entry: ENTRY,
          threadOpen: false,
          onOpenThread: () => {},
          onReact: () => {},
          onDismissCard: () => {},
          dismissingCardId: null,
        }),
      );
    });

    const chip = [...container.querySelectorAll("button")].find((button) =>
      button.className.includes("rounded-full"),
    );
    expect(chip).not.toBeNull();
    expect(chip?.className).toContain("min-h-6");
    expect(chip?.className).toContain("md:min-h-0");
  });
});
