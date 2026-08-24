// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import { usePresence } from "@/hooks/use-presence";
import { PRESENCE_HEARTBEAT_MS } from "@/lib/awareness";

function fakeClient(over: Partial<OpenCompanyClient> = {}): OpenCompanyClient {
  return {
    presence: vi.fn(async () => ({ people: [] })),
    announcePresence: vi.fn(async () => undefined),
    disconnectPresenceBeacon: vi.fn(),
    ...over,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let lastState: ReturnType<typeof usePresence> | null;

function Probe({ client }: { client: OpenCompanyClient }) {
  lastState = usePresence(client, "acme");
  return null;
}

function mount(client: OpenCompanyClient) {
  return act(async () => {
    root.render(createElement(Probe, { client }));
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  sessionStorage.clear();
  localStorage.clear();
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

/**
 * Regression coverage for the three findings fixed alongside the per-tab
 * lease: a stable per-tab id gets attached to every request, a 404 disables
 * the feature but nothing else does, and a steady peer's snapshot is
 * refreshed on the heartbeat rather than only once at mount.
 */
describe("usePresence", () => {
  it("attaches the same consoleId to every announce and to the disconnect beacon", async () => {
    const client = fakeClient();
    await mount(client);

    const announce = client.announcePresence as ReturnType<typeof vi.fn>;
    expect(announce).toHaveBeenCalled();
    const [, , consoleId] = announce.mock.calls[0] as [string, string, string];
    expect(typeof consoleId).toBe("string");
    expect(consoleId.length).toBeGreaterThan(0);

    await act(async () => {
      lastState?.choose("offline");
    });
    const beacon = client.disconnectPresenceBeacon as ReturnType<typeof vi.fn>;
    expect(beacon).toHaveBeenCalledWith("acme", consoleId);
  });

  it("keeps the same consoleId across a remount of the same tab", async () => {
    const clientA = fakeClient();
    await mount(clientA);
    const idA = (clientA.announcePresence as ReturnType<typeof vi.fn>).mock.calls[0][2];

    await act(async () => {
      root.unmount();
    });
    root = createRoot(container);
    const clientB = fakeClient();
    await mount(clientB);
    const idB = (clientB.announcePresence as ReturnType<typeof vi.fn>).mock.calls[0][2];

    // Same tab, so the same `sessionStorage`: the id survives a remount,
    // matching the server's expectation that reloading a tab renews one
    // lease rather than doubling it onto two different keys.
    expect(idB).toBe(idA);
  });

  it("mints a different consoleId for what a new tab's sessionStorage looks like", async () => {
    const clientA = fakeClient();
    await mount(clientA);
    const idA = (clientA.announcePresence as ReturnType<typeof vi.fn>).mock.calls[0][2];

    await act(async () => {
      root.unmount();
    });
    // A second tab never shares the first tab's `sessionStorage` — simulated
    // here by clearing it, since jsdom gives every test one global instance.
    sessionStorage.clear();
    root = createRoot(container);
    const clientB = fakeClient();
    await mount(clientB);
    const idB = (clientB.announcePresence as ReturnType<typeof vi.fn>).mock.calls[0][2];

    expect(idB).not.toBe(idA);
  });

  it("disables presence on a 404 (the route does not exist on this host)", async () => {
    const client = fakeClient({
      announcePresence: vi.fn(async () => {
        throw new ApiError(404, "not_found", "no such route");
      }),
    });
    await mount(client);
    expect(lastState?.supported).toBe(false);
  });

  it("does not disable presence on a transient failure", async () => {
    const client = fakeClient({
      announcePresence: vi.fn(async () => {
        throw new ApiError(0, "network_error", "cannot reach the company host");
      }),
    });
    await mount(client);
    expect(lastState?.supported).toBe(true);
  });

  it("does not disable presence when the initial snapshot read fails transiently", async () => {
    const client = fakeClient({
      presence: vi.fn(async () => {
        throw new ApiError(503, "unavailable", "quiescing");
      }),
    });
    await mount(client);
    expect(lastState?.supported).toBe(true);
  });

  it("re-reads the snapshot on every heartbeat, not just at mount", async () => {
    vi.useFakeTimers();
    const client = fakeClient();
    await act(async () => {
      root.render(createElement(Probe, { client }));
    });
    const presence = client.presence as ReturnType<typeof vi.fn>;
    const callsAtMount = presence.mock.calls.length;
    expect(callsAtMount).toBeGreaterThan(0);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(PRESENCE_HEARTBEAT_MS);
    });
    expect(presence.mock.calls.length).toBeGreaterThan(callsAtMount);
  });
});
