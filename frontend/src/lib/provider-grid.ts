// The OAuth page's ONE provider grid (issue #582).
//
// ## What this replaces
//
// The page used to render two provider lists and neither consulted the other:
//
//  1. `ComposioSection`'s tile grid, built from the backend's live Composio
//     catalog and its own `GET …/composio/connections` probe;
//  2. a categorised grid built from the eleven hardcoded `CONNECTION_PROVIDERS`
//     tiles, whose status came from `GET …/connections`.
//
// So Gmail appeared twice on one screen — connected in one grid, offering a
// Connect button in the other — and the lower grid's button was actionable, so
// an operator who trusted it would start a second sign-in for an account
// already connected.
//
// The two grids disagreeing was not a rendering accident. Each read a
// *different* route, and the two routes applied different rules to the same
// underlying Composio state: `GET …/connections` discarded it entirely unless
// the company explicitly granted the `composio` tool namespace, while
// `GET …/composio/connections` never consulted the grant. 13 of the 21 shipped
// companies grant no `composio`, so for most of them the contradiction was the
// steady state rather than a race. That gate is gone (see `composio_view` in
// `src/server/ops/connections_read.rs`), which is what makes one grid possible:
// `GET …/connections` is now a complete answer to "what is connected", across
// both namespaces, for every provider — not just the ones a manifest declared.
//
// ## The division of labour, now that there is one grid
//
//  - **What is connected** comes from `GET …/connections` alone. It is the only
//    route that reconciles the native `oauth/{provider}` catalog with Composio,
//    and it is the answer an agent's tool belt is built from.
//  - **What can be connected** comes from the backend's Composio catalog.
//  - **How to connect it** is `connectRoute`, unchanged and still the single
//    rule the tile renders and the click calls.
//
// `CONNECTION_PROVIDERS` is consequently no longer a *list* — the backend
// catalog is. It is metadata: which ids the host's `well_known` table keys, the
// Composio slug each maps to, and the brand colours.
//
// ## The native catalog stops being offerable (issue #822)
//
// It was still half a list here. Every `CONNECTION_PROVIDERS` tile the backend
// catalog did not cover was appended anyway — five of the eleven against the
// host's built-in starter list, all eleven against a host that answers no
// catalog at all — and each of them offered a Connect that #396 says confers
// nothing: `oauth/{provider}` is written by the callback and read by no agent
// tool. An operator on a self-hosted instance could register a provider
// application, complete a real handshake, see the tile go green, and have given
// their agents nothing. `connectRoute` no longer offers that route, and this
// file no longer offers the tile: a provider appears because the backend catalog
// carries it, not because the console has a logo for it.
//
// What is appended instead is narrower and load-bearing: **a provider the host
// reports as connected**, catalog or no catalog. Retracting an offer must not
// hide a credential the company already holds — the tile, its `via` and its
// Disconnect are how an operator sees and releases one. So the tail went from
// "everything we have metadata for" to "everything that is actually connected",
// which is the same union stated honestly: what can be connected, plus what is.

import type { ComposioConnectedAccount, ComposioToolkitEntry } from "@/api/composio";
import type { ConnectionState } from "@/api/types";
import {
  buildProviderRows,
  type ProviderRow,
} from "@/lib/composio-catalog";
import {
  CONNECTION_PROVIDERS,
  connectRoute,
  toolkitSlug,
  type ComposioReach,
  type ConnectRoute,
  type ConnectionProvider,
} from "@/lib/connections";

