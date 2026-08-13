// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ComposioConnectedAccount, ComposioToolkitEntry } from "@/api/composio";
import type { ConnectionState, UsageDto } from "@/api/types";
import type { ComposioReach } from "@/lib/connections";
import { buildGridProviders, type GridProvider } from "@/lib/provider-grid";
import { ProviderDetail } from "@/views/connections/ProviderDetail";

/**
 * The provider detail view's claims (issue #404).
 *
 * This suite is normally for pure functions — see `vitest.config.ts`. The
 * exception is earned the same way `select-popup-width` earns it: the thing
 * under test *is* what reaches the operator's eye. The issue is not asking for
 * fields on a panel, it is asking that every sentence on the panel be one the
 * system can back, and three of them cannot be checked anywhere but here:
 *
 *  1. no account is marked as the one agents use, because none is;
 *  2. a missing connection date says so rather than showing a blank;
 *  3. a member is not offered a disconnect the host will refuse (#403).
 */

const OPEN: ComposioReach = {
  inBuild: true,
  granted: true,
  hasCredential: true,
  openMode: true,
  effectiveToolkits: [],
};

const EMPTY_USAGE: UsageDto = {
  totals: {
    inputTokens: 0,
    outputTokens: 0,
    tokens: 0,
    costUsd: 0,
    oauthCalls: 0,
    connections: 0,
    searchCalls: 0,
  },
  series: [],
  byAgent: [],
  byProvider: [],
};

function entry(slug: string, name: string): ComposioToolkitEntry {
  return { slug, name, description: "", logo: null, categories: [] };
}

function account(over: Partial<ComposioConnectedAccount> = {}): ComposioConnectedAccount {
  return { id: "conn-1", status: "ACTIVE", connected: true, ...over };
}

/** A real grid row, built the way the page builds it. */
function gmail(accounts: ComposioConnectedAccount[], via: ConnectionState["via"] = ["composio"]) {
  const rows = buildGridProviders(
    [entry("gmail", "Gmail")],
    [],
    { gmail: { provider: "gmail", connected: true, via } },
    OPEN,
    false,
    { gmail: accounts },
  );
  return rows.find((p) => p.slug === "gmail")!;
}

/** A host that answers the usage route with `byProvider` rows. */
function clientWith(byProvider: UsageDto["byProvider"]): OpenCompanyClient {
  return {
    usage: async () => ({ ...EMPTY_USAGE, byProvider }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function render(provider: GridProvider, canManage: boolean, client = clientWith([])) {
  await act(async () => {
    root.render(
      createElement(ProviderDetail, {
        client,
        company: null,
        provider,
        canManage,
        busy: false,
        onClose: () => {},
        onConnectAnother: () => {},
        onDisconnectAccount: () => {},
      }),
    );
  });
}

/** The panel renders through a portal, so it is on `document`, not `container`. */
function text(): string {
  return document.body.textContent ?? "";
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the provider detail view", () => {
  it("lists every account with the status Composio reported", async () => {
    await render(
      gmail([
        account({ id: "conn-gmail-1", account: "ops@acme.test" }),
        account({ id: "conn-gmail-2", account: "billing@acme.test", status: "EXPIRED", connected: false }),
      ]),
      true,
    );
    expect(text()).toContain("ops@acme.test");
    expect(text()).toContain("billing@acme.test");
    // Verbatim, not re-spelled: "set up and since expired" and "never finished
    // setting up" are different sentences that both flatten to "not connected".
    expect(text()).toContain("EXPIRED");
  });

  it("marks no account as the one agents use, because nothing chooses one", async () => {
    // OpenHuman marks the first of several as the default and inheriting that
    // was the plan. It is not true here: `composio_execute` posts
    // `{tool, arguments}` and no connection id, so Composio resolves which
    // account acts. A "Default" chip would name a decision this product does
    // not make — the exact shape of invention the issue rules out.
    await render(
      gmail([
        account({ id: "conn-gmail-1", account: "ops@acme.test" }),
        account({ id: "conn-gmail-2", account: "billing@acme.test" }),
      ]),
      true,
    );
    expect(text()).not.toContain("Default");
    expect(text()).toContain("composio_execute");
  });

  it("says a connection date is not recorded rather than leaving it blank", async () => {
    await render(gmail([account({ account: "ops@acme.test" })]), true);
    expect(text()).toContain("connection date not recorded");
  });

  it("states usage is counted per provider, not per account", async () => {
    await render(
      gmail([
        account({ id: "conn-gmail-1", account: "ops@acme.test" }),
        account({ id: "conn-gmail-2", account: "billing@acme.test" }),
      ]),
      true,
      clientWith([{ provider: "gmail", calls: 12 }]),
    );
    expect(text()).toContain("12");
    expect(text()).toContain("per provider rather than per account");
  });

  it("does not report a zero when the host records no usage at all", async () => {
    // A failed usage read is not a company that made no calls. Rendering "0"
    // here is precisely the plausible-looking zero the issue forbids.
    const broken = {
      usage: async () => {
        throw new Error("no usage route on this host");
      },
    } as unknown as OpenCompanyClient;
    await render(gmail([account()]), true, broken);
    expect(text()).toContain("does not report usage");
    expect(text()).not.toContain("0 calls");
  });

  it("says what a disconnect will and will not revoke", async () => {
    await render(gmail([account({ account: "ops@acme.test" })]), true);
    expect(text()).toContain("removes the connection at Composio");
    expect(text()).toContain("does not sign the company out");
  });

  it("offers a member no control the host would refuse", async () => {
    // Issue #403. Courtesy, not enforcement — the host answers 403 whatever
    // this renders — but a Disconnect that can only fail is a poor thing to
    // put in front of someone reading the page to find out what is wired.
    await render(gmail([account({ account: "ops@acme.test" })]), false);
    expect(text()).toContain("ops@acme.test");
    expect(text()).not.toContain("Disconnect");
    expect(text()).not.toContain("Connect another account");
    expect(text()).toContain("Only an admin can connect or disconnect");
  });

  it("names the inert native credential as a caveat, not as a second connection", async () => {
    // #396: the native `oauth/{provider}` entry is written and read by nothing.
    // A detail view is where that would look healthiest, so it is stated.
    await render(gmail([account()], ["composio", "native"]), true);
    expect(text()).toContain("No agent reads it");
  });
});
