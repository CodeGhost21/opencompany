// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { tauriCore } from "@/api/transport/bridge";
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

/**
 * The bridge as Tauri v2 actually injects it.
 *
 * `invoke` and `Channel` under `core`, because that is where `withGlobalTauri`
 * puts them. The fixture this suite used to install put them at the top level —
 * the **v1** shape, which Tauri v2 never produces — so 82 desktop tests passed
 * against an object that did not correspond to the runtime, while the shipped
 * bridge resolved to `null` on every command
 * ([#616](https://github.com/tinyhumansai/opencompany/issues/616)). A mock is
 * evidence only if it is the shape the runtime hands over, so "the injected
 * bridge shape" below pins this one to the real package.
 */
function installBridge(options: { embedded?: unknown } = {}): void {
  calls = [];
  (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
    core: {
      invoke: (command: string, args: Record<string, unknown> = {}) => {
        calls.push({ command, args });
        if (command === "oc_embedded") return Promise.resolve(options.embedded ?? null);
        return Promise.resolve(undefined);
      },
      Channel: class {
        onmessage: ((message: string) => void) | null = null;
      },
    },
  };
}

/**
 * The installed mock, for the tests that swap `invoke` out.
 *
 * One accessor rather than the path spelled at each site: the shape knowledge
 * that was wrong is knowledge no test should be repeating.
 */
function bridgeMock(): { invoke: unknown } {
  return (window as unknown as { __TAURI__: { core: { invoke: unknown } } }).__TAURI__.core;
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

  it("does not disconnect before a registration still in flight has landed", async () => {
    // `oc_connect` resolving after `oc_disconnect` landed would leave the
    // connection registered in the core while the console believes it is gone.
    const order: string[] = [];
    let releaseConnect: (() => void) | undefined;
    bridgeMock().invoke = (
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
    // Let the chained `oc_connect` actually be issued. `registerConnection`
    // sequences behind whatever is parked under the id, so the invoke happens a
    // microtask later rather than synchronously.
    await new Promise((resolve) => setTimeout(resolve, 0));
    // Nothing has completed yet: the connect is parked, so the disconnect is too.
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

  it("does not let a pending disconnect land on top of a fresh registration", async () => {
    // The mirror of the test above. `forgetConnection` sequences behind a
    // pending registration; if `registerConnection` did not do the same for a
    // pending disconnect, a re-register racing an in-flight `oc_disconnect`
    // could land first and then be torn down by it — the connection would be
    // registered in the console and absent from the core.
    const order: string[] = [];
    let releaseDisconnect: (() => void) | undefined;
    bridgeMock().invoke = (
      command: string,
    ) => {
      if (command === "oc_disconnect") {
        return new Promise<void>((resolve) => {
          releaseDisconnect = () => {
            order.push("oc_disconnect");
            resolve();
          };
        });
      }
      order.push(command);
      return Promise.resolve();
    };

    await registerConnection("conn-mirror", "https://acme.test");
    forgetConnection("conn-mirror");
    await new Promise((resolve) => setTimeout(resolve, 0));

    // Re-register while the disconnect is still parked, then let every
    // microtask drain WITHOUT releasing it. The second `oc_connect` must not
    // have been issued yet: that is the difference chaining makes, and without
    // it the invoke fires here and only the resolution order differs — which is
    // not something an assertion on the final order can see.
    const again = registerConnection("conn-mirror", "https://acme.test");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(order).toEqual(["oc_connect"]);

    releaseDisconnect?.();
    await again;

    // The reconnect is last, so the core ends up holding the connection.
    expect(order).toEqual(["oc_connect", "oc_disconnect", "oc_connect"]);
  });

  it("keeps a failed registration from resurfacing on every later request", async () => {
    // The promise is awaited by the transport on each call, so a rejected one
    // stored in the map would throw again for every request forever. The
    // request that follows fails on its own merits instead.
    bridgeMock().invoke = () =>
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
  it("comes back as an addressable base url, and an identity to key it on", async () => {
    // The identity is the load-bearing half. The port is ephemeral by design,
    // so a console keyed on the address recognises a new host every launch and
    // leaves the previous one's row behind, dead (#615) — this is where the
    // core's answer to "who is listening there" crosses into the console.
    installBridge({
      embedded: {
        baseUrl: "http://127.0.0.1:52341",
        dataDir: "/tmp/oc",
        instanceId: "0f9d8c7b6a5e4f3d2c1b0a9988776655",
      },
    });
    await expect(embeddedHost()).resolves.toEqual({
      baseUrl: "http://127.0.0.1:52341",
      dataDir: "/tmp/oc",
      instanceId: "0f9d8c7b6a5e4f3d2c1b0a9988776655",
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

/**
 * Where the bridge actually is.
 *
 * The tests above prove the console drives a bridge correctly. They cannot
 * prove it drives the *right object*, because that object is one this file
 * writes: a suite and a fixture that agree with each other and with nothing
 * else pass exactly as loudly as a correct pair. That is how #616 shipped — 82
 * desktop tests green over a bridge that resolved to `null` in production.
 *
 * So these read the shape off `@tauri-apps/api`, the package Tauri bundles into
 * the global, rather than off anything written here. A future version that
 * moves `invoke` fails at the version bump instead of at someone's first launch.
 */
describe("the injected bridge shape", () => {
  afterEach(() => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
  });

  it("namespaces `invoke` and `Channel` under `core`, not at the top level", async () => {
    // `withGlobalTauri` assigns `window.__TAURI__` this package's whole bundle
    // — `crates/tauri/scripts/bundle.global.js` ends `window.__TAURI__ =
    // __TAURI_IIFE__` — so the package's export map IS the global's shape.
    const api = await import("@tauri-apps/api");

    expect(typeof api.core.invoke).toBe("function");
    expect(typeof api.core.Channel).toBe("function");

    // The v1 shape. Asserted absent rather than merely unused: `undefined` is
    // what both readers in `transport/` dereferenced, and the failure was
    // silent precisely because reading a missing property is not an error.
    expect("invoke" in api).toBe(false);
    expect("Channel" in api).toBe(false);
  });

  it("resolves a bridge from the v2 global the mocks install", () => {
    installBridge();
    expect(typeof tauriCore()?.invoke).toBe("function");
  });

  it("refuses the v1 global rather than returning something that cannot invoke", () => {
    // What Tauri v1 injected, and what every mock in this suite used to
    // install. It must not resolve: a bridge that type-checks and no-ops is the
    // failure #616 is about.
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      invoke: () => Promise.resolve(),
      Channel: class {},
    };
    expect(tauriCore()).toBeNull();
  });

  it("is null in a browser, which has no global at all", () => {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
    expect(tauriCore()).toBeNull();
  });
});
