// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

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
