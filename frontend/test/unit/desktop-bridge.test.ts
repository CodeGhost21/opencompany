// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectionReady,
  embeddedHost,
  forgetConnection,
  pairDevice,
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

  it("sends no device material at all, because the core looks it up", async () => {
    // `oc_connect` takes no device token. The core resolves a paired session
    // from the keychain by connection id, so there is nothing for the console
    // to pass — which is what makes it impossible, rather than merely
    // impolite, for the webview to choose one.
    const id = addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "device", ref: "device-abc" },
    });
    await connectionReady(id);

    const connect = calls.filter((c) => c.command === "oc_connect").at(-1);
    expect(connect?.args).not.toHaveProperty("deviceSession");
    expect(JSON.stringify(connect?.args)).not.toContain("device-abc");
  });

  it("asks the core to pair, and passes the code and label through", async () => {
    // What this CAN assert from the console: the command shape. Whether a token
    // comes back is decided in Rust by `PairedDevice`, which has no token
    // field — a mock here proves nothing about that, and the earlier version of
    // this test omitted `token` from its own fixture and then asserted the
    // absence of it, which is a test checking itself. The real guarantee is the
    // Rust-side `a_paired_device_carries_no_token`.
    (window as unknown as { __TAURI__: { invoke: unknown } }).__TAURI__.invoke = (
      command: string,
      args: Record<string, unknown>,
    ) => {
      calls.push({ command, args });
      return Promise.resolve({ company: "acme", deviceId: "dev-1", expiresAtMillis: 1 });
    };

    const device = await pairDevice("conn-1", "https://acme.test", "the-code", "Ada's Mac");
    expect(device).toEqual({ company: "acme", deviceId: "dev-1", expiresAtMillis: 1 });
    expect(calls.at(-1)).toMatchObject({
      command: "oc_pair_device",
      args: {
        connectionId: "conn-1",
        baseUrl: "https://acme.test",
        code: "the-code",
        label: "Ada's Mac",
      },
    });
  });

  it("does not disconnect before a registration still in flight has landed", async () => {
    // `oc_connect` resolving after `oc_disconnect` landed would leave the
    // connection registered in the core while the console believes it is gone.
    const order: string[] = [];
    let releaseConnect: (() => void) | undefined;
    (window as unknown as { __TAURI__: { invoke: unknown } }).__TAURI__.invoke = (
      command: string,
    ) => {
      if (command === "oc_connect") {
        return new Promise<void>((resolve) => {
          releaseConnect = () => {
            order.push("oc_connect");
            resolve();
          };
        });
      }
      order.push(command);
      return Promise.resolve();
    };

    const pending = registerConnection("conn-race", "https://acme.test");
    forgetConnection("conn-race");
    // Nothing has gone out yet: the connect is parked, so the disconnect is too.
    expect(order).toEqual([]);

    releaseConnect?.();
    await pending;
    await connectionReady("conn-race");

    expect(order).toEqual(["oc_connect", "oc_disconnect"]);
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
    // The disconnect is sequenced behind any registration still in flight, so
    // it is issued on a later tick rather than synchronously.
    await connectionReady(id);

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