/** One tile in the merged grid: what to draw, and what a click does. */
export interface GridProvider extends ProviderRow {
  /**
   * The id the *host* knows this provider by — what `disconnectConnection` is
   * called with, and what a manifest declares.
   *
   * Distinct from {@link ProviderRow.slug}, which is Composio's spelling and
   * what `POST …/composio/authorize` takes. They differ for every hyphenated
   * tile (`google-calendar` / `googlecalendar`) and outright for `x` /
   * `twitter`, so collapsing them into one field would silently send one
   * namespace's key to the other's route.
   *
   * Resolved local metadata first, then **the host's own spelling from
   * `GET …/connections`**, then the slug. That middle step is what keeps
   * Disconnect working for a natively connected provider the console has no
   * tile for (issue #822): its row is now the only reason it has a tile at all,
   * and `DELETE …/connections/{provider}` has to name it the way the host does.
   * The slug fallback remains correct for the rest — a provider nothing local
   * and nothing connected names has no host id to give.
   */
  providerId: string;
  /** How this tile's button behaves — rendered from and acted on identically. */
  route: ConnectRoute;
  /** The namespaces reporting it connected. Empty when it is not. */
  via: ("native" | "composio")[];
  /**
   * A Composio path exists but could not be read, so `connected: false` means
   * "unknown", not "no". The tile must say so rather than painting a confident
   * disconnected state over an unanswered probe.
   */
  unverified: boolean;
  /** The connected account label, when the host knows one. Never a credential. */
  account?: string;
  /**
   * The Composio accounts this company holds for the toolkit (issue #404).
   *
   * Empty for a provider connected only natively, for a provider connected
   * through nothing, and on a host that predates the `accounts` field — the
   * three cases share one rendering, which is that there is no connection
   * object to open.
   *
   * Several entries is an ordinary state, not a misconfiguration: the
   * connections are the company owner's to grant, and an owner can hold two
   * Gmail accounts. #316 settled one account per *login*, which is a different
   * surface (`src/server/users/`) from which Gmail an agent acts through.
   */
  accounts: ComposioConnectedAccount[];
  /**
   * Whether this tile has something a disconnect can address.
   *
   * Two different routes sit behind this one flag, and they are not
   * interchangeable — see {@link disconnectRouteFor}. `POST
   * …/connections/{provider}/disconnect` blanks the host's own
   * `oauth/{provider}` secret; `DELETE …/composio/connections/{id}` revokes one
   * account at Composio. Sending a Composio-connected provider to the native
   * route blanks a secret it never had, reports success, and leaves the tile
   * connected on the next refresh.
   *
   * Until #696 the second route did not exist, so this was `via.includes(
   * "native")` and a Composio tile deliberately offered no Disconnect at all.
   */
  canDisconnect: boolean;
}

/**
 * Which route releases this tile, or `null` when nothing here can be released.
 *
 * `composio` carries the ids because the host addresses a revoke by connection
 * id — a toolkit with two accounts has two, and exactly one of them is the
 * operator's target. The caller decides how to spend that: one account revokes
 * directly, several is a question only the detail view can ask.
 */
export function disconnectRouteFor(
  row: GridProvider,
): { kind: "native" } | { kind: "composio"; accounts: ComposioConnectedAccount[] } | null {
  if (!row.connected) return null;
  const live = row.accounts.filter((a) => a.connected);
  // Composio first: a tile connected through both is connected through Composio
  // *for the agents*, which is the connection an operator means. The native
  // secret is inert until #396 lands, so blanking it silently would leave the
  // provider working and the tile still connected.
  //
  // The route this resolves to is the same under BYOK: the host revokes through
  // Composio's own `DELETE /connected_accounts/{id}` there rather than through
  // the backend, which is a difference in who is dialled, not in what the
  // console can offer.
  if (live.length > 0) return { kind: "composio", accounts: live };
  if (row.via.includes("native")) return { kind: "native" };
  return null;
}

/**
 * How many of a toolkit's accounts an agent can actually act as, and how many
 * exist but cannot be acted as yet.
 *
 * **The one rule for counting accounts** (issue #923). The page had two. The
 * tile grid printed `accounts.length` — every account, whatever its state —
 * under a badge gated on {@link GridProvider.connected}, which the host defines
 * as *at least one account is `ACTIVE`*. Mixing those two reads produced a page
 * that contradicted itself in both directions at once: a Gmail holding one
 * `ACTIVE` account and five `INITIATED` ones announced "6 accounts connected",
 * while a Notion holding three `INITIATED` ones and no `ACTIVE` one announced
 * "not connected" directly above the three accounts it holds.
 *
 * Both facts come from the same per-account `connected` flag the host already
 * sends, which it sets for `ACTIVE`/`CONNECTED` alone. Nothing new is derived
 * here; the two numbers are simply counted once, in one place, for every
 * surface that reports them.
 */
