// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  addConnection,
  getConnection,
  listConnections,
  removeConnection,
  resetConnections,
  restoreConnections,
} from "@/connections/registry";
import { findProfile, readProfiles } from "@/connections/profileStore";
import { scopedKey } from "@/connections/types";

/**
 * The connection registry, and the two properties that make it safe to key
 * everything else on.
 *
 * The console holds N hosts at once. Every browser-local key — tour progress,
 * last-read channel, mail draft, workspace migration flag — is namespaced by a
 * connection id, so these tests are really about that namespace: it must be
 * stable for one host over time, and distinct between hosts.
 */

beforeEach(() => {
  resetConnections();
  window.localStorage.clear();
});

describe("connection ids", () => {
  it("stay the same for a host across reloads", () => {
    // THE regression this store exists to prevent. Ids are minted randomly, so
    // without persistence a reload re-mints — and every scoped key moves to a
    // fresh namespace, orphaning the state silently. No error, just a console
    // with amnesia and a cause nowhere near the symptom.
    const first = addConnection({ baseUrl: "https://acme.test" });

    // A reload: the module's in-memory entries are gone, `localStorage` is not.
    resetConnections();
    const second = addConnection({ baseUrl: "https://acme.test" });

    expect(second).toBe(first);
    expect(scopedKey("oc-tour", { connection: second, company: "acme" })).toBe(
      scopedKey("oc-tour", { connection: first, company: "acme" }),
    );
  });

  it("differ between hosts, so their local state cannot collide", () => {
    const a = addConnection({ baseUrl: "https://a.test" });
    const b = addConnection({ baseUrl: "https://b.test" });

    expect(a).not.toBe(b);
    // Same company name on two hosts: the case buzz gets wrong.
    expect(scopedKey("oc-tour", { connection: a, company: "acme" })).not.toBe(
      scopedKey("oc-tour", { connection: b, company: "acme" }),
    );
  });

  it("differ per addressed company on one host", () => {
    // `?company=a` and `?company=b` against one host are two consoles, and
    // their view state should not be shared.
    const a = addConnection({ baseUrl: "https://acme.test", defaultCompany: "one" });
    const b = addConnection({ baseUrl: "https://acme.test", defaultCompany: "two" });
    expect(a).not.toBe(b);
  });

  it("never contain the scoped-key separator", () => {
    // `scopedKey` splits on `::`. An id containing it would make the split
    // ambiguous, and two different scopes could render the same key.
    for (let i = 0; i < 50; i += 1) {
      resetConnections();
      window.localStorage.clear();
      const id = addConnection({ baseUrl: `https://host-${i}.test` });
      expect(id).not.toContain(":");
    }
  });
});

describe("registering a host", () => {
  it("does not duplicate a row for one already registered", () => {
    // The web build adds its bootstrap connection from a `useMemo`, which
    // StrictMode double-invokes.
    const first = addConnection({ baseUrl: "https://acme.test" });
    const second = addConnection({ baseUrl: "https://acme.test" });

    expect(second).toBe(first);
    expect(listConnections()).toHaveLength(1);
  });

  it("normalises a trailing slash, so one host is one connection", () => {
    const bare = addConnection({ baseUrl: "https://acme.test" });
    const slashed = addConnection({ baseUrl: "https://acme.test/" });
    expect(slashed).toBe(bare);
  });

  it("starts out connecting, with nothing claimed about the host yet", () => {
    const id = addConnection({ baseUrl: "https://acme.test" });
    const connection = getConnection(id);
    expect(connection?.status).toBe("connecting");
    // `null`, not an empty identity: nothing has been asked yet, which is a
    // different thing from a host that answered with nothing.
    expect(connection?.identity).toBeNull();
    expect(connection?.companies).toEqual([]);
  });

  it("labels a host by its authority until it says otherwise", () => {
    expect(getConnection(addConnection({ baseUrl: "https://acme.test:9000" }))?.label).toBe(
      "acme.test:9000",
    );
    // Same-origin, which is how the web build is configured.
    expect(getConnection(addConnection({ baseUrl: "" }))?.label).toBe("This host");
  });
});

describe("persistence", () => {
  it("stores no secret, whatever credential the connection holds", () => {
    // Written to `localStorage`, which any script in the page can read. A
    // device token must live in the OS keychain, and only a handle to it here.
    addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "device", ref: "keychain-handle" },
    });
    const raw = window.localStorage.getItem("oc.connections.v1") ?? "";
    expect(raw).toContain("keychain-handle");
    expect(raw).not.toContain("oc_dev_");
  });

  it("forgets a removed host rather than resurrecting it on reload", () => {
    const id = addConnection({ baseUrl: "https://acme.test" });
    removeConnection(id);

    expect(findProfile("https://acme.test", null)).toBeUndefined();
    resetConnections();
    expect(addConnection({ baseUrl: "https://acme.test" })).not.toBe(id);
  });

  it("ignores a corrupt entry instead of registering a connection with no id", () => {
    // Hand-edited or half-written storage. An entry whose `id` is `undefined`
    // would collapse every scoped key onto one shared namespace.
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([{ baseUrl: "https://acme.test" }, "nonsense", null]),
    );
    expect(readProfiles()).toEqual([]);
    expect(addConnection({ baseUrl: "https://acme.test" })).toBeTruthy();
  });

  it("survives storage it cannot parse at all", () => {
    window.localStorage.setItem("oc.connections.v1", "{not json");
    expect(readProfiles()).toEqual([]);
  });
});

