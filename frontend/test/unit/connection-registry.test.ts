// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import {
  addConnection,
  adoptEmbeddedHost,
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
 * The host running inside this application (#615).
 *
 * The one connection whose address is *expected* to differ from last launch's:
 * it binds an ephemeral port on purpose, so recognising it the way every other
 * host is recognised — by address — read each launch as a first meeting and
 * left the previous one's row behind, dead, durable and identically labelled.
 *
 * A relaunch is modelled the way the app performs one: memory cleared,
 * `localStorage` kept, `restoreConnections` first, then the embedded host
 * arriving over IPC at whatever port the OS gave it this time.
 */
describe("the embedded host", () => {
  const INSTANCE = "0f9d8c7b6a5e4f3d2c1b0a9988776655";

  function relaunch(): void {
    resetConnections();
    restoreConnections();
  }

  it("is one row however many times the application restarts", () => {
    const first = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65145",
      instanceId: INSTANCE,
    });

    relaunch();
    const second = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65275",
      instanceId: INSTANCE,
    });
    relaunch();
    const third = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65364",
      instanceId: INSTANCE,
    });

    expect(listConnections()).toHaveLength(1);
    // The same id throughout, so the tour state, last-read channel and mail
    // draft scoped to it survive the relaunch rather than being orphaned.
    expect(second).toBe(first);
    expect(third).toBe(first);
    expect(readProfiles()).toHaveLength(1);
  });

  it("follows the port it is actually listening on", () => {
    // Keeping one row is only half of it: the row that survives has to address
    // the live port, not the closed one it was restored with.
    adoptEmbeddedHost({ baseUrl: "http://127.0.0.1:65145", instanceId: INSTANCE });
    relaunch();
    const id = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65275",
      instanceId: INSTANCE,
    });

    expect(getConnection(id)?.baseUrl).toBe("http://127.0.0.1:65275");
    expect(readProfiles()[0]?.baseUrl).toBe("http://127.0.0.1:65275");
    // Re-probed rather than left showing what the old address concluded.
    expect(getConnection(id)?.status).toBe("connecting");
  });

  it("clears the rows an older version already left behind", () => {
    // The registry is durable, so fixing the accumulation is not enough on its
    // own — an existing install starts with the pile already there. This is the
    // state from the issue, verbatim: the bootstrap host plus one dead "This
    // computer" per previous launch, none of them carrying an identity because
    // no version that wrote them reported one.
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "5pnbp7zfx7w6",
          baseUrl: "",
          label: "This host",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
        {
          id: "vad0klxipf59",
          baseUrl: "http://127.0.0.1:65275",
          label: "This computer",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
        {
          id: "4g4392soz5vm",
          baseUrl: "http://127.0.0.1:65364",
          label: "This computer",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );

    restoreConnections();
    const id = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65401",
      instanceId: INSTANCE,
    });

    const rows = listConnections();
    expect(rows).toHaveLength(2);
    // The bootstrap connection is not this application's host and is not
    // touched — whatever else is wrong with it belongs to #613.
    expect(rows.map((c) => c.baseUrl).sort()).toEqual(["", "http://127.0.0.1:65401"]);
    // One of the orphans is adopted rather than discarded: they were all this
    // machine's host, so its scoped local state is this host's state.
    expect(["vad0klxipf59", "4g4392soz5vm"]).toContain(id);
    // And it now carries the identity, so the next launch matches on that
    // rather than on the guess that recovered it here.
    expect(readProfiles().find((p) => p.id === id)?.instanceId).toBe(INSTANCE);
  });

  it("does not hand a different instance the previous one's row", () => {
    // A second data root — `OPENCOMPANY_DATA_DIR` pointed elsewhere, say. It is
    // a different host that happens to run in the same application, and
    // adopting the row would merge two hosts' scoped local state: exactly the
    // silent mixing `types.ts` exists to prevent.
    const first = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65145",
      instanceId: INSTANCE,
    });
    relaunch();
    const second = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65275",
      instanceId: "ffffffffffffffffffffffffffffffff",
    });

    expect(second).not.toBe(first);
    // Still one row: the host this application no longer serves has no address
    // left to be reached at, so keeping its row would only re-create the bug.
    expect(listConnections()).toHaveLength(1);
  });

  it("leaves a loopback host the operator added by hand alone", () => {
    // The margin on the recovery above. Somebody running `opencompany serve` in
    // a terminal and adding it is ordinary, and deleting their connection would
    // be a worse bug than the one being fixed. They are labelled by authority,
    // never with the name this client gives its own host.
    const theirs = addConnection({ baseUrl: "http://127.0.0.1:8080" });
    expect(getConnection(theirs)?.label).toBe("127.0.0.1:8080");

    adoptEmbeddedHost({ baseUrl: "http://127.0.0.1:65145", instanceId: INSTANCE });

    expect(getConnection(theirs)).toBeDefined();
    expect(listConnections()).toHaveLength(2);
  });

  it("does not duplicate under StrictMode's double invocation", () => {
    const first = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65145",
      instanceId: INSTANCE,
    });
    const second = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:65145",
      instanceId: INSTANCE,
    });

    expect(second).toBe(first);
    expect(listConnections()).toHaveLength(1);
  });

  it("still collapses to one row on a shell that reports no identity", () => {
    // A `pnpm dev` console against an older `cargo` build. Without an identity
    // there is nothing to match on, but the invariant — one host inside this
    // application — holds regardless, so the row is still reused rather than
    // multiplied.
    const first = adoptEmbeddedHost({ baseUrl: "http://127.0.0.1:65145" });
    relaunch();
    const second = adoptEmbeddedHost({ baseUrl: "http://127.0.0.1:65275" });

    expect(second).toBe(first);
    expect(listConnections()).toHaveLength(1);
    expect(getConnection(second)?.baseUrl).toBe("http://127.0.0.1:65275");
  });
});
