// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { useTyping } from "@/hooks/use-typing";
import { TYPING_RECEIVE_TTL_MS } from "@/lib/awareness";

function fakeClient(): OpenCompanyClient {
  return { typing: vi.fn(async () => undefined) } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let lastState: ReturnType<typeof useTyping> | null;

function Probe({ client }: { client: OpenCompanyClient }) {
  lastState = useTyping(client, "acme");
  return null;
}

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(Probe, { client }));
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  lastState = null;
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
  vi.useRealTimers();
});

const frame = (atMillis: number) => ({
  userId: "u-ada",
  chatId: "engineering",
  atMillis,
});

/**
 * Regression coverage for the bug the e2e suite caught once `typingExpiry`
 * stopped capping expiry at `frame.atMillis + TTL` (necessary to fix the
 * clock-skew finding — see `awareness.ts`): a redelivered frame — an SSE
 * reconnect replaying the last event a mocked or buffered transport still
 * holds is the real case — must not indefinitely renew an indicator that no
 * genuine new ping actually bought.
 */
describe("useTyping dedup", () => {
  it("shows a typer on the first frame", async () => {
    const client = fakeClient();
    await mount(client);
    await act(async () => {
      lastState?.onFrame(frame(1_000));
    });
    expect(lastState?.typers.map((t) => t.userId)).toEqual(["u-ada"]);
  });

  it("ignores an exact replay of the same frame rather than renewing it", async () => {
    vi.useFakeTimers();
    const client = fakeClient();
    await mount(client);

    await act(async () => {
      lastState?.onFrame(frame(1_000));
    });
    const firstExpiry = lastState?.typers[0]?.expiresAt;

    // Time passes, but well inside the TTL, and the exact same frame (same
    // `atMillis`) arrives again — the shape of a reconnect replaying its last
    // buffered event, not a fresh keystroke.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    await act(async () => {
      lastState?.onFrame(frame(1_000));
    });
    expect(lastState?.typers[0]?.expiresAt).toBe(firstExpiry);

    // And the indicator still expires on the original schedule rather than
    // lingering because of the replay.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(TYPING_RECEIVE_TTL_MS);
    });
    expect(lastState?.typers).toEqual([]);
  });

  it("treats a frame with a newer atMillis as a genuine renewal", async () => {
    vi.useFakeTimers();
    const client = fakeClient();
    await mount(client);

    await act(async () => {
      lastState?.onFrame(frame(1_000));
    });
    const firstExpiry = lastState?.typers[0]?.expiresAt;

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    await act(async () => {
      lastState?.onFrame(frame(3_000));
    });
    expect(lastState?.typers[0]?.expiresAt).not.toBe(firstExpiry);
    expect(lastState?.typers.map((t) => t.userId)).toEqual(["u-ada"]);
  });
});
