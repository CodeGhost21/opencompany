// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ComposioConnectedAccount, ComposioToolkitEntry } from "@/api/composio";
import type { ComposioReach } from "@/lib/connections";
import { buildGridProviders, type GridProvider } from "@/lib/provider-grid";
import { ProvidersSection } from "@/views/connections/ProvidersSection";

/**
 * What a provider tile says about its accounts (issue #923).
 *
 * `provider-grid.test.ts` pins the counting rule as a function. This pins what
 * the operator actually reads, which is where the two rules met and disagreed —
 * the rule could be right and the tile still print the old sentence, and that is
 * exactly the regression this file exists to catch.
 *
 * The three shapes below are the three rows the issue reported from
 * `smoke1.staging`, and the strings asserted are what the grid said before the
 * fix. A tile is a button, so its text is reachable without driving the page.
 */

const OPEN: ComposioReach = {
  inBuild: true,
  granted: true,
  hasCredential: true,
  openMode: true,
  effectiveToolkits: [],
};

function entry(slug: string, name: string): ComposioToolkitEntry {
  return { slug, name, description: "", logo: null, categories: [] };
}

function acct(id: string, connected: boolean): ComposioConnectedAccount {
  return { id, status: connected ? "ACTIVE" : "INITIATED", connected };
}

/**
 * A grid row built the way the page builds it.
 *
 * `connected` mirrors the host's own rule — at least one account `ACTIVE` — so
 * the fixture cannot drift from `group_by_toolkit` in `src/server/ops/composio.rs`
 * and quietly stop describing the reported bug.
 */
function row(slug: string, name: string, accounts: ComposioConnectedAccount[]): GridProvider {
  const connected = accounts.some((a) => a.connected);
  const rows = buildGridProviders(
    [entry(slug, name)],
    [],
    { [slug]: { provider: slug, connected, via: connected ? ["composio"] : [] } },
    OPEN,
    false,
    { [slug]: accounts },
  );
  return rows.find((p) => p.slug === slug)!;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

async function render(
  providers: GridProvider[],
  overrides: { granted?: boolean | undefined; probeFailed?: boolean } = {},
) {
  await act(async () => {
    root.render(
      createElement(ProvidersSection, {
        providers,
        canManage: true,
        busy: null,
        noCredential: false,
        granted: true,
        probeFailed: false,
        openMode: false,
        degraded: null,
        loading: false,
        onConnect: () => {},
        onDisconnect: () => {},
        onOpen: () => {},
        onConnectSlug: () => {},
        ...overrides,
      }),
    );
  });
}

function text(): string {
  return container.textContent ?? "";
}

describe("what a tile says about its accounts", () => {
  it("does not count accounts no agent can act as", async () => {
    // Gmail as reported: one ACTIVE, five INITIATED. The tile said
    // "6 accounts connected" — `accounts.length`, under a badge that only
    // required one of them to be live.
    await render([
      row("gmail", "Gmail", [
        acct("g1", true),
        acct("g2", false),
        acct("g3", false),
        acct("g4", false),
        acct("g5", false),
        acct("g6", false),
      ]),
    ]);
    expect(text()).not.toContain("6 accounts connected");
    expect(text()).toContain("connected");
  });

  it("counts the live accounts and not the rest when several are live", async () => {
    // GitHub as reported: three ACTIVE, two INITIATED. The tile said five.
    await render([
      row("github", "GitHub", [
        acct("h1", true),
        acct("h2", true),
        acct("h3", true),
        acct("h4", false),
        acct("h5", false),
      ]),
    ]);
    expect(text()).toContain("3 accounts connected");
    expect(text()).not.toContain("5 accounts connected");
  });

  it("does not call a provider that holds three accounts 'not connected'", async () => {
    // Notion as reported: three INITIATED, none ACTIVE. The tile said
    // "not connected", two inches above the three accounts the page listed.
    await render([
      row("notion", "Notion", [acct("n1", false), acct("n2", false), acct("n3", false)]),
    ]);
    expect(text()).toContain("3 accounts, none connected");
    // The exact contradiction: the words the operator read while three accounts
    // sat below them. `toContain` would also match the new sentence's tail, so
    // this asserts the standalone phrase is gone.
    expect(text()).not.toMatch(/(^|[^,] )not connected/);
  });

  it("still says a provider with nothing connected is not connected", async () => {
    // The control. "none connected" must not leak onto a provider that holds no
    // accounts at all — that one really is unconnected, and the old wording is
    // the right wording for it.
    await render([row("slack", "Slack", [])]);
    expect(text()).toContain("not connected");
    expect(text()).not.toContain("none connected");
  });

  it("shows an unconnected provider's typical access before sign-in", async () => {
    await render([row("slack", "Slack", [])]);
    expect(text()).toContain("Typical access:");
  });
});

describe("a connected tile that does not deliver drops the success styling", () => {
  // A provider can be genuinely connected and still reach nobody when the grant
  // is withheld (issue #1407). The glyph and shell already demote; the account
  // line and the tile hover must follow, or the tile green-checks a connection
  // its own caption says delivers no tools.
  it("neutral-tones the account line and the hover, not just the glyph", async () => {
    await render([row("github", "GitHub", [acct("h1", true)])], { granted: false });

    // The caption states the demotion in words...
    expect(text()).toContain("tools not delivered");

    // ...and the account line under it wears the muted tone, never the success
    // colour the caption contradicts.
    const line = Array.from(container.querySelectorAll("span")).find((s) =>
      s.textContent?.includes("tools not delivered"),
    );
    expect(line?.className).toContain("text-muted-foreground");
    expect(line?.className).not.toContain("text-status-done-text");

    // The tile itself hovers neutral rather than flashing the success tint.
    const tile = container.querySelector("[data-testid='open-provider-github']");
    expect(tile?.className).toContain("hover:bg-muted/70");
    expect(tile?.className).not.toContain("hover:bg-status-done/10");
  });
});
