// The Connections page's one provider grid (issue #582).
//
// The bug these pin: the page rendered two provider lists from two routes that
// applied different rules to the same Composio state, so Gmail could show as
// connected in one and offer an actionable Connect button in the other, on one
// screen. These tests are about the merge that leaves one list — that a provider
// appears exactly once, that its connected state has a single source, and that
// no action is offered which the host cannot carry out.

import { describe, expect, it } from "vitest";

import type { ComposioConnectedAccount, ComposioToolkitEntry } from "@/api/composio";
import type { ConnectionState } from "@/api/types";
import {
  accountSummary,
  buildGridProviders,
  connectedProviderCount,
  disconnectRouteFor,
  grantStanding,
  tallyAccounts,
  tileDelivers,
} from "@/lib/provider-grid";
import { CONNECTION_PROVIDERS, type ComposioReach } from "@/lib/connections";

/** A host where Composio is live and everything is reachable (open mode). */
const OPEN: ComposioReach = {
  inBuild: true,
  granted: true,
  hasCredential: true,
  openMode: true,
  effectiveToolkits: [],
};

function entry(slug: string, over: Partial<ComposioToolkitEntry> = {}): ComposioToolkitEntry {
  return { slug, name: "", description: "", logo: null, categories: [], ...over };
}

function states(...rows: ConnectionState[]): Record<string, ConnectionState> {
  return Object.fromEntries(rows.map((r) => [r.provider, r]));
}

function bySlug(providers: ReturnType<typeof buildGridProviders>, slug: string) {
  const found = providers.filter((p) => p.slug === slug);
  expect(found.length, `expected exactly one ${slug} tile, got ${found.length}`).toBe(1);
  return found[0];
}

