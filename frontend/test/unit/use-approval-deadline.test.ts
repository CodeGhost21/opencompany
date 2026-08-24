// @vitest-environment jsdom

import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { useApprovalDeadline } from "@/hooks/use-approval-deadline";

let container: HTMLDivElement;
let root: Root;

function probe(value: () => number): () => number {
  let latest: number | undefined;
  const Probe = (): ReactElement | null => {
    latest = value();
    return null;
  };
  act(() => root.render(createElement(Probe)));
  return () => {
    if (latest === undefined) throw new Error("the hook never rendered");
    return latest;
  };
}

/** Flush microtasks so the effect fires and its promise settles. */
async function flush() {
  for (let i = 0; i < 10; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
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
});

describe("useApprovalDeadline", () => {
  it("returns the default 24 when an older host omits approvalTtlHours", async () => {
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async () => ({
        mode: "supervised",
        // No approvalTtlHours — an older host.
      }),
    } as unknown as OpenCompanyClient;

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(24);
  });

  it("returns the default 24 when the host read fails", async () => {
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async () => {
        throw new Error("network down");
      },
    } as unknown as OpenCompanyClient;

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(24);
  });

  it("returns the read value when the host does respond", async () => {
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async () => ({
        approvalTtlHours: 48,
        mode: "supervised",
      }),
    } as unknown as OpenCompanyClient;

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(48);
  });

  it("falls back to 24 when the host returns null", async () => {
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async () => ({
        approvalTtlHours: null,
        mode: "supervised",
      }),
    } as unknown as OpenCompanyClient;

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(24);
  });

  it("resets on a company switch when the new read fails", async () => {
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: vi.fn(async () => ({
        approvalTtlHours: 48,
        mode: "supervised",
      })),
    } as unknown as OpenCompanyClient;

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(48);

    // Now switch company with a failing read. The hook must reset before the
    // read and leave 24 (the default), not 48 from the previous company.
    const failing = {
      ...client,
      scopeFor: () => "/api/v1/other",
      get: async () => {
        throw new Error("unreachable");
      },
    } as unknown as OpenCompanyClient;

    act(() => {
      hours = probe(() => useApprovalDeadline(failing, "other"));
    });
    await flush();
    expect(hours()).toBe(24);
  });

  it("refreshes the deadline when the policy changes while mounted", async () => {
    vi.useFakeTimers();
    const get = vi.fn(async () => ({
      approvalTtlHours: 24,
      mode: "supervised",
    }));
    const client = {
      scopeFor: () => "/api/v1/acme",
      get,
    } as unknown as OpenCompanyClient & { get: typeof get };

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    await flush();
    expect(hours()).toBe(24);

    // Another operator raises the TTL on the host while the view stays
    // mounted. The next poll tick must bring the new value into the sentence
    // — a mount-only read would keep advertising 24 until a remount.
    get.mockResolvedValue({ approvalTtlHours: 72, mode: "supervised" });
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await flush();
    expect(hours()).toBe(72);
  });

  it("discards an out-of-order response from a slower poll", async () => {
    vi.useFakeTimers();
    // Two reads started, two deferred responses. A is the older request, B the
    // newer one, and they resolve in the reverse order they were issued — a
    // poll that takes longer than the interval, or a visibility-triggered
    // refresh racing a request started just before the tab hid.
    let resolveA!: (v: unknown) => void;
    let resolveB!: (v: unknown) => void;
    const get = vi
      .fn()
      .mockReturnValueOnce(
        new Promise((res) => {
          resolveA = res;
        }),
      )
      .mockReturnValueOnce(
        new Promise((res) => {
          resolveB = res;
        }),
      );
    const client = {
      scopeFor: () => "/api/v1/acme",
      get,
    } as unknown as OpenCompanyClient & { get: typeof get };

    let hours!: () => number;
    act(() => {
      hours = probe(() => useApprovalDeadline(client, "acme"));
    });
    // A is in flight; nothing has resolved yet.
    await flush();
    expect(hours()).toBe(24);

    // The next poll tick starts B.
    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    await flush();

    // B — the newer read — lands first with the raised deadline.
    await act(async () => {
      resolveB({ approvalTtlHours: 72, mode: "supervised" });
    });
    await flush();
    expect(hours()).toBe(72);

    // A — the older read — resolves last with the stale deadline. The hook must
    // discard it: the header already advertises what the host enforces, and
    // letting A land would repaint a stale TTL until the next poll corrects it.
    await act(async () => {
      resolveA({ approvalTtlHours: 24, mode: "supervised" });
    });
    await flush();
    expect(hours()).toBe(72);
  });
});