export interface AccountTally {
  /** Accounts an agent can act as — the host's per-account `connected`. */
  live: number;
  /**
   * Accounts the company holds that are not usable: mid-handshake
   * (`INITIATED`), lapsed (`EXPIRED`), or any other state Composio may add.
   *
   * Counted rather than named. The host forwards `status` verbatim and leaves
   * the vocabulary to the console precisely because it will not guess at values
   * it has not seen, and a summary line that had to name them would have to
   * guess too. What the operator needs from a tile is that these accounts
   * *exist* — the states themselves are listed per account below it.
   */
  pending: number;
}

/** Count a toolkit's accounts by whether an agent can act as them. */
export function tallyAccounts(accounts: ComposioConnectedAccount[] | undefined): AccountTally {
  const rows = accounts ?? [];
  const live = rows.filter((a) => a.connected).length;
  return { live, pending: rows.length - live };
}

/**
 * What a surface says about a toolkit's accounts, or `null` when it holds none
 * and the caller should fall back to its own wording.
 *
 * Shared by the tile grid and the provider detail panel so the two cannot drift
 * again (issue #923): they described the same accounts differently, and #582
 * had already had to remove a *second grid* for saying "connected" where the
 * first said otherwise. A third implementation of "connected" would not have
 * helped, so there is one.
 *
 * "none connected" is deliberate for a toolkit holding only unusable accounts.
 * "not connected" is what the tile said before, and it is false: an account
 * mid-handshake is not the absence of an account, and the operator reading it
 * could see three of them listed below. This states both halves — the accounts
 * are there, and none of them is usable yet.
 */
export function accountSummary(accounts: ComposioConnectedAccount[] | undefined): string | null {
  const { live, pending } = tallyAccounts(accounts);
  if (live > 0) {
    return live === 1 ? "1 account connected" : `${live} accounts connected`;
  }
  if (pending > 0) {
    return pending === 1 ? "1 account, not connected" : `${pending} accounts, none connected`;
  }
  return null;
}

/**
 * Whether the company grants the `composio` tool namespace, as a genuine
 * tri-state (issue #1478).
 *
 * `granted` arrives from an unvalidated `client.get` cast, so `undefined` is
 * reachable — an older host, or any response-shape drift. The bug this closes:
 * one surface defaulted `undefined` to "not granted" (a warning badge + banner)
 * while another defaulted it to "granted", so a single render showed both. Every
 * surface routes the field through here instead, so `undefined` reads as
 * `"unknown"` on all of them — never a definite answer.
 */
export type GrantStanding = "granted" | "not-granted" | "unknown";

export function grantStanding(granted: boolean | undefined): GrantStanding {
  if (granted === true) return "granted";
  if (granted === false) return "not-granted";
  return "unknown";
}

/**
 * The one count of connected providers (issue #1407).
 *
 * The header badge and the section heading each computed this independently from
 * the same array; deriving both from one function keeps them in step by
 * construction.
 */
export function connectedProviderCount(providers: GridProvider[]): number {
  return providers.filter((p) => p.connected).length;
}

/**
 * Whether a connected tile's tools actually reach teammates (issue #1407).
 *
 * A connection is real whether or not the `composio` namespace is granted — the
 * grant governs the tool belt, not the handshake — but a green "connected" tile
 * under a banner saying the tools reach nobody is the contradiction #1262/#1407
 * are about. A Composio-route tile whose grant is explicitly `false` is
 * connected-but-not-delivering, so it must not wear the success colour. An
 * `unknown` grant is NOT demoted: that would render an unchecked field as a
 * definite negative.
 */
