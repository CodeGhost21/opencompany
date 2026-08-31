// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TurnStep } from "@/api/types";
import {
  ChatLiveReceipt,
  formatElapsed,
  RECEIPT_STALL_AFTER_MS,
  type ChatReceipt,
} from "@/views/chat/ChatLiveReceipt";
import type { Channel } from "@/views/chat/model";

/**
 * Coverage for the chat live receipt (issue #1934) — the row that fills the gap
 * between a sent instruction and its reply. The unit runner has no
 * testing-library, so this renders through `react-dom/client` directly (the
 * same shape `use-typing.test.ts` and `inference-model-picker.test.ts` use) and
 * drives its self-contained 1s clock with fake timers.
 */

const BASE = 1_700_000_000_000;

function channel(overrides: Partial<Channel> = {}): Channel {
  return {
    id: "engineering",
    name: "engineering",
    voice: "Engineering",
    kind: "channel",
    purpose: "",
    tone: "sky",
    ...overrides,
  };
}

function runningStep(label: string): TurnStep {
  return { kind: "tool_call", status: "running", label };
}

let container: HTMLDivElement;
let root: Root;

async function render(props: {
  channel?: Channel;
  receipt: ChatReceipt;
  agentNames?: Record<string, string>;
  steps?: TurnStep[];
}) {
  await act(async () => {
    root.render(
      createElement(ChatLiveReceipt, {
        channel: props.channel ?? channel(),
        receipt: props.receipt,
        agentNames: props.agentNames,
        steps: props.steps ?? [],
      }),
    );
  });
}

function text(): string {
  return container.textContent ?? "";
}

function receiptEl(): HTMLElement {
  const el = container.querySelector<HTMLElement>('[data-testid="chat-live-receipt"]');
  if (!el) throw new Error("receipt row not rendered");
  return el;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.setSystemTime(BASE);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
  vi.useRealTimers();
});

describe("ChatLiveReceipt", () => {
  it("shows Sent with an elapsed readout when no agent and no steps", async () => {
    await render({ receipt: { startedAt: BASE - 5_000, lastFrameAt: BASE - 5_000 } });
    expect(text()).toContain("Sent");
    expect(text()).toContain("5s");
    expect(text()).not.toContain("Picked up by");
  });

  it("names the teammate once an agent id resolves", async () => {
    await render({
      receipt: { startedAt: BASE, lastFrameAt: BASE, agentId: "a-ada" },
      agentNames: { "a-ada": "Ada" },
    });
    expect(text()).toContain("Picked up by Ada");
    // A raw agent id is never rendered.
    expect(text()).not.toContain("a-ada");
  });

  it("falls back to the channel voice for an unresolvable agent id", async () => {
    await render({
      channel: channel({ voice: "Front desk" }),
      receipt: { startedAt: BASE, lastFrameAt: BASE, agentId: "a-ghost" },
      agentNames: {},
    });
    expect(text()).toContain("Picked up by Front desk");
    expect(text()).not.toContain("a-ghost");
  });

  it("shows the running step label when a step is in flight", async () => {
    await render({
      receipt: { startedAt: BASE, lastFrameAt: BASE, agentId: "a-ada" },
      agentNames: { "a-ada": "Ada" },
      steps: [runningStep("Searching the web")],
    });
    expect(text()).toContain("On step Searching the web");
  });

  it("goes stalled after 30s with no frame, and a fresh frame clears it", async () => {
    const receipt: ChatReceipt = { startedAt: BASE, lastFrameAt: BASE };
    await render({ receipt });
    expect(receiptEl().dataset.stalled).toBe("false");

    // Push the self-contained clock past the 30s stall window with no new frame.
    await act(async () => {
      vi.advanceTimersByTime(31_000);
    });
    expect(receiptEl().dataset.stalled).toBe("true");
    expect(text()).toContain("No update for 30s");

    // A fresh frame bumps lastFrameAt (a new receipt object from the shell) —
    // the stall is soft and reversible, so it clears without a remount.
    await render({ receipt: { startedAt: BASE, lastFrameAt: BASE + 31_000 } });
    expect(receiptEl().dataset.stalled).toBe("false");
    expect(text()).not.toContain("No update for 30s");
  });

  it("is already stalled at exactly the 30s threshold, not a tick after (issue #1935 review)", async () => {
    // coderabbit 3892517524: `clock - lastFrameAt > RECEIPT_STALL_AFTER_MS`
    // reads exactly 30,000ms as "not yet stalled", so the notice appeared
    // about a second late. `>=` is the fix under test.
    const receipt: ChatReceipt = { startedAt: BASE, lastFrameAt: BASE };
    await render({ receipt });

    await act(async () => {
      vi.advanceTimersByTime(RECEIPT_STALL_AFTER_MS);
    });
    expect(receiptEl().dataset.stalled).toBe("true");
    expect(text()).toContain("No update for 30s");
  });

  it("advances the elapsed readout as its own clock ticks", async () => {
    await render({ receipt: { startedAt: BASE, lastFrameAt: BASE } });
    expect(text()).toContain("0s");
    await act(async () => {
      vi.advanceTimersByTime(3_000);
    });
    expect(text()).toContain("3s");
  });
});

describe("formatElapsed", () => {
  it("renders sub-minute durations as seconds", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(5_400)).toBe("5s");
    expect(formatElapsed(59_999)).toBe("59s");
  });

  it("rolls into m:ss at a minute and pads the seconds", () => {
    expect(formatElapsed(60_000)).toBe("1:00");
    expect(formatElapsed(65_000)).toBe("1:05");
    expect(formatElapsed(605_000)).toBe("10:05");
  });

  it("floors a negative elapsed to zero", () => {
    expect(formatElapsed(-1_000)).toBe("0s");
  });
});
