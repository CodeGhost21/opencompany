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

function approveButtons() {
  return [...container.querySelectorAll("button")].filter((b) =>
    (b.textContent ?? "").includes("Approv"),
  ) as HTMLButtonElement[];
}

/**
 * Codex #3906594069: a second card can be dispatched into an already-open
 * thread before the first one settles, leaving two distinct in-review cards
 * live in the same thread. Before this, only the newest ({@link reviewTaskId})
 * had an Approve control at all — the older card's pill rendered by `Line`
 * had none, so the operator had to settle the newer card first. Each entry in
 * `additionalReviewAnchors` now gets its own notice + Approve button.
 */
describe("other in-review cards in the same thread (Codex #3906594069)", () => {
  it("gives the older card its own Approve control alongside the newest one", async () => {
    const onReviewCard = vi.fn();
    await render({
      reviewTaskId: "t-2",
      onReviewCard,
      additionalReviewAnchors: [{ taskId: "t-1", anchorId: "relay-1" }],
    });

    const buttons = approveButtons();
    expect(buttons).toHaveLength(2);

    await act(async () => buttons[1]?.click());
    expect(onReviewCard).toHaveBeenCalledWith("t-1", "approve");

    await act(async () => buttons[0]?.click());
    expect(onReviewCard).toHaveBeenCalledWith("t-2", "approve");
  });

  it("disables only the additional card's own control while its verdict is in flight, not the newest's", async () => {
    await render({
      reviewTaskId: "t-2",
      onReviewCard: vi.fn(),
      additionalReviewAnchors: [{ taskId: "t-1", anchorId: "relay-1" }],
      reviewingTaskId: new Set(["t-1"]),
    });

    const buttons = approveButtons();
    expect(buttons).toHaveLength(2);
    expect(buttons[0]?.disabled).toBe(false);
    expect(buttons[1]?.disabled).toBe(true);
    expect(buttons[1]?.textContent).toContain("Approving");
  });

  it("renders nothing extra when there is only the one card", async () => {
    await render({ reviewTaskId: "t-1", onReviewCard: vi.fn() });
    expect(approveButtons()).toHaveLength(1);
  });
});