export function tileDelivers(row: GridProvider, granted: boolean | undefined): boolean {
  if (!row.connected) return false;
  if (row.route.kind === "composio" && grantStanding(granted) === "not-granted") return false;
  return true;
}

/** The local tile for a Composio slug, when the console has metadata for one. */
function nativeTileFor(slug: string): ConnectionProvider | undefined {
  return CONNECTION_PROVIDERS.find((p) => toolkitSlug(p.toolkit) === slug);
}

/**
 * The local tile a host row names, matched in *either* spelling.
 *
 * `GET …/connections` keys a row by whichever namespace produced it, so `x` and
 * `twitter` are the same tile and only the local metadata knows it. Both the
 * status fold and the connected tail need that alias, and needed it identically.
 */
function tileForHostProvider(provider: string): ConnectionProvider | undefined {
  const key = toolkitSlug(provider);
  return CONNECTION_PROVIDERS.find(
    (p) => toolkitSlug(p.id) === key || toolkitSlug(p.toolkit) === key,
  );
}

/**
 * Collapse the host's connection rows into `slug -> connected`.
 *
 * Normalized and OR-ed rather than indexed, because the host can emit two rows
 * for one provider when an id and its Composio slug do not normalize alike: a
 * manifest declaring `provider = "x"` yields a disconnected `x` row while
 * Composio's connected state arrives separately as `twitter`. Both must land on
 * the same tile, and connected must win — the union the host already performs
 * within a row, extended across the alias it cannot see.
 */
function connectedBySlug(
  states: Readonly<Record<string, ConnectionState>>,
): Record<string, boolean> {
  const out: Record<string, boolean> = {};
  for (const state of Object.values(states)) {
    const key = toolkitSlug(state.provider);
    // Fold the alias too, so a `twitter` row also marks the `x` tile connected
    // for a console that keys the tile by its id rather than its slug.
    const tile = tileForHostProvider(state.provider);
    for (const alias of new Set([key, tile ? toolkitSlug(tile.toolkit) : key])) {
      out[alias] = out[alias] === true || state.connected;
    }
  }
  return out;
}

/**
 * The host's own spelling for each provider it reported, keyed by normalized
 * slug.
 *
 * `GET …/connections` answers under the manifest's spelling (`google-calendar`)
 * or Composio's (`googlecalendar`) depending on which namespace the row came
 * from, and `DELETE …/connections/{provider}` only accepts the former. First row
 * wins; a raw id and its normalized form are the same string in every case where
 * they differ only in case.
 */
function hostIdBySlug(states: Readonly<Record<string, ConnectionState>>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const state of Object.values(states)) {
    const key = toolkitSlug(state.provider);
    if (out[key] === undefined) out[key] = state.provider;
  }
  return out;
}

/**
 * Every host row that speaks about one tile, most-informative first.
 *
 * A connected row wins over a disconnected one for the alias reason above; among
 * equals the row carrying `via`/`account` is preferred, so a tile does not lose
 * its account label to an empty duplicate.
 */
function statesFor(
  row: { slug: string; providerId: string },
  states: Readonly<Record<string, ConnectionState>>,
): ConnectionState[] {
  const wanted = new Set([toolkitSlug(row.slug), toolkitSlug(row.providerId)]);
  return Object.values(states)
    .filter((s) => wanted.has(toolkitSlug(s.provider)))
    .sort((a, b) => {
      if (a.connected !== b.connected) return a.connected ? -1 : 1;
      return (b.via?.length ?? 0) - (a.via?.length ?? 0);
    });
}