describe("credentials", () => {
  it("never writes a platform bearer to local storage", () => {
    // A platform bearer is a machine credential. It arrives in `?token=` and
    // `stripAuthParams` deletes it from the address bar immediately, so that it
    // does not linger anywhere readable — persisting it here would undo that
    // and go further, since `localStorage` has no expiry and any injected
    // script can read it.
    const token = "platform-bearer-do-not-persist";
    addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "platform", token },
    });

    expect(window.localStorage.getItem("oc.connections.v1")).not.toContain(token);
    // The kind survives as a marker — "this host authenticates as the
    // platform" — while the secret does not. The live token is re-derived from
    // the URL on the next load.
    expect(readProfiles()[0]?.credential).toEqual({ kind: "platform" });
    // In memory it is still the live credential — only the written form is
    // redacted.
    expect(getConnection(listConnections()[0].id)?.credential).toEqual({
      kind: "platform",
      token,
    });
  });

  it("re-applies the live bearer to a restored connection", () => {
    // The consequence of not persisting it, and the reason `addConnection`
    // cannot simply return early. `restoreConnections` runs first on every
    // load and can only supply what was written down; the bootstrap add that
    // follows carries the token from `?token=`. Returning the existing entry
    // without adopting it would leave the connection permanently
    // unauthenticated after one reload.
    const token = "fresh-bearer";
    const first = addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "platform", token: "stale" },
    });

    // A reload: memory is gone, storage is not.
    resetConnections();
    const restored = restoreConnections();
    expect(getConnection(restored[0])?.credential).toEqual({ kind: "platform" });

    const bootstrap = addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "platform", token },
    });
    expect(bootstrap).toBe(first);
    expect(getConnection(bootstrap)?.credential).toEqual({ kind: "platform", token });
  });

  it("leaves a device credential alone, because a ref is not a secret", () => {
    // `device.ref` names a keychain entry rather than holding the token, which
    // is the whole reason the type is shaped that way. Redacting it would lose
    // the handle and there would be nothing to look the secret up with.
    addConnection({
      baseUrl: "https://acme.test",
      credential: { kind: "device", ref: "keychain-handle-1" },
    });
    expect(readProfiles()[0]?.credential).toEqual({ kind: "device", ref: "keychain-handle-1" });
  });
});

/**
 * The same-origin connection, which is a host in one runtime and nothing at all
 * in the other.
 *
 * An empty base url means "the origin serving this page". A browser is served
 * by its host, so that resolves; a desktop webview is served by
 * `tauri://localhost`, where no host has ever listened. Issue #613: the desktop
 * added one anyway, selected it, and reported "couldn't reach a company host at
 * this origin" on every launch with a healthy embedded host in the same rail.
 */
describe("a same-origin profile", () => {
  const desktop = (present: boolean) => {
    if (present) (window as unknown as { __TAURI__: unknown }).__TAURI__ = {};
    else delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
  };

  // The registry is module state, so a runtime marker left set would follow the
  // suite into the next file.
  afterEach(() => desktop(false));

  it("is dropped and forgotten when the desktop restores its hosts", () => {
    // Written by a build that added the bootstrap connection unconditionally.
    // Skipping the add is not enough on its own — this store is what brings a
    // connection back, so the dead row would return on every launch forever.
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        { id: "5pnbp7zfx7w6", baseUrl: "", label: "This host", defaultCompany: null, credential: { kind: "cookie" } },
        {
          id: "4g4392soz5vm",
          baseUrl: "http://127.0.0.1:65364",
          label: "This computer",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
    desktop(true);

    const restored = restoreConnections();

    expect(restored).toHaveLength(1);
    expect(getConnection(restored[0])?.baseUrl).toBe("http://127.0.0.1:65364");
    // Forgotten, not merely skipped: a row nothing restores is a row nothing
    // ever removes, and this store is what someone reads to see what the
    // console holds.
    expect(readProfiles().map((p) => p.baseUrl)).toEqual(["http://127.0.0.1:65364"]);
  });

  it("takes a scheme-less host down with it, because that is what people type", () => {
    // "Add a host" does no validation, so `localhost:8080` becomes a row and is
    // written down. It joins to a relative url in Rust exactly as `""` does, and
    // without this it would be restored on every launch forever — the same
    // permanence the empty-base row had, reached by a path a person can walk.
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "conn-typed",
          baseUrl: "localhost:8080",
          label: "localhost:8080",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
        {
          id: "conn-real",
          baseUrl: "https://acme.example.com",
          label: "Acme",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
    desktop(true);

    expect(restoreConnections()).toEqual(["conn-real"]);
    expect(readProfiles().map((p) => p.baseUrl)).toEqual(["https://acme.example.com"]);
  });

  it("is still a host in a browser, where the origin serves one", () => {
    // The other half of the rule, and the one that must not change: this is how
    // every web deployment finds its host.
    const id = addConnection({ baseUrl: "" });
    resetConnections();

    expect(restoreConnections()).toEqual([id]);
    expect(getConnection(id)?.label).toBe("This host");
  });
});