describe("buildGridProviders", () => {
  it("renders a provider exactly once even when the host reports it twice", () => {
    // The host emits two rows for one provider whenever an id and its Composio
    // slug do not normalize alike: a manifest declaring `x` yields a
    // disconnected `x` row while Composio's connected state arrives as
    // `twitter`. Two rows must still be one tile — this is acceptance criterion
    // 1, and the failure it guards is the whole issue.
    const providers = buildGridProviders(
      [entry("twitter")],
      [],
      states(
        { provider: "x", connected: false },
        { provider: "twitter", connected: true, via: ["composio"] },
      ),
      OPEN,
      false,
    );
    const tile = bySlug(providers, "twitter");
    expect(tile.connected).toBe(true);
    expect(tile.via).toEqual(["composio"]);
  });

  it("takes connected from the host's reconciled rows, not from a second probe", () => {
    const providers = buildGridProviders(
      [entry("gmail"), entry("slack")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"], account: "ops@acme.test" }),
      OPEN,
      false,
    );
    expect(bySlug(providers, "gmail").connected).toBe(true);
    expect(bySlug(providers, "gmail").account).toBe("ops@acme.test");
    // Absent from `/connections` means not connected — the single answer, with
    // no second list to contradict it.
    expect(bySlug(providers, "slack").connected).toBe(false);
  });

  it("offers no connect action for an already-connected provider", () => {
    // Acceptance criterion 3. A connected tile is not a Connect affordance, so
    // an operator cannot start a second sign-in for one account — the shape
    // #396 had to clean up once.
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
    );
    expect(bySlug(providers, "gmail").connected).toBe(true);
  });

  it("matches a hyphenated console id to its Composio slug", () => {
    const providers = buildGridProviders(
      [entry("googlecalendar")],
      [],
      states({ provider: "googlecalendar", connected: true, via: ["composio"] }),
      OPEN,
      false,
    );
    const tile = bySlug(providers, "googlecalendar");
    expect(tile.connected).toBe(true);
    // The host id stays distinct from the Composio slug: one is what
    // `disconnectConnection` takes for a historical credential, the other is
    // what `composio/authorize` takes, and sending either to the other's route fails.
    expect(tile.providerId).toBe("google-calendar");
  });

  it("offers no tile at all when the host's catalog is empty", () => {
    // Issue #822, and the assertion this replaces said the opposite: every one
    // of the eleven local tiles survived an empty catalog, so a self-hoster with
    // Composio switched off saw a full page of Connect buttons. Each of those
    // completed a real handshake and stored `oauth/{provider}` — which no agent
    // tool reads (#396). The honest render of an inert route is no tile.
    expect(buildGridProviders([], [], {}, null, false)).toEqual([]);
    for (const local of CONNECTION_PROVIDERS) {
      expect(
        buildGridProviders([], [], {}, OPEN, false).some((p) => p.slug === local.toolkit),
        `${local.id} is still offered from local metadata alone`,
      ).toBe(false);
    }
  });

  it("keeps a natively connected provider listed, with its Disconnect", () => {
    // The half that must NOT go with the offer. Retracting an invitation is not
    // permission to hide a credential the company already stored: this is the
    // only surface that shows it exists and the only one that releases it.
    const providers = buildGridProviders(
      [],
      [],
      states({
        provider: "slack",
        connected: true,
        via: ["native"],
        credentialSource: "static",
        account: "acme-workspace",
      }),
      null,
      false,
    );
    const tile = bySlug(providers, "slack");
    expect(tile.connected).toBe(true);
    expect(tile.via).toEqual(["native"]);
    expect(tile.canDisconnect).toBe(true);
    expect(tile.account).toBe("acme-workspace");
    // And the host's own id, which is what `DELETE …/connections/{provider}`
    // takes — the tile exists because of that row, so it must name it.
    expect(tile.providerId).toBe("slack");
  });

  it("lists a connected provider the console has no metadata for", () => {
    // The tail is now "what is connected", not "what we have a logo for", so a
    // provider only the manifest ever named still gets a tile — and a Disconnect
    // addressed the way the host spells it, not the way the slug normalizes.
    const providers = buildGridProviders(
      [],
      [],
      states({ provider: "zoom-pro", connected: true, via: ["native"] }),
      null,
      false,
    );
    const tile = bySlug(providers, "zoompro");
    expect(tile.connected).toBe(true);
    expect(tile.canDisconnect).toBe(true);
    expect(tile.providerId).toBe("zoom-pro");
  });

  it("does not list a provider the host merely answered about", () => {
    // A disconnected row is the host saying "not connected", which is the same
    // information as the tile's absence — and listing it would put the offer
    // back under a different name.
    expect(
      buildGridProviders([], [], states({ provider: "slack", connected: false }), null, false),
    ).toEqual([]);
  });

  it("folds a connected alias into one tile rather than appending a second", () => {
    // The `x` / `twitter` split, now that a connected row can create a tile of
    // its own: both rows describe one provider, and the local metadata is what
    // says so. Two tiles here would reintroduce the duplicate #582 removed.
    const providers = buildGridProviders(
      [],
      [],
      states(
        { provider: "x", connected: false },
        { provider: "twitter", connected: true, via: ["composio"] },
      ),
      null,
      false,
    );
    expect(providers).toHaveLength(1);
    expect(providers[0].slug).toBe("twitter");
    expect(providers[0].providerId).toBe("x");
  });

  it("does not duplicate a local tile the backend catalog also offers", () => {
    const providers = buildGridProviders([entry("gmail", { name: "Gmail" })], [], {}, OPEN, false);
    expect(providers.filter((p) => p.slug === "gmail")).toHaveLength(1);
  });

  it("offers Disconnect only where the host holds something to release", () => {
    // `DELETE …/connections/{provider}` blanks the host's own `oauth/{provider}`
    // secret; no host route releases a Composio connection. A Disconnect on a
    // Composio-only tile would report success and leave it connected on the next
    // refresh — the same contradiction in a different place.
    const providers = buildGridProviders(
      [entry("gmail"), entry("slack")],
      [],
      states(
        { provider: "gmail", connected: true, via: ["composio"] },
        { provider: "slack", connected: true, via: ["native"] },
      ),
      OPEN,
      false,
    );
    expect(bySlug(providers, "gmail").canDisconnect).toBe(false);
    expect(bySlug(providers, "slack").canDisconnect).toBe(true);
  });

  it("reports an unread Composio probe as unknown rather than disconnected", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: false, unverified: true }),
      OPEN,
      false,
    );
    expect(bySlug(providers, "gmail").unverified).toBe(true);
  });

  it("does not call a connected provider unverified", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states(
        { provider: "gmail", connected: true, via: ["native"] },
        { provider: "gmail-alias", connected: false, unverified: true },
      ),
      OPEN,
      false,
    );
    expect(bySlug(providers, "gmail").unverified).toBe(false);
  });

  it("routes an undeclared provider through Composio on a platform-managed host", () => {
    // Issue #599's case, carried over: `GET …/connections` answers only for
    // declared providers, so a tenant declaring none leaves every tile with no
    // row of its own. The instance-level `attested` fact still applies.
    const providers = buildGridProviders([entry("gmail")], [], {}, OPEN, true);
    expect(bySlug(providers, "gmail").route).toEqual({ kind: "composio", toolkit: "gmail" });
  });

  it("says a platform-managed tile is managed when Composio cannot reach it", () => {
    const providers = buildGridProviders([entry("gmail")], [], {}, null, true);
    expect(bySlug(providers, "gmail").route).toEqual({ kind: "managed" });
  });

  it("gives a slug from the by-slug hatch a tile of its own", () => {
    const providers = buildGridProviders([], ["hubspot"], {}, OPEN, false);
    expect(bySlug(providers, "hubspot").route).toEqual({ kind: "composio", toolkit: "hubspot" });
  });

  it("orders connected providers first", () => {
    const providers = buildGridProviders(
      [entry("zendesk"), entry("gmail"), entry("stripe")],
      [],
      states({ provider: "stripe", connected: true, via: ["composio"] }),
      OPEN,
      false,
    );
    expect(providers[0].slug).toBe("stripe");
  });

  it("connects a provider whose company does not grant the composio namespace", () => {
    // The heart of #582 on this side of the wire. The grant governs whether
    // agents receive Composio tools; it never governed whether a sign-in can
    // complete. Gating the route on it — while the other list did not — is what
    // made one page say both things.
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      { ...OPEN, granted: false },
      false,
    );
    const tile = bySlug(providers, "gmail");
    expect(tile.connected).toBe(true);
    expect(bySlug(buildGridProviders([entry("slack")], [], {}, { ...OPEN, granted: false }, false), "slack").route).toEqual({
      kind: "composio",
      toolkit: "slack",
    });
  });
});

