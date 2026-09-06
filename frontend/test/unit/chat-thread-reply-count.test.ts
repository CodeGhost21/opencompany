// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import type { TeamMember } from "@/lib/team";
import { ThreadPanel } from "@/views/chat/ThreadPanel";
import { buildTimeline, inlineReplyIds, type Channel } from "@/views/chat/model";

/**
 * One capped turn, rendered once.
 *
 * A turn that runs out of steps emits two replies parented to the operator's
 * message — the agent's partial write-up, then the host's
 * `iteration_cap_pause_notice`. Promoting the first inline broke three things
 * at once, all measured on a real turn:
 *
 *   * the write-up rendered inline **and** in the panel, because
 *     `repliesInThread` walks the parent chain and knows nothing of what was
 *     promoted — one message, two surfaces;
 *   * the chip counted 1 (promoted filtered out) while the panel counted 2
 *     (nothing filtered), so one thread reported two sizes at once;
 *   * and the panel could not drop the promoted reply to fix either, because
 *     the notice beneath it opens "The reply above is a pause".
 *
 * Both halves are fixed here: promotion stops when the runtime spoke more than
 * once, so nothing is split across surfaces, and the panel counts the same set
 * the chip does, so the two can never disagree again.
 */

const CHANNEL: Channel = {
  id: "engineering",
  name: "engineering",
  voice: "Engineering",
  kind: "channel",
  purpose: "",
};

const MEMBERS: TeamMember[] = [];

const ROOT: ChatMessage = { id: "p", from: "you", text: "group the issues", at: 0 };
/** The agent's partial write-up. */
const WRITE_UP: ChatMessage = {
  id: "r1",
  parentId: "p",
  from: "company",
  text: "here is what I have so far",
  at: 1,
};
/** The host's pause notice — `SYSTEM_AUTHOR` projects to `from: "system"`. */
const PAUSE: ChatMessage = {
  id: "r2",
  parentId: "p",
  from: "system",
  text: "The reply above is a pause, not a finished answer",
  at: 2,
};

const CAPPED_TURN = [ROOT, WRITE_UP, PAUSE];

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
  vi.restoreAllMocks();
});

async function render(over: Partial<Parameters<typeof ThreadPanel>[0]> = {}) {
  await act(async () => {
    root.render(
      createElement(ThreadPanel, {
        channel: CHANNEL,
        members: MEMBERS,
        parent: ROOT,
        replies: [WRITE_UP, PAUSE],
        sending: false,
        onSend: vi.fn(),
        onClose: vi.fn(),
        ...over,
      }),
    );
  });
}

describe("a capped turn's two replies", () => {
  it("promotes neither, so no message renders on two surfaces", () => {
    const entries = buildTimeline(CAPPED_TURN, CHANNEL, MEMBERS);

    // Only the root is laid out in the channel — the write-up is not also
    // inline, so opening the thread cannot show it a second time.
    expect(entries.map((e) => e.message.id)).toEqual(["p"]);
    expect(inlineReplyIds(CAPPED_TURN).size).toBe(0);
  });

  it("counts the same thread size on the chip and in the panel", async () => {
    const entries = buildTimeline(CAPPED_TURN, CHANNEL, MEMBERS);
    const chip = entries.find((e) => e.message.id === "p")?.replies ?? [];
    expect(chip.map((r) => r.id)).toEqual(["r1", "r2"]);

    await render({ inlineReplyIds: inlineReplyIds(CAPPED_TURN) });

    expect(container.textContent).toContain(`${chip.length} replies`);
  });

  it("keeps the notice's referent, so 'the reply above' has one", async () => {
    await render({ inlineReplyIds: inlineReplyIds(CAPPED_TURN) });

    expect(container.textContent).toContain("here is what I have so far");
    expect(container.textContent).toContain("The reply above is a pause");
  });
});

describe("the panel's count", () => {
  /**
   * The operator writing again is not the runtime speaking twice, so the lone
   * answer is still promoted — and the panel must then leave it out of the
   * count, or the chip ("1 reply", promoted filtered) and the panel would
   * disagree exactly as they did before.
   */
  it("excludes a promoted reply, matching the chip", async () => {
    const followUp: ChatMessage = { id: "r2", parentId: "p", from: "you", text: "and by when?", at: 2 };
    const messages = [ROOT, WRITE_UP, followUp];

    const entries = buildTimeline(messages, CHANNEL, MEMBERS);
    const chip = entries.find((e) => e.message.id === "p")?.replies ?? [];
    // The answer went inline; only the operator's own follow-up is left to open.
    expect(chip.map((r) => r.id)).toEqual(["r2"]);

    await render({ replies: [WRITE_UP, followUp], inlineReplyIds: inlineReplyIds(messages) });

    expect(container.textContent).toContain(`${chip.length} reply`);
    expect(container.textContent).not.toContain("2 replies");
  });

  it("counts every reply when the caller cannot say what was promoted", async () => {
    // No `inlineReplyIds`: a panel that does not know what the channel showed
    // must not silently under-count. This is what every caller did before the
    // prop existed.
    await render();

    expect(container.textContent).toContain("2 replies");
  });
});