/**
 * Build the page's single provider grid.
 *
 * `catalog` is the backend's Composio catalog (or the manifest allowlist, or a
 * flagged fallback — the host already decided which, and re-deciding here is how
 * the two lists drifted apart in the first place). `extra` carries slugs
 * connected through the by-slug escape hatch this session. `states` is
 * `GET …/connections`, the sole authority on connected. `reach` and
 * `platformManaged` feed `connectRoute`.
 *
 * `composioAccounts` is `GET …/composio/connections`, keyed by normalized
 * toolkit slug — the connection *objects* behind the booleans `states` already
 * reconciles (issue #404). It stays a separate argument rather than being
 * folded into `states`: `GET …/connections` is still the sole authority on
 * whether a provider is connected, and a second source answering that question
 * is precisely the shape of the bug #582 removed. This one answers a different
 * question — *which accounts*, so a revoke has something to address.
 *
 * The tail is the point of the union, and since #822 it is a *connected* tail:
 * a provider the host reports as connected gets a tile whether or not the
 * catalog offers one. It used to be every `CONNECTION_PROVIDERS` tile the
 * catalog missed, which is how a host the catalog does not cover came to offer
 * Connects for a route that stores a credential no agent reads (#396).
 * Dropping the offer must not drop the record: a company that connected Slack
 * through the hatch keeps its tile, its `via: ["native"]` and its Disconnect,
 * on a page that no longer invites anyone else to do the same.
 */
export function buildGridProviders(
  catalog: readonly ComposioToolkitEntry[],
  extra: readonly string[],
  states: Readonly<Record<string, ConnectionState>>,
  reach: ComposioReach | null,
  platformManaged: boolean,
  composioAccounts: Readonly<Record<string, ComposioConnectedAccount[]>> = {},
): GridProvider[] {
  const offered = new Set(catalog.map((entry) => toolkitSlug(entry.slug)));
  const connectedOnly: ComposioToolkitEntry[] = [];
  for (const state of Object.values(states)) {
    if (!state.connected) continue;
    // Local metadata decides the tile's spelling where there is any, so the two
    // rows a split alias produces (`x` and `twitter`) land on one tile rather
    // than two — the same fold `connectedBySlug` performs for the status.
    const tile = tileForHostProvider(state.provider);
    const slug = tile ? toolkitSlug(tile.toolkit) : toolkitSlug(state.provider);
    if (!slug || offered.has(slug)) continue;
    offered.add(slug);
    connectedOnly.push({
      slug,
      name: tile?.name ?? "",
      description: tile?.description ?? "",
      logo: null,
      categories: [],
    });
  }

  const rows = buildProviderRows(
    [...catalog, ...connectedOnly],
    extra,
    connectedBySlug(states),
  );
  const hostIds = hostIdBySlug(states);

  return rows.map((row) => {
    const tile = nativeTileFor(row.slug);
    const providerId = tile?.id ?? hostIds[row.slug] ?? row.slug;
    const matched = statesFor({ slug: row.slug, providerId }, states);
    const state = matched[0];
    // A tile the host said nothing about still inherits the instance-level
    // `attested` fact — it is a property of the pod, not of one provider.
    const effective =
      state ?? (platformManaged ? ({ credentialSource: "attested" } as const) : undefined);
    // Union across every row that speaks about this tile: the host reconciles
    // within a row, and the alias split means there can be two.
    const via = [...new Set(matched.flatMap((s) => s.via ?? []))];
    // Looked up under both spellings for the same reason `statesFor` is: the
    // tile's Composio slug and the id the host knows it by diverge for every
    // hyphenated provider, and `x` / `twitter` outright.
    const accounts =
      composioAccounts[toolkitSlug(row.slug)] ?? composioAccounts[toolkitSlug(providerId)] ?? [];
    const grid: GridProvider = {
      ...row,
      providerId,
      route: connectRoute({ toolkit: row.slug }, effective, reach),
      via,
      // Only unknown if nothing already answered yes: a provider we know is
      // connected needs no second opinion.
      unverified: !row.connected && matched.some((s) => s.unverified === true),
      // Prefer the host's reconciled label, then a Composio account's own. A
      // tile with two accounts shows neither here — one of two labels on a
      // tile reads as "this is the account", which is the question the detail
      // view exists to answer properly.
      account:
        matched.find((s) => s.account)?.account ??
        (accounts.length === 1 ? accounts[0].account : undefined),
      accounts,
      canDisconnect: false,
    };
    return { ...grid, canDisconnect: disconnectRouteFor(grid) !== null };
  });
}