// The connection as an object rather than a boolean (issue #404).
//
// The bug these pin is one wire: every Disconnect on this page went to
// `POST …/connections/{provider}/disconnect`, which blanks the host's own
// `oauth/{provider}` secret. A Composio-connected provider never had one, so the
// call answered 200, the toast said "Disconnected Gmail", and Gmail was still
// connected on the next refresh. `DELETE …/composio/connections/{id}` (PR #696)
// is the route that releases it, and it is addressed by account id.
describe("disconnect routing", () => {
  function account(over: Partial<ComposioConnectedAccount> = {}): ComposioConnectedAccount {
    return { id: "conn-1", status: "ACTIVE", connected: true, ...over };
  }

  it("routes a Composio-connected provider to its account, not to the native blank", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
      { gmail: [account({ id: "conn-gmail-1", account: "ops@acme.test" })] },
    );
    const tile = bySlug(providers, "gmail");
    expect(tile.canDisconnect).toBe(true);
    expect(disconnectRouteFor(tile)).toEqual({
      kind: "composio",
      accounts: [account({ id: "conn-gmail-1", account: "ops@acme.test" })],
    });
  });

  // BYOK changes who the host dials to revoke — Composio's own
  // `DELETE /connected_accounts/{id}` rather than the backend's route — and not
  // whether the console can offer it. The grid is therefore identical in both
  // modes, which is what this pins: the route is resolved from what the tile is
  // connected through, never from which Composio the company uses.
  it("offers the Composio disconnect whichever Composio the company uses", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
      { gmail: [account({ id: "conn-gmail-1", account: "ops@acme.test" })] },
    );
    const tile = bySlug(providers, "gmail");
    expect(tile.canDisconnect).toBe(true);
    expect(disconnectRouteFor(tile)?.kind).toBe("composio");
  });

  it("keeps the native route for a provider only the host itself holds", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["native"] }),
      OPEN,
      false,
    );
    expect(disconnectRouteFor(bySlug(providers, "gmail"))).toEqual({ kind: "native" });
  });

  it("prefers the Composio account when a provider is connected through both", () => {
    // The native secret is inert until #396 lands, so blanking it would leave
    // the provider working, the agents' tools intact, and the tile connected —
    // a disconnect that reports success and releases nothing.
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["native", "composio"] }),
      OPEN,
      false,
      { gmail: [account()] },
    );
    expect(disconnectRouteFor(bySlug(providers, "gmail"))?.kind).toBe("composio");
  });

  it("offers nothing for a provider connected through Composio on a host predating #696", () => {
    // No `accounts` on the wire means no id, so there is nothing a revoke can
    // be addressed to. Drawing a Disconnect anyway is the exact defect above.
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
    );
    const tile = bySlug(providers, "gmail");
    expect(tile.accounts).toEqual([]);
    expect(tile.canDisconnect).toBe(false);
    expect(disconnectRouteFor(tile)).toBeNull();
  });

  it("ignores an account Composio no longer reports as connected", () => {
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
      { gmail: [account({ status: "EXPIRED", connected: false })] },
    );
    const tile = bySlug(providers, "gmail");
    // Still listed — the detail view shows it, and "set up and since expired"
    // is a state an operator has to be able to see and clear.
    expect(tile.accounts).toHaveLength(1);
    expect(disconnectRouteFor(tile)).toBeNull();
  });

  it("has nothing to release for a provider that is not connected", () => {
    const providers = buildGridProviders([entry("gmail")], [], {}, OPEN, false);
    expect(disconnectRouteFor(bySlug(providers, "gmail"))).toBeNull();
  });

  it("finds the accounts of a provider whose host id differs from its slug", () => {
    // `google-calendar` (host) vs `googlecalendar` (Composio). Keying the
    // account map by one spelling and looking it up by the other is how a
    // connected provider ends up with no object to open.
    const providers = buildGridProviders(
      [entry("googlecalendar")],
      [],
      states({ provider: "google-calendar", connected: true, via: ["composio"] }),
      OPEN,
      false,
      { googlecalendar: [account({ id: "conn-cal-1" })] },
    );
    expect(bySlug(providers, "googlecalendar").accounts).toHaveLength(1);
  });

  it("shows no account label on a tile holding two of them", () => {
    // One of two labels on a tile reads as "this is the account it acts as",
    // which is precisely the claim nothing here can back: `composio_execute`
    // sends no connection id, so which one Composio resolves is not ours to
    // report. The count is true; a name would not be.
    const providers = buildGridProviders(
      [entry("gmail")],
      [],
      states({ provider: "gmail", connected: true, via: ["composio"] }),
      OPEN,
      false,
      {
        gmail: [
          account({ id: "conn-gmail-1", account: "ops@acme.test" }),
          account({ id: "conn-gmail-2", account: "billing@acme.test" }),
        ],
      },
    );
    const tile = bySlug(providers, "gmail");
    expect(tile.account).toBeUndefined();
    expect(tile.accounts).toHaveLength(2);
  });
});

