// Which route a Connections tile's button takes, and whether it gets one at all
// (issue #599).
//
// The bug these pin: the grid renders every catalog tile, but `GET …/connections`
// only answers for providers the company manifest declares. A hosted tenant
// declares none, so every tile had `state === undefined` — the `attested` guard
// had no row to read itself out of, and all eleven fell through to a Connect
// that 400'd with "provider is not enabled on this host".

import { describe, expect, it } from "vitest";

import {
  CONNECTION_PROVIDERS,
  composioCanAuthorize,
  connectRoute,
  connectionStateFor,
  toolkitSlug,
  type ComposioReach,
  type ConnectionProvider,
} from "@/lib/connections";

/** The tile for `id`, which must exist — a typo here should fail loudly. */
function tile(id: string): ConnectionProvider {
  const found = CONNECTION_PROVIDERS.find((p) => p.id === id);
  if (!found) throw new Error(`no such tile: ${id}`);
  return found;
}

/** A host where Composio is live and everything is reachable (open mode). */
const OPEN: ComposioReach = {
  inBuild: true,
  granted: true,
  hasCredential: true,
  openMode: true,
  effectiveToolkits: [],
};

describe("toolkitSlug", () => {
  // Mirrors `toolkit_slug()` in src/server/ops/connections_read.rs — the two
  // namespaces only reconcile into one row if both sides normalize the same way.
  it("strips punctuation and case, matching the backend rule", () => {
    expect(toolkitSlug("google-calendar")).toBe("googlecalendar");
    expect(toolkitSlug("google-drive")).toBe("googledrive");
    expect(toolkitSlug("GitHub")).toBe("github");
    expect(toolkitSlug("gmail")).toBe("gmail");
  });
});

describe("connectionStateFor", () => {
  it("matches a row keyed by the manifest's own provider id", () => {
    const state = { provider: "slack", connected: true };
    expect(connectionStateFor(tile("slack"), { slack: state })).toBe(state);
  });

  it("matches a reconciled Composio row across the spelling difference", () => {
    // The host appends rows for Composio-connected providers keyed by Composio
    // slug. `googlecalendar` and the `google-calendar` tile are one provider;
    // a raw `states[p.id]` lookup misses it and the tile keeps saying Connect
    // while the account is in fact connected.
    const state = { provider: "googlecalendar", connected: true };
    expect(connectionStateFor(tile("google-calendar"), { googlecalendar: state })).toBe(state);
  });

  it("matches when only the tile's toolkit differs outright from its id", () => {
    // `x` → `twitter` is the case no normalization rule can derive.
    const state = { provider: "twitter", connected: true };
    expect(connectionStateFor(tile("x"), { twitter: state })).toBe(state);
  });

  it("is undefined when no namespace mentions the provider", () => {
    expect(connectionStateFor(tile("stripe"), {})).toBeUndefined();
  });

  it("prefers the manifest row when two rows describe the same tile and none is connected", () => {
    // Direct-id precedence. Both rows match the `x` tile; with nothing
    // connected the manifest's own row is the authority.
    const manifest = { provider: "x", connected: false, account: "from-manifest" };
    const composio = { provider: "twitter", connected: false, account: "from-composio" };
    expect(connectionStateFor(tile("x"), { x: manifest, twitter: composio })).toBe(manifest);
  });

  it("prefers a connected row over a disconnected one for the same tile", () => {
    // `toolkit_slug("x")` is `x`, not `twitter`, so the host cannot union these
    // two itself: a manifest `provider = "x"` yields a disconnected `x` row
    // while Composio's connected `twitter` state arrives as a separate appended
    // row. Taking the first match would report the tile disconnected while the
    // account is in fact connected.
    const manifest = { provider: "x", connected: false };
    const composio = { provider: "twitter", connected: true };
    expect(connectionStateFor(tile("x"), { x: manifest, twitter: composio })).toBe(composio);
  });

  it("applies the same union to a hyphenated id and its Composio slug", () => {
    const manifest = { provider: "google-calendar", connected: false };
    const composio = { provider: "googlecalendar", connected: true };
    expect(
      connectionStateFor(tile("google-calendar"), {
        "google-calendar": manifest,
        googlecalendar: composio,
      }),
    ).toBe(composio);
  });
});

