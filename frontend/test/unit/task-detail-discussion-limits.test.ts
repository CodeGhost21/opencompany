// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { DiscussionMessage } from "@/api/tasks";

/**
 * The Discussion tab's two silences.
 *
 * **A message is truncated at 4000 codepoints, host-side, with a `201`.**
 * `cap_discussion` in `src/ports/tasks.rs` takes the first 4000 characters and
 * stores them; the write succeeds either way. Nothing in the composer said so —
 * no counter, no cap — so an operator pasting a long note learned the limit
 * existed by reading their own echoed row and finding its tail gone, after the
 * words were unrecoverable.
 *
 * **Withdrawing a message had no busy state.** It is the only mutating control
 * on this screen that did not: `redact()` set no flag, so a second click fired
 * a second `DELETE`, and a third a third. The host is idempotent so nothing
 * ever broke — what it cost was the operator's confidence, because the only
 * answer to a click was the row changing whenever a request happened to land.
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

const { DiscussionTab, MAX_DISCUSSION_CHARS, capDiscussion } = await import(
  "@/views/TaskDetailView"
);

const MESSAGE: DiscussionMessage = {
  seq: 7,
  author: "ops",
  atMillis: new Date("2026-03-02T10:00:00Z").getTime(),
  text: "the invoice numbers do not line up",
};

/** Counts `DELETE`s and never settles them, so "in flight" is a real state. */
function stallingClient(seen: string[]): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    del: async (path: string) => {
      seen.push(path);
      return new Promise(() => {});
    },
  } as unknown as OpenCompanyClient;
}

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

function textarea(): HTMLTextAreaElement {
  return container.querySelector("textarea") as HTMLTextAreaElement;
}

/**
 * The counter the composer names in `aria-describedby`.
 *
 * Found through that attribute rather than by class or position, because the
 * link is half of what is being claimed: a description the textarea does not
 * point at is one no screen reader reads. `getElementById` rather than a `#id`
 * selector — `useId` mints ids containing characters a CSS selector would need
 * escaped, and jsdom here has no escaping helper to do it with.
 */
function counter(): HTMLElement {
  const id = textarea().getAttribute("aria-describedby");
  expect(id).toBeTruthy();
  const found = document.getElementById(id!);
  expect(found).not.toBeNull();
  return found!;
}

/** React listens for `input` on its own value setter, not on `.value = …`. */
async function type(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("capDiscussion", () => {
  it("leaves anything within the limit exactly as it was typed", () => {
    expect(capDiscussion("a short note")).toBe("a short note");
    const atCap = "x".repeat(MAX_DISCUSSION_CHARS);
    expect(capDiscussion(atCap)).toBe(atCap);
  });

  it("clips past it, to the host's own number", () => {
    expect([...capDiscussion("x".repeat(MAX_DISCUSSION_CHARS + 500))].length).toBe(
      MAX_DISCUSSION_CHARS,
    );
  });

  it("counts codepoints, the way the host counts them", () => {
    // `String.length` is UTF-16 code units, so a message of astral-plane
    // characters would be cut at half the limit the host actually applies —
    // and the counter beside it would have been describing a different rule
    // from the one doing the cutting. `cap_discussion` takes `chars()`.
    //
    // Deliberately *past* the cap and not at it: a fixture of exactly
    // `MAX_DISCUSSION_CHARS` emoji returns early on the length check and never
    // reaches the slice, which is how the first version of this test passed
    // against a `text.slice(…)` that cut 4000 emoji down to 2000.
    const emoji = "🙂".repeat(MAX_DISCUSSION_CHARS + 100);
    expect([...capDiscussion(emoji)].length).toBe(MAX_DISCUSSION_CHARS);
  });
});

describe("the composer surfaces the limit", () => {
  it("describes the box by a counter that is always there for a screen reader", async () => {
    await render(stallingClient([]));
    expect(counter().textContent).toContain(`of ${MAX_DISCUSSION_CHARS} characters`);
  });

  it("stays out of the way for a one-line note", async () => {
    await render(stallingClient([]));
    // A permanent "0 of 4000" under every note is noise that teaches nobody
    // anything, so it is hidden until the count starts to matter.
    expect(counter().className).toContain("sr-only");
  });

  it("shows itself as the limit comes into range", async () => {
    await render(stallingClient([]));
    await type(textarea(), "x".repeat(MAX_DISCUSSION_CHARS - 10));
    expect(counter().className).not.toContain("sr-only");
    expect(counter().textContent).toContain(`${MAX_DISCUSSION_CHARS - 10} of`);
  });

  it("refuses the overflow in the box rather than letting the host eat it", async () => {
    await render(stallingClient([]));
    await type(textarea(), "x".repeat(MAX_DISCUSSION_CHARS + 200));
    expect([...textarea().value].length).toBe(MAX_DISCUSSION_CHARS);
    expect(counter().textContent).toContain("the most a message can hold");
  });
});

describe("withdrawing a message has an in-flight state", () => {
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

  it("sends one DELETE per confirmation, not one per click", async () => {
    const seen: string[] = [];
    await render(stallingClient(seen));
    await withdraw();
    expect(seen).toHaveLength(1);

    // The request never settles, so this is the repeat-click window exactly.
    const trigger = container.querySelector(
      '[data-testid="discussion-redact"]',
    ) as HTMLButtonElement;
    expect(trigger.disabled).toBe(true);
    await act(async () => trigger.click());
    expect(seen).toHaveLength(1);
  });

  it("shows which row is being withdrawn", async () => {
    await render(stallingClient([]));
    await withdraw();
    const trigger = container.querySelector('[data-testid="discussion-redact"]')!;
    expect(trigger.getAttribute("data-busy")).toBe("true");
    expect(trigger.querySelector(".animate-spin")).not.toBeNull();
  });

  it("leaves the control alone until something is actually in flight", async () => {
    await render(stallingClient([]));
    const trigger = container.querySelector(
      '[data-testid="discussion-redact"]',
    ) as HTMLButtonElement;
    expect(trigger.disabled).toBe(false);
    expect(trigger.getAttribute("data-busy")).toBeNull();
  });
});