// The one rule for counting accounts (issue #923).
//
// The bug these pin is #582's shape one level down. #582 removed a second
// *grid*; this is two rules meeting inside the surviving one. The tile printed
// `accounts.length` — every account, whatever its state — under a badge gated
// on `connected`, which the host sets when at least one account is `ACTIVE`.
// The two reads disagreed in both directions on one screen, and the account
// list two inches below the grid showed the operator that they did.
//
// The counts below are the three rows the issue reported, verbatim.
describe("accountSummary", () => {
  function acct(id: string, connected: boolean): ComposioConnectedAccount {
    return { id, status: connected ? "ACTIVE" : "INITIATED", connected };
  }

  it("counts only the accounts an agent can act as", () => {
    // Gmail as reported: one ACTIVE, five INITIATED. The grid said "6 accounts
    // connected" — it counted the five that no agent can use.
    const gmail = [
      acct("g1", true),
      acct("g2", false),
      acct("g3", false),
      acct("g4", false),
      acct("g5", false),
      acct("g6", false),
    ];
    expect(tallyAccounts(gmail)).toEqual({ live: 1, pending: 5 });
    expect(accountSummary(gmail)).toBe("1 account connected");

    // GitHub as reported: three ACTIVE, two INITIATED. The grid said five.
    const github = [
      acct("h1", true),
      acct("h2", true),
      acct("h3", true),
      acct("h4", false),
      acct("h5", false),
    ];
    expect(tallyAccounts(github)).toEqual({ live: 3, pending: 2 });
    expect(accountSummary(github)).toBe("3 accounts connected");
  });

  it("says a provider holds accounts even when none of them is usable", () => {
    // Notion as reported: three INITIATED, none ACTIVE. The grid collapsed this
    // to "not connected" — directly above the three accounts it listed. An
    // account mid-handshake is not the absence of an account, which is the
    // distinction the issue's Expected asks the grid to keep.
    const notion = [acct("n1", false), acct("n2", false), acct("n3", false)];
    expect(tallyAccounts(notion)).toEqual({ live: 0, pending: 3 });
    expect(accountSummary(notion)).toBe("3 accounts, none connected");
    expect(accountSummary(notion)).not.toBe("not connected");
  });

  it("leaves the wording to the caller when the host sent no accounts", () => {
    // A host predating #404 answers without `accounts` while still reporting
    // the toolkit connected. `null` is what lets the tile keep saying
    // "connected via composio" rather than inventing a count of zero.
    expect(accountSummary(undefined)).toBeNull();
    expect(accountSummary([])).toBeNull();
    expect(tallyAccounts(undefined)).toEqual({ live: 0, pending: 0 });
  });

  it("uses the singular for one of each", () => {
    expect(accountSummary([acct("a", true)])).toBe("1 account connected");
    expect(accountSummary([acct("a", false)])).toBe("1 account, not connected");
  });
});

