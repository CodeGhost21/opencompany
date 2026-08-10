// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectionReady,
  embeddedHost,
  forgetConnection,
  registerConnection,
  resetDesktopRegistrations,
} from "@/api/transport/desktop";
import { addConnection, removeConnection, resetConnections } from "@/connections/registry";

/**
 * The desktop's missing half.
 *
 * `ProxyTransport` addresses a host by connection id and the Rust core resolves
 * that id against its own registry. Nothing told the core about any connection,
 * so every proxied request answered `no such connection: <id>` — a desktop that
 * could not complete one round trip. These tests are about that handshake:
 * every connection the console holds must reach the core, and no request may
 * overtake its own registration.
 */

interface Invocation {
  command: string;
  args: Record<string, unknown>;
}

let calls: Invocation[] = [];

function installBridge(options: { embedded?: unknown } = {}): void {
  calls = [];
  (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
    invoke: (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      if (command === "oc_embedded") return Promise.resolve(options.embedded ?? null);
      return Promise.resolve(undefined);
    },
    Channel: class {
      onmessage: ((message: string) => void) | null = null;
    },
  };
}

beforeEach(() => {
  resetConnections();
  resetDesktopRegistrations();
  window.localStorage.clear();
  installBridge();
});

afterEach(() => {
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
  vi.restoreAllMocks();
});

describe("registering connections with the core", () => {
  it("announces every host the console adds", async () => {
    const id = addConnection({ baseUrl: "https://acme.test" });
    await connectionReady(id);

    const connect = calls.filter((c) => c.command === "oc_connect");
    expect(connect).toHaveLength(1);
    expect(connect[0].args).toMatchObject({
      connectionId: id,
      baseUrl: "https://acme.test",
    });
  });

  it("forwards a platform bearer, so the core can present it", async () => {
    const id = addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "platform", token: "bearer-1" },
    });
    await connectionReady(id);

    expect(calls.at(-1)?.args).toMatchObject({ platformToken: "bearer-1" });
  });

  it("does not forward a device ref as if it were a token", async () => {
    // `ref` names a keychain entry; nothing resolves one yet. Passing it
    // through would authenticate as the literal handle, which is worse than
    // arriving unauthenticated — the host would reject it with no clue why.
    const id = addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "device", ref: "keychain-handle-1" },
    });
    await connectionReady(id);

    const connect = calls.filter((c) => c.command === "oc_connect").at(-1);
    expect(connect?.args.deviceSession).toBeNull();
    expect(JSON.stringify(connect?.args)).not.toContain("keychain-handle-1");
  });

  it("re-announces when a connection adopts a new credential", async () => {
    const id = addConnection({ baseUrl: "https://acme.test" });
    await connectionReady(id);
    addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "platform", token: "bearer-2" },
    });
    await connectionReady(id);

    const connect = calls.filter((c) => c.command === "oc_connect");
    expect(connect).toHaveLength(2);
    expect(connect[1].args).toMatchObject({ connectionId: id, platformToken: "bearer-2" });
  });

  it("drops a removed host from the core", async () => {
    const id = addConnection({ baseUrl: "https://acme.test" });
    await connectionReady(id);
    removeConnection(id);

    expect(calls.filter((c) => c.command === "oc_disconnect")).toEqual([
      { command: "oc_disconnect", args: { connectionId: id } },
    ]);
  });

  it("keeps a failed registration from resurfacing on every later request", async () => {
    // The promise is awaited by the transport on each call, so a rejected one
    // stored in the map would throw again for every request forever. The
    // request that follows fails on its own merits instead.
    (window as unknown as { __TAURI__: { invoke: unknown } }).__TAURI__.invoke = () =>
      Promise.reject(new Error("core is gone"));
    vi.spyOn(console, "error").mockImplementation(() => {});

    await expect(registerConnection("c1", "https://acme.test")).resolves.toBeUndefined();
    await expect(connectionReady("c1")).resolves.toBeUndefined();
  });

  it("is inert in a browser, where there is no bridge at all", async () => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
    await expect(registerConnection("c1", "https://acme.test")).resolves.toBeUndefined();
    await expect(embeddedHost()).resolves.toBeNull();
    expect(() => forgetConnection("c1")).not.toThrow();
  });
});

describe("the embedded host", () => {
  it("comes back as an addressable base url", async () => {
    installBridge({
      embedded: { baseUrl: "http://127.0.0.1:52341", dataDir: "/tmp/oc" },
    });
    await expect(embeddedHost()).resolves.toEqual({
      baseUrl: "http://127.0.0.1:52341",
      dataDir: "/tmp/oc",
    });
  });

  it("is null when the core could not start one", async () => {
    // Most often the data root is already held — by another window, or by an
    // `opencompany serve` in a terminal. The desktop still holds remote hosts,
    // so this is a row the console renders, not a reason to have no console.
    installBridge({ embedded: null });
    await expect(embeddedHost()).resolves.toBeNull();
  });
});
