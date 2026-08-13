// The Connections page's ONE provider grid (issue #582).
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
//  - **What can be connected** comes from the backend's Composio catalog, plus
//    the local `CONNECTION_PROVIDERS` tiles for the native-only hatch.
//  - **How to connect it** is `connectRoute`, unchanged and still the single
//    rule the tile renders and the click calls.
//
// `CONNECTION_PROVIDERS` is consequently no longer a *list* — the backend
// catalog is. It is metadata for the native route: which ids the host's
// `well_known` table can run an OAuth handshake for, and the brand colours for
// tiles the backend catalog does not cover.

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
   * The id the *host* knows this provider by — what `startConnection` and
   * `disconnectConnection` are called with, and what a manifest declares.
   *
   * Distinct from {@link ProviderRow.slug}, which is Composio's spelling and
   * what `POST …/composio/authorize` takes. They differ for every hyphenated
   * tile (`google-calendar` / `googlecalendar`) and outright for `x` /
   * `twitter`, so collapsing them into one field would silently send one
   * namespace's key to the other's route. Falls back to the slug for a provider
   * the local catalog has no tile for — which is most of them, and is correct:
   * a provider with no native metadata has no native route to name.
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
  if (live.length > 0) return { kind: "composio", accounts: live };
  if (row.via.includes("native")) return { kind: "native" };
  return null;
}

/** The local tile for a Composio slug, when the console has metadata for one. */
function nativeTileFor(slug: string): ConnectionProvider | undefined {
  return CONNECTION_PROVIDERS.find((p) => toolkitSlug(p.toolkit) === slug);
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
    const tile = CONNECTION_PROVIDERS.find(
      (p) => toolkitSlug(p.id) === key || toolkitSlug(p.toolkit) === key,
    );
    for (const alias of new Set([key, tile ? toolkitSlug(tile.toolkit) : key])) {
      out[alias] = out[alias] === true || state.connected;
    }
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
 * The tail is the point of the union: `CONNECTION_PROVIDERS` tiles whose toolkit
 * the catalog did not offer are appended anyway. A host with Composio switched
 * off returns an empty catalog, and without this the page would render nothing
 * at all rather than the eleven native tiles a self-hoster can genuinely use.
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
  const nativeOnly: ComposioToolkitEntry[] = CONNECTION_PROVIDERS.filter(
    (p) => !offered.has(toolkitSlug(p.toolkit)),
  ).map((p) => ({
    slug: p.toolkit,
    name: p.name,
    description: p.description,
    logo: null,
    categories: [],
  }));

  const rows = buildProviderRows(
    [...catalog, ...nativeOnly],
    extra,
    connectedBySlug(states),
  );

  return rows.map((row) => {
    const tile = nativeTileFor(row.slug);
    const providerId = tile?.id ?? row.slug;
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
