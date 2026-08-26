// @vitest-environment jsdom

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantRuntimeProvider } from "@assistant-ui/react";

import type { OpenCompanyClient } from "@/api/client";
import type { Thread } from "@/lib/threads";
import { Transcript } from "@/views/conversation/Transcript";
import { useConversationRuntime } from "@/views/conversation/runtime";

/**
 * Codex review finding on #1757 (PR #1781): the still-supported `#/conversation`
 * route builds its thread list straight from `/desks` via `threadsFromDesks`,
 * which used to drop the desk's `system` flag on the floor — so opening the
 * durable Operator report as a legacy "thread" rendered an ordinary writable
 * composer, and `useConversationRuntime`'s `onNew` would reach `client.chat`
 * with `chat: "operator"` before the server's read-only guard finally refused
 * it. `ChatView`'s Chat-tab composer already had this exact fix
 * (`chat-thread-operator-readonly.test.ts`); this surface did not.
 *
 * `threadsFromDesks` (`@/lib/threads.ts`) now carries `readOnly: desk.system`
 * onto the `Thread`, and both the composer render (`Transcript.tsx`, disabled
 * input/button + banner) and the data layer (`runtime.ts`'s `onNew`, a belt
 * that never calls `client.chat` regardless of what the composer rendered)
 * enforce it. These pin both halves, and that an ordinary thread is
 * unaffected.
 */

const OPERATOR_THREAD: Thread = {
  id: "operator",
  contact: { name: "Operator", kind: "company" },
  blurb: "Workflow reports",
  messages: [],
  readOnly: true,
};

const MAIN_THREAD: Thread = {
  id: "main",
  contact: { name: "Your company", kind: "company" },
  blurb: "The main line",
  messages: [],
};

/**
 * jsdom performs no layout and ships no `ResizeObserver` at all — the
 * transcript viewport primitive observes its box on mount regardless, so
 * rendering it here needs the same no-op stub `chat-scroll-anchor.test.ts`
 * uses.
 */
class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
let savedResizeObserver: typeof globalThis.ResizeObserver | undefined;
/**
 * jsdom performs no layout, so `Element.scrollTo` does not exist either — the
 * viewport's auto-scroll-to-bottom effect calls it on mount regardless. A
 * no-op is enough here: nothing in these assertions cares where the
 * transcript scrolled to.
 */
let savedScrollTo: typeof Element.prototype.scrollTo | undefined;

let container: HTMLDivElement;
let root: Root;
let chatSpy: ReturnType<typeof vi.fn>;

function clientWith(chat: ReturnType<typeof vi.fn>): OpenCompanyClient {
  const named: Record<string, unknown> = { chat };
  return new Proxy(named, {
    get: (target, prop: string) => target[prop] ?? (() => Promise.resolve([])),
  }) as unknown as OpenCompanyClient;
}

/** Reproduces `ChatPane`'s wiring (`Conversation.tsx`) without the rest of the shell. */
function Harness({ thread, client }: { thread: Thread; client: OpenCompanyClient }) {
  const [sending, setSending] = useState(false);
  const { runtime } = useConversationRuntime({
    client,
    company: "acme",
    thread,
    setMessages: () => {},
    running: sending,
    setSending,
  });
  return createElement(
    AssistantRuntimeProvider,
    { runtime },
    createElement(Transcript, {
      contact: thread.contact,
      onAddToBoard: () => {},
      addingId: null,
      onDismissCard: () => {},
      dismissingCardId: null,
      sending,
      readOnly: thread.readOnly,
    }),
  );
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  savedResizeObserver = globalThis.ResizeObserver;
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = TestResizeObserver;
  savedScrollTo = Element.prototype.scrollTo;
  Element.prototype.scrollTo = () => {};
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  chatSpy = vi.fn().mockResolvedValue({ responses: [] });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = savedResizeObserver;
  if (savedScrollTo) Element.prototype.scrollTo = savedScrollTo;
});

async function render(thread: Thread) {
  const client = clientWith(chatSpy);
  await act(async () => {
    root.render(createElement(Harness, { thread, client }));
  });
}

function textarea() {
  return container.querySelector("textarea") as HTMLTextAreaElement;
}

function sendButton() {
  return container.querySelector('[aria-label="Send"]') as HTMLButtonElement;
}

async function type(text: string) {
  const el = textarea();
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  await act(async () => {
    setValue?.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("Conversation route composer on the Operator system thread (issue #1757)", () => {
  it("disables the composer for the Operator thread", async () => {
    await render(OPERATOR_THREAD);
    await type("can I help?");

    expect(textarea().disabled).toBe(true);
    expect(textarea().placeholder).toBe("This channel is read-only");
    expect(sendButton().disabled).toBe(true);
  });

  it("never reaches client.chat from a click on the Operator thread", async () => {
    await render(OPERATOR_THREAD);
    await type("can I help?");
    await act(async () => sendButton().click());

    expect(chatSpy).not.toHaveBeenCalled();
  });

  it("keeps the composer working on an ordinary thread", async () => {
    await render(MAIN_THREAD);
    await type("on it");

    expect(textarea().disabled).toBe(false);
    expect(sendButton().disabled).toBe(false);

    await act(async () => sendButton().click());
    expect(chatSpy).toHaveBeenCalledTimes(1);
    expect(chatSpy).toHaveBeenCalledWith("on it", "acme", "main", undefined, undefined, true);
  });
});