/**
 * The composio-grant tri-state (issue #1478).
 *
 * The bug this pins: `granted` arrives from an unvalidated cast, so `undefined`
 * is reachable, and two surfaces defaulted it in OPPOSITE directions — one to
 * "not granted", one to "granted" — so a single render showed both. Routing
 * every surface through `grantStanding` makes `undefined` read as `"unknown"`
 * everywhere, so they cannot disagree on the same field.
 */
describe("grantStanding", () => {
  it("maps an explicit boolean to its standing", () => {
    expect(grantStanding(true)).toBe("granted");
    expect(grantStanding(false)).toBe("not-granted");
  });

  it("reads a missing grant as unknown — never as a definite negative", () => {
    expect(grantStanding(undefined)).toBe("unknown");
  });
});

describe("connectedProviderCount", () => {
  it("counts the connected tiles, so the header and heading share one number", () => {
    const providers = buildGridProviders(
      [entry("gmail"), entry("slack"), entry("notion")],
      [],
      states(
        { provider: "gmail", connected: true, via: ["composio"] },
        { provider: "slack", connected: true, via: ["composio"] },
      ),
      OPEN,
      false,
    );
    expect(connectedProviderCount(providers)).toBe(2);
  });
});

/**
 * Whether a connected tile actually delivers its tools (issue #1407).
 *
 * A connection is real whether or not the `composio` namespace is granted, but a
 * green "connected" tile under a banner saying the tools reach nobody is the
 * contradiction. A not-granted Composio tile is connected-but-not-delivering; an
 * unchecked grant is NOT demoted (that would render unknown as a definite no).
 */
describe("tileDelivers", () => {
  function gmail(reach: ComposioReach) {
    return bySlug(
      buildGridProviders(
        [entry("gmail")],
        [],
        states({ provider: "gmail", connected: true, via: ["composio"] }),
        reach,
        false,
      ),
      "gmail",
    );
  }

  it("delivers when a connected composio tile's grant is present", () => {
    expect(tileDelivers(gmail(OPEN), true)).toBe(true);
  });

  it("does NOT deliver when the composio grant is explicitly absent", () => {
    // The #1407 case: real connection, no grant — connected-but-not-delivering,
    // so the tile must drop the success colour.
    expect(tileDelivers(gmail({ ...OPEN, granted: false }), false)).toBe(false);
  });

  it("keeps delivering when the grant is unknown — unknown is not a denial", () => {
    expect(tileDelivers(gmail(OPEN), undefined)).toBe(true);
  });

  it("a disconnected tile never delivers", () => {
    const disconnected = bySlug(
      buildGridProviders([entry("slack")], [], states(), OPEN, false),
      "slack",
    );
    expect(tileDelivers(disconnected, true)).toBe(false);
  });
});
