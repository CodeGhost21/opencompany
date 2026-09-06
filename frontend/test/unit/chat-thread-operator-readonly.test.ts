// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TeamMember } from "@/lib/team";
import { ThreadPanel } from "@/views/chat/ThreadPanel";
import { operatorChannelFrom } from "@/views/chat/model";

/**
 * Issue #1757 follow-up (codex + CodeRabbit review on the Operator channel
 * PR). The main composer already disables on `readOnly` — `Boolean(channel?.
 * system)` — but `ThreadPanel` renders its own composer and used to disable
 * it only while `sending`. Opening a durable Operator report as a thread and
 * replying there reached `onSend` (and would have reached `client.chat`)
 * before the server's read-only guard finally refused it, after the operator
 * had already written and submitted the reply.
 *
 * Issue #1757 rework: the Operator channel is its own surface now (`GET
 * {scope}/operator-channel`), not an entry `list_desks` returns, so the
 * fixture channel is built through `operatorChannelFrom` — the same
 * projection `ChatView` uses — rather than a hand-rolled literal.
 *
 * # The read-only answer changed: no composer, not a disabled one
 *
 * The first fix left the composer on screen and disabled, with the
 * placeholder "This channel is read-only". That still reads as a claim that
 * replying is a thing you do here — a textarea, an `@` button, a paperclip, a
 * formatting toggle and a Send button, under a notice that has just said
 * there is nothing to reply to. The panel now renders the notice and **no
 * composer at all**.
 *
 * So these assert absence, not disabledness: no textarea, no Send button, no
 * mention/attach/formatting controls, and a notice in their place. The
 * "never reaches `onSend`" pins that carried the old fix are subsumed — there
 * is no control left to click or press Enter in — and the writable cases
 * below are what keeps this from being satisfiable by a panel that renders
 * nothing at all.
 */

const CHANNEL = operatorChannelFrom({
  id: "operator",
  name: "Operator",
  description: "Workflow reports and notifications",
});

const MEMBERS: TeamMember[] = [];

let container: HTMLDivElement;
let root: Root;
let sent: ReturnType<typeof vi.fn>;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  sent = vi.fn();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(readOnly: boolean | undefined) {
  await act(async () => {
    root.render(
      createElement(ThreadPanel, {
        channel: CHANNEL,
        members: MEMBERS,
        parent: { id: "p", from: "company", text: "nightly report", at: 0 },
        replies: [],
        sending: false,
        readOnly,
        onSend: sent,
        onClose: vi.fn(),
      }),
    );
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

describe("thread composer on a read-only channel (issue #1757)", () => {
  it("renders no composer at all for a read-only channel", async () => {
    await render(true);

    expect(textarea()).toBeNull();
    expect(sendButton()).toBeNull();
  });

  it("renders none of the composer's affordances either", async () => {
    await render(true);

    // Each of these is a separate claim that some action exists here. A
    // disabled one is still that claim, so absence is what is asserted.
    for (const label of ["Mention someone", "Attach a file", "Formatting"]) {
      expect(container.querySelector(`[aria-label="${label}"]`)).toBeNull();
    }
    // The keyboard hint describes a send there is no longer any way to make.
    expect(container.textContent).not.toContain("to send");
  });

  it("puts the explanation in the space the composer used to occupy", async () => {
    await render(true);

    const notice = container.querySelector('[data-testid="thread-read-only-notice"]');
    expect(notice).not.toBeNull();
    expect(notice?.getAttribute("role")).toBe("status");
    expect(notice?.textContent).toContain("There is nothing to reply to here");
    // It is the last thing in the panel: the composer is not below it, hidden
    // or otherwise. `lastElementChild` fails the moment one is rendered again.
    expect(container.querySelector("aside")?.lastElementChild).toBe(notice);
  });

  it("keeps the thread composer working on an ordinary channel", async () => {
    await render(false);
    await type("on it");

    expect(textarea().placeholder).toBe("Reply…");
    expect(sendButton().disabled).toBe(false);
    expect(container.querySelector('[data-testid="thread-read-only-notice"]')).toBeNull();

    await act(async () => sendButton().click());
    expect(sent).toHaveBeenCalledTimes(1);
    expect(sent).toHaveBeenLastCalledWith("on it", undefined, undefined, undefined);
  });

  it("defaults to the writable behaviour when readOnly is omitted", async () => {
    await render(undefined);
    await type("on it");
    expect(textarea().placeholder).toBe("Reply…");
    expect(sendButton().disabled).toBe(false);
  });
});
