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
});

describe("composioCanAuthorize", () => {
  it("is false without a reachable Composio", () => {
    expect(composioCanAuthorize(null, "slack")).toBe(false);
    expect(composioCanAuthorize({ ...OPEN, inBuild: false }, "slack")).toBe(false);
    expect(composioCanAuthorize({ ...OPEN, granted: false }, "slack")).toBe(false);
    // `credentialSource: "none"` — nothing to authorize against.
    expect(composioCanAuthorize({ ...OPEN, hasCredential: false }, "slack")).toBe(false);
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

  it("keeps the self-hosted hatch when the host registered its own provider app", () => {
    // `static` is a deliberate act by the operator — either a registered
    // provider application or a token this company stored. Preferring Composio
    // here would quietly take away the hatch they configured.
    expect(connectRoute(tile("github"), { credentialSource: "static" }, OPEN)).toEqual({
      kind: "native",
    });
    expect(connectRoute(tile("github"), { credentialSource: "static" }, null)).toEqual({
      kind: "native",
    });
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

  it("reports unavailable for a provider the host explicitly has no route for", () => {
    expect(connectRoute(tile("stripe"), { credentialSource: "none" }, null)).toEqual({
      kind: "unavailable",
    });
  });

  it("gives the eight ids with no native backend key a working Composio route", () => {
    // `well_known()` recognises only slack / google / gmail / github, so these
    // could never complete a native handshake no matter how the host is
    // configured. Composio is what makes them connectable at all.
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