describe("composioCanAuthorize", () => {
  it("is false without a reachable Composio", () => {
    expect(composioCanAuthorize(null, "slack")).toBe(false);
    expect(composioCanAuthorize({ ...OPEN, inBuild: false }, "slack")).toBe(false);
    // `credentialSource: "none"` — nothing to authorize against.
    expect(composioCanAuthorize({ ...OPEN, hasCredential: false }, "slack")).toBe(false);
  });

  // Issue #582. This assertion used to read `false`, and that was half of the
  // page's self-contradiction: `POST …/composio/authorize` never consulted the
  // grant (`resolve_tenant` reads the credential and the toolkit allowlist and
  // nothing else), so the sign-in this predicate refused to offer worked fine
  // from the provider list that ignored the grant. One surface said "connect
  // me", the other said "no route", for the same account on the same screen.
  //
  // The grant governs whether AGENTS receive Composio tools, which is a caveat
  // the grid states next to the connected badge — not a reason to withhold a
  // handshake that completes.
  it("offers a route without the composio grant, which gates tools and not the handshake", () => {
    expect(composioCanAuthorize({ ...OPEN, granted: false }, "slack")).toBe(true);
  });

  it("allows any toolkit in open mode, where the backend allowlist governs", () => {
    // Issue #397: an empty manifest list means "allow everything", so the
    // effective list is a display list here, not a limit.
    expect(composioCanAuthorize(OPEN, "hubspot")).toBe(true);
  });

  it("honours the manifest allowlist as a real limit outside open mode", () => {
    const narrow: ComposioReach = {
      ...OPEN,
      openMode: false,
      effectiveToolkits: ["gmail", "googlecalendar"],
    };
    expect(composioCanAuthorize(narrow, "googlecalendar")).toBe(true);
    // Offering a Connect for a toolkit outside the list would only move the
    // 400 from one backend to the other.
    expect(composioCanAuthorize(narrow, "stripe")).toBe(false);
  });
});

