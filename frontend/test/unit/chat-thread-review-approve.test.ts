// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import type { TeamMember } from "@/lib/team";
import { ThreadPanel } from "@/views/chat/ThreadPanel";
import type { Channel } from "@/views/chat/model";

/**
 * CodeRabbit #3905116857: a card that settles inside an already-open thread
 * folds its settle pill into a plain reply line — `ThreadPanel`'s own `Line`,
 * not `MessageRow` — which has no Approve button anywhere. Before this, the
 * only way to approve such a card was to close the thread and find the pill
 * back in the channel. The reviewing notice at the foot of the panel is now
 * the Approve surface for that case.
 */

const CHANNEL: Channel = {
  id: "engineering",
  name: "engineering",
  voice: "Engineering",
  kind: "channel",
  purpose: "",
};

const MEMBERS: TeamMember[] = [];

const PARENT: ChatMessage = { id: "p", from: "you", text: "kick off the writeup", at: 0 };

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
});

async function render(props: Partial<Parameters<typeof ThreadPanel>[0]> = {}) {
  await act(async () => {
    root.render(
      createElement(ThreadPanel, {
        channel: CHANNEL,
        members: MEMBERS,
        parent: PARENT,
        replies: [],
        sending: false,
        onSend: vi.fn(),
        onClose: vi.fn(),
        reviewing: true,
        ...props,
      }),
    );
  });
}

function approveButton() {
  return [...container.querySelectorAll("button")].find((b) =>
    (b.textContent ?? "").includes("Approv"),
  ) as HTMLButtonElement | undefined;
}

describe("the reviewing notice's Approve control (CodeRabbit #3905116857)", () => {
  it("renders no Approve control when the card id or the handler is missing", async () => {
    await render();
    expect(container.textContent).toContain("This card is ready for review");
    expect(approveButton()).toBeUndefined();
  });

  it("offers Approve beside the notice once both the card id and the handler are given", async () => {
    const onReviewCard = vi.fn();
    await render({ reviewTaskId: "t-1", onReviewCard });

    const button = approveButton();
    expect(button).toBeTruthy();
    expect(button?.disabled).toBe(false);

    await act(async () => button?.click());
    expect(onReviewCard).toHaveBeenCalledWith("t-1", "approve");
  });

  it("disables the control while the verdict is in flight", async () => {
    await render({ reviewTaskId: "t-1", onReviewCard: vi.fn(), reviewInFlight: true });

    const button = approveButton();
    expect(button?.disabled).toBe(true);
    expect(button?.textContent).toContain("Approving");
  });

  it("renders nothing extra when the thread is not a review surface at all", async () => {
    await render({ reviewing: false, reviewTaskId: "t-1", onReviewCard: vi.fn() });

    expect(container.textContent).not.toContain("This card is ready for review");
    expect(approveButton()).toBeUndefined();
  });
});
