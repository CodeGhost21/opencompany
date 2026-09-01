// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  AssistantRuntimeProvider,
  useExternalStoreRuntime,
  type ThreadMessageLike,
} from "@assistant-ui/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { Transcript } from "@/views/conversation/Transcript";

/**
 * The `#/conversation` surface's half of the same fix as
 * `chat-readonly-composer.test.ts`.
 *
 * This route reads the Operator feed through `operatorThread`
 * (`conversation-operator-thread.test.ts`) and forwards `readOnly` into
 * `Transcript`, which answered it by disabling the assistant-ui composer:
 * a greyed-out input reading "This channel is read-only", a Send button, and
 * an "Enter to send · Shift+Enter for a new line" hint — all directly under a
 * notice saying there is nothing to reply to here. That is the surface the
 * bug was reported against.
 *
 * The composer is now not rendered at all. As in the channel test, every
 * read-only assertion is an assertion of absence, so the writable case is
 * what proves the queries can find anything in the first place.
 */

function Harness({ readOnly }: { readOnly?: boolean }) {
  const runtime = useExternalStoreRuntime<ThreadMessageLike>({
    isRunning: false,
    messages: [],
    convertMessage: (m) => m,
    onNew: async () => {},
  });
  return createElement(
    AssistantRuntimeProvider,
    { runtime },
    createElement(Transcript, {
      contact: { name: "Operator", kind: "company" as const },
      readOnly,
      onAddToBoard: () => {},
      addingId: null,
      onDismissCard: () => {},
      dismissingCardId: null,
      sending: false,
    }),
  );
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // assistant-ui's viewport measures its content; jsdom ships no
  // `ResizeObserver`. A no-op is enough — nothing here asserts on layout.
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(readOnly?: boolean) {
  const node: ReactNode = createElement(Harness, { readOnly });
  await act(async () => {
    root.render(node);
  });
}

describe("the conversation transcript on a read-only thread", () => {
  it("renders no composer input and no Send button", async () => {
    await render(true);

    expect(container.querySelector("textarea")).toBeNull();
    expect(container.querySelector('[aria-label="Send"]')).toBeNull();
    expect(container.querySelector('[data-tour="chat-composer"]')).toBeNull();
  });

  it("drops the keyboard hint with the send it describes", async () => {
    await render(true);

    expect(container.textContent).not.toContain("Enter to send");
    expect(container.textContent).not.toContain("Shift+Enter");
  });

  it("keeps the single notice that explains why, and does not repeat it", async () => {
    await render(true);

    const notices = Array.from(container.querySelectorAll('[role="status"]')).filter((n) =>
      n.textContent?.includes("There is nothing to reply to here"),
    );
    expect(notices).toHaveLength(1);
    // Nothing below it: the composer is gone, not hidden underneath.
    expect(notices[0].nextElementSibling).toBeNull();
  });
});

describe("the conversation transcript on an ordinary thread", () => {
  it("still renders the whole composer", async () => {
    await render(false);

    const input = container.querySelector("textarea");
    expect(input).not.toBeNull();
    expect(input?.getAttribute("placeholder")).toBe("Message Operator…");
    expect(container.querySelector('[aria-label="Send"]')).not.toBeNull();
    expect(container.querySelector('[data-tour="chat-composer"]')).not.toBeNull();
    expect(container.textContent).toContain("Enter to send · Shift+Enter for a new line");
    expect(container.textContent).not.toContain("There is nothing to reply to here");
  });

  it("defaults to writable when readOnly is omitted", async () => {
    await render(undefined);

    expect(container.querySelector("textarea")).not.toBeNull();
    expect(container.querySelector('[aria-label="Send"]')).not.toBeNull();
  });
});