describe("connectRoute", () => {
  it("routes every tile through Composio on a tenant that declares no connections", () => {
    // The #599 regression guard. No manifest rows at all, so no tile has state.
    for (const provider of CONNECTION_PROVIDERS) {
      expect(connectRoute(provider, undefined, OPEN)).toEqual({
        kind: "composio",
        toolkit: provider.toolkit,
      });
    }
  });

  it("never offers a Connect when no route can succeed", () => {
    // The shipped behaviour before this fix: a button on every tile, and every
    // one of them 400s. `unavailable` is what the operator gets instead.
    for (const provider of CONNECTION_PROVIDERS) {
      expect(connectRoute(provider, undefined, null)).toEqual({ kind: "unavailable" });
    }
  });

  it("no longer offers the self-hosted hatch, whose credential no agent reads", () => {
    // Issue #822. Both assertions used to read `{ kind: "native" }`, on the
    // reasoning that `static` is a deliberate act by the operator — a
    // registered provider application, or a token this company stored — and
    // that preferring Composio would take away the hatch they configured.
    //
    // What that missed is what the hatch confers: nothing. `oauth/{provider}`
    // is written by the callback and read by no agent tool — zero occurrences
    // under `src/harness/` (#396) — so the arm preserved the operator's
    // configuration by handing them a green tile and no capability. A Connect
    // that 400s is a bad button; one that succeeds and buys nothing is a false
    // promise, and #599 only fixed the first kind.
    //
    // A host that also reaches Composio now takes the route that does confer
    // something...
    expect(connectRoute(tile("github"), { credentialSource: "static" }, OPEN)).toEqual({
      kind: "composio",
      toolkit: "github",
    });
    // ...and a host whose only route was the hatch says so, which is what the
    // tile already renders without an action.
    expect(connectRoute(tile("github"), { credentialSource: "static" }, null)).toEqual({
      kind: "unavailable",
    });
  });

  it("returns no native route for any tile, under any host shape", () => {
    // The regression guard for #822 as a whole: the arm is gone, not merely
    // deprioritised, so no combination of tier and reach can reach it. `static`
    // is the tier that used to, and it is in the sweep.
    const tiers = [undefined, "static", "attested", "company", "none"] as const;
    for (const provider of CONNECTION_PROVIDERS) {
      for (const tier of tiers) {
        for (const reach of [OPEN, null]) {
          const state = tier === undefined ? undefined : { credentialSource: tier };
          expect(
            connectRoute(provider, state, reach).kind,
            `${provider.id} still routes natively for ${tier ?? "no"} state`,
          ).not.toBe("native");
        }
      }
    }
  });

  it("prefers Composio over a platform identity that runs no connection here", () => {
    // `attested` says the platform owns connections, but Composio is a live
    // route on the same host — and the one that actually gives agents tools.
    expect(connectRoute(tile("notion"), { credentialSource: "attested" }, OPEN)).toEqual({
      kind: "composio",
      toolkit: "notion",
    });
  });

  it("falls back to managed when the platform runs connections and Composio does not", () => {
    expect(connectRoute(tile("notion"), { credentialSource: "attested" }, null)).toEqual({
      kind: "managed",
    });
  });

  it("routes on whether a Composio credential exists, not on which tier it is", () => {
    // Issue #586 adds a `company` tier (the company's own TinyHumans key)
    // alongside `attested` and `static`. `connectRoute` names only the two
    // tiers that describe the LOCAL host, so a new Composio tier is additive
    // by construction and needs no case here.
    //
    // This pins the specific combination neither #599 nor #586 covers alone:
    // the company credential is set, and the provider is not in `states`.
    const companyTier: ComposioReach = { ...OPEN, hasCredential: true };
    expect(connectRoute(tile("stripe"), undefined, companyTier)).toEqual({
      kind: "composio",
      toolkit: "stripe",
    });
    // And a connection row reporting the new tier is not mistaken for the
    // native hatch — only `static` means a local handshake can complete.
    expect(connectRoute(tile("stripe"), { credentialSource: "company" }, companyTier)).toEqual({
      kind: "composio",
      toolkit: "stripe",
    });
    expect(connectRoute(tile("stripe"), { credentialSource: "company" }, null)).toEqual({
      kind: "unavailable",
    });
  });

  it("reports unavailable for a provider the host explicitly has no route for", () => {
    expect(connectRoute(tile("stripe"), { credentialSource: "none" }, null)).toEqual({
      kind: "unavailable",
    });
  });

  it("gives the eight ids with no native backend key a working Composio route", () => {
    // Native OAuth is retired, and these provider ids never had a native route
    // before it retired. Composio is what makes them connectable at all.
    for (const id of [
      "google-calendar",
      "notion",
      "google-drive",
      "dropbox",
      "stripe",
      "hubspot",
      "x",
      "linkedin",
    ]) {
      expect(connectRoute(tile(id), undefined, OPEN).kind).toBe("composio");
    }
  });
});

describe("the tile catalog", () => {
  it("gives every tile a Composio toolkit slug", () => {
    for (const provider of CONNECTION_PROVIDERS) {
      expect(provider.toolkit, `${provider.id} has no toolkit`).toBeTruthy();
      // The slug is what the host is called with, so it must already be in
      // Composio's spelling rather than needing normalization at the call site.
      expect(toolkitSlug(provider.toolkit)).toBe(provider.toolkit);
    }
  });
});
