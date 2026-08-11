// The catalog of third-party accounts a company can act through. This is the
// console's view of what *can* be connected; whether a host can actually run
// the OAuth handshake depends on its `/connections` surface (see the client).
//
// ## Two routes, one tile (issue #599)
//
// A tile can be connected two ways, and which one is live is a property of the
// *host*, not of the tile:
//
//  - **Composio** — the hosted path. Composio runs the OAuth itself and the
//    resulting connection is a tool belt the agents actually receive.
//  - **Native** — this host's own registered provider application, the
//    self-hosted hatch documented on `src/server/ops/connections.rs`. Reported
//    as `credentialSource: "static"`.
//
// Until #599 every tile hard-routed to native. On a hosted tenant no
// `OPENCOMPANY_OAUTH_*` variable is injected, so all eleven Connect buttons
// 400'd with "provider is not enabled on this host" — a grid of buttons that
// could never succeed. [`connectRoute`] is now the single place that decides,
// and it can answer "neither", so a button that cannot work is never rendered.

export type ConnectionCategory =
  | "Communication"
  | "Productivity"
  | "Developer"
  | "Finance"
  | "Social"
  | "Storage";

export interface ConnectionProvider {
  /**
   * The provider identity in *this host's* namespace: the manifest
   * `[[connection]] provider = "…"` and the key `GET …/connections` reports
   * status under.
   *
   * It is also the token the native hatch resolves — `startConnection(id)` needs
   * a matching `well_known(id)` key in `src/server/ops/connections.rs` (today:
   * `slack`, `github`, `google`, `gmail`). An id outside that set has **no
   * native route**, which is no longer a dead tile: {@link connectRoute} sends
   * it down the Composio path instead, and reports `unavailable` when neither
   * route is open rather than rendering a Connect that 400s.
   */
  id: string;
  /**
   * The Composio toolkit slug this tile authorizes against — the hosted route,
   * and the only route on a hosted tenant.
   *
   * Composio slugs are lowercase and unpunctuated (`googlecalendar`) while ids
   * here are hyphenated (`google-calendar`), and a few differ outright (`x` is
   * `twitter`). Stated per tile rather than derived, because {@link toolkitSlug}
   * normalization alone cannot produce `twitter` from `x`.
   *
   * Mirrors the backend's `toolkit_slug()`
   * (`src/server/ops/connections_read.rs`), which is what reconciles the two
   * namespaces into one row per provider.
   */
  toolkit: string;
  name: string;
  description: string;
  category: ConnectionCategory;
  /** Brand-ish color for the provider's monogram tile. */
  color: string;
  /** Short glyph for the tile (1–2 chars). Falls back to the name initial. */
  glyph?: string;
}

export const CONNECTION_PROVIDERS: ConnectionProvider[] = [
  {
    id: "gmail",
    toolkit: "gmail",
    name: "Gmail",
    description: "Send and read email from a connected inbox.",
    category: "Communication",
    color: "#EA4335",
    glyph: "M",
  },
  {
    id: "slack",
    toolkit: "slack",
    name: "Slack",
    description: "Post updates and take requests from your workspace.",
    category: "Communication",
    color: "#4A154B",
    glyph: "#",
  },
  {
    id: "google-calendar",
    toolkit: "googlecalendar",
    name: "Google Calendar",
    description: "Schedule and read events on a shared calendar.",
    category: "Productivity",
    color: "#4285F4",
    glyph: "31",
  },
  {
    id: "notion",
    toolkit: "notion",
    name: "Notion",
    description: "Read and write docs and databases.",
    category: "Productivity",
    color: "#0F0F0F",
    glyph: "N",
  },
  {
    id: "google-drive",
    toolkit: "googledrive",
    name: "Google Drive",
    description: "Store and retrieve files and deliverables.",
    category: "Storage",
    color: "#1FA463",
    glyph: "△",
  },
  {
    id: "dropbox",
    toolkit: "dropbox",
    name: "Dropbox",
    description: "Sync assets and shared folders.",
    category: "Storage",
    color: "#0061FF",
    glyph: "▽",
  },
  {
    id: "github",
    toolkit: "github",
    name: "GitHub",
    description: "Open issues and pull requests in your repos.",
    category: "Developer",
    color: "#181717",
    glyph: "GH",
  },
  {
    id: "stripe",
    toolkit: "stripe",
    name: "Stripe",
    description: "Create invoices and read payment activity.",
    category: "Finance",
    color: "#635BFF",
    glyph: "S",
  },
  {
    id: "hubspot",
    toolkit: "hubspot",
    name: "HubSpot",
    description: "Sync contacts and deals in your CRM.",
    category: "Finance",
    color: "#FF7A59",
    glyph: "H",
  },
  {
    // Composio still spells this toolkit `twitter`; the tile keeps the current
    // product name. Exactly the case a normalization rule cannot derive.
    id: "x",
    toolkit: "twitter",
    name: "X",
    description: "Publish posts and read mentions.",
    category: "Social",
    color: "#000000",
    glyph: "X",
  },
  {
    id: "linkedin",
    toolkit: "linkedin",
    name: "LinkedIn",
    description: "Publish updates and manage your page.",
    category: "Social",
    color: "#0A66C2",
    glyph: "in",
  },
];

export const CONNECTION_CATEGORY_ORDER: ConnectionCategory[] = [
  "Communication",
  "Productivity",
  "Developer",
  "Finance",
  "Social",
  "Storage",
];

