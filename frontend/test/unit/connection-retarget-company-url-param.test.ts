// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { resolveConfig } from "@/config";
import {
  addConnection,
  getConnection,
  resetConnections,
  restoreConnections,
  retargetCompanyUrlParam,
  retargetDefaultCompany,
} from "@/connections/registry";

/**
 * Codex review on #1828 (PR comment 3863028385): `retargetDefaultCompany`
 * fixes the persisted profile a reset leaves behind, but a connection opened
 * from a `?company=<id>` link is re-minted from that URL on every reload —
 * `resolveConfig()` reads it fresh from `window.location.search`, and a
 * reset never touches the address bar. Left stale, the next reload's
 * bootstrap `addConnection` call looks up `findProfile(baseUrl, archivedId)`,
 * which no longer matches the retargeted profile (its `defaultCompany` is now
 * the replacement's id), mints a fresh duplicate connection still scoped to
 * the archived id, and that connection's boot effect asks the host for an id
 * that no longer exists — a connection error instead of the replacement.
 *
 * `retargetCompanyUrlParam` is what `ConnectionConsole`'s `onCompanyCreated`
 * now calls beside `retargetDefaultCompany` on every reset. These tests drive
 * the real bootstrap sequence App.tsx runs on load (`resolveConfig` +
 * `restoreConnections` + the bootstrap `addConnection` call) against a real
 * `window.location`, in the style of `magic-link-scope.test.ts`.
 */

function land(search: string): void {
  window.history.replaceState({}, "", `/${search}`);
}

beforeEach(() => {
  resetConnections();
  window.localStorage.clear();
  land("");
});

afterEach(() => {
  resetConnections();
  window.localStorage.clear();
  land("");
});

describe("retargetCompanyUrlParam", () => {
  it("keeps a ?company= bootstrap reusing the same connection across a reload after reset", () => {
    land("?company=acme");
    const bootConfig = resolveConfig();
    expect(bootConfig.company).toBe("acme");
    const id = addConnection({ baseUrl: "https://acme.test", defaultCompany: bootConfig.company });

    // The reset: registry and URL both retargeted, as ConnectionConsole's
    // onCompanyCreated now does.
    retargetDefaultCompany(id, "acme-x7f2a91c");
    retargetCompanyUrlParam("acme", "acme-x7f2a91c");

    expect(window.location.search).toBe("?company=acme-x7f2a91c");

    // A reload: the in-memory registry is gone; the URL and localStorage
    // persist. Replay exactly what App.tsx's bootstrap does.
    resetConnections();
    const reloadedConfig = resolveConfig();
    expect(reloadedConfig.company).toBe("acme-x7f2a91c");

    restoreConnections();
    const reloadedId = addConnection({
      baseUrl: "https://acme.test",
      defaultCompany: reloadedConfig.company,
    });

    // The same connection is reused — no orphaned duplicate left pointed at
    // the archived company.
    expect(reloadedId).toBe(id);
    expect(getConnection(reloadedId)?.defaultCompany).toBe("acme-x7f2a91c");
    expect(getConnection(reloadedId)?.status).not.toBe("unauthenticated");
  });

  it("without it, a reload mints a duplicate connection still scoped to the archived id", () => {
    land("?company=acme");
    const bootConfig = resolveConfig();
    const id = addConnection({ baseUrl: "https://acme.test", defaultCompany: bootConfig.company });

    // The registry is retargeted, but the URL param is deliberately left
    // alone — this is the state the previous fix (retargetDefaultCompany
    // alone) left a reload in.
    retargetDefaultCompany(id, "acme-x7f2a91c");

    resetConnections();
    const reloadedConfig = resolveConfig();
    expect(reloadedConfig.company).toBe("acme"); // still the archived id

    restoreConnections();
    const reloadedId = addConnection({
      baseUrl: "https://acme.test",
      defaultCompany: reloadedConfig.company,
    });

    // A different connection, still scoped to the id the reset archived —
    // the exact orphaning `retargetCompanyUrlParam` exists to prevent.
    expect(reloadedId).not.toBe(id);
    expect(getConnection(reloadedId)?.defaultCompany).toBe("acme");
  });

  it("is a no-op when the URL never named the archived id", () => {
    land("?company=acme");

    retargetCompanyUrlParam("someone-else", "acme-x7f2a91c");

    expect(window.location.search).toBe("?company=acme");
  });

  it("preserves the hash and other query params", () => {
    land("?api=https%3A%2F%2Facme.test&company=acme&hub=1#/overview");

    retargetCompanyUrlParam("acme", "acme-x7f2a91c");

    expect(window.location.search).toContain("company=acme-x7f2a91c");
    expect(window.location.search).toContain("hub=1");
    expect(window.location.hash).toBe("#/overview");
  });
});