// ---------------------------------------------------------------------------
// Which route a tile's Connect takes (issue #599)
// ---------------------------------------------------------------------------

/**
 * Normalize a provider id or toolkit slug to one comparable key.
 *
 * The console spells ids hyphenated (`google-calendar`), Composio spells slugs
 * unpunctuated (`googlecalendar`), and `GET …/connections` returns rows keyed
 * either way — manifest rows under the manifest's spelling, reconciled Composio
 * rows under the Composio slug. Matching raw strings therefore misses a
 * genuinely connected provider and leaves its tile showing Connect.
 *
 * Mirrors `toolkit_slug()` in `src/server/ops/connections_read.rs`; keep the two
 * in step.
 */
export function toolkitSlug(value: string): string {
  return value.replace(/[^a-zA-Z0-9]/g, "").toLowerCase();
}

/**
 * The host's status row for a tile, matched across both spellings.
 *
 * Tries the tile's own id first (a manifest row is keyed that way), then falls
 * back to comparing normalized keys so a reconciled Composio row keyed
 * `googlecalendar` still finds the `google-calendar` tile.
 */
export function connectionStateFor<T extends { provider: string }>(
  provider: ConnectionProvider,
  states: Record<string, T>,
): T | undefined {
  const direct = states[provider.id];
  if (direct) return direct;
  const wanted = new Set([toolkitSlug(provider.id), toolkitSlug(provider.toolkit)]);
  return Object.values(states).find((state) => wanted.has(toolkitSlug(state.provider)));
}

/** What this host offers for Composio, as far as routing a tile is concerned. */
export interface ComposioReach {
  /** Whether the `composio` feature is compiled into this build. */
  inBuild: boolean;
  /** Whether the company explicitly grants `composio`. */
  granted: boolean;
  /** Whether a credential of any tier resolves; `none` means nothing to authorize against. */
  hasCredential: boolean;
  /** Open mode — the backend's own allowlist governs, so any slug it permits is reachable. */
  openMode: boolean;
  /** The toolkits offered as rows; the hard limit when not in open mode. */
  effectiveToolkits: readonly string[];
}

/**
 * Whether `toolkit` can actually be authorized against Composio on this host.
 *
 * The allowlist half matters as much as the credential half: outside open mode
 * the manifest list is a real limit, so offering a Connect for a toolkit outside
 * it would just move the 400 from one backend to the other. In open mode the
 * effective list is a *display* list, not a limit — any slug the backend permits
 * is reachable — so it is deliberately not consulted (issue #397).
 */
export function composioCanAuthorize(reach: ComposioReach | null, toolkit: string): boolean {
  if (!reach || !reach.inBuild || !reach.granted || !reach.hasCredential) return false;
  if (reach.openMode) return true;
  const wanted = toolkitSlug(toolkit);
  return reach.effectiveToolkits.some((slug) => toolkitSlug(slug) === wanted);
}

/**
 * How a tile's Connect should behave.
 *
 * - `native` — this host has its own registered provider application (or the
 *   company already stored a token): the self-hosted hatch, unchanged.
 * - `composio` — authorize `toolkit` through Composio's hosted OAuth.
 * - `managed` — the platform runs connections for this instance and there is no
 *   Composio route either; nothing to do here.
 * - `unavailable` — no route can succeed, so the tile says so instead of
 *   offering a button that fails.
 */
export type ConnectRoute =
  | { kind: "native" }
  | { kind: "composio"; toolkit: string }
  | { kind: "managed" }
  | { kind: "unavailable" };

/**
 * Decide the route for one tile — the single rule the grid renders *and* acts
 * on, so the button shown and the call made can never disagree.
 *
 * Precedence, and why:
 *
 * 1. **`static` → native.** The host registered a provider application for this
 *    provider, or the company stored its own token. Both are deliberate acts by
 *    the operator; preferring Composio here would quietly take away the hatch
 *    they configured. This is the self-hosted route staying supported.
 * 2. **Composio, when it can authorize this toolkit.** The hosted path, and the
 *    only one on a tenant — which is injected no `OPENCOMPANY_OAUTH_*` variable
 *    at all. It is also the route that makes the connection a capability: a
 *    native connection is recorded against the company but no agent tool reads
 *    it (see the catalog advisory in `ConnectionsView`).
 * 3. **`attested` → managed.** A platform-projected identity, so connections are
 *    the platform's to run and no local Connect could work.
 * 4. **Otherwise unavailable.** Notably this is where an unknown provider lands
 *    on a host with no Composio: `credentialSource` is `undefined` because the
 *    manifest never declared it, and a Connect would 400.
 *
 * Step 4 is the bug #599 reports. The grid renders every catalog tile, but
 * `GET …/connections` only answers for providers the manifest declares — so on
 * a tenant that declares none, every tile had `state === undefined`, the
 * `attested` guard never fired (there was no row to read it from), and all
 * eleven fell through to a Connect that 400'd.
 */
export function connectRoute(
  provider: ConnectionProvider,
  state: { credentialSource?: string } | undefined,
  reach: ComposioReach | null,
): ConnectRoute {
  if (state?.credentialSource === "static") return { kind: "native" };
  if (composioCanAuthorize(reach, provider.toolkit)) {
    return { kind: "composio", toolkit: provider.toolkit };
  }
  if (state?.credentialSource === "attested") return { kind: "managed" };
  return { kind: "unavailable" };
}
