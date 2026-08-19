// Metadata for the providers this console knows natively, and the rule that
// decides how a tile's button behaves.
//
// ## Not the page's provider list (issue #582)
//
// It used to be one, rendered as a categorised grid of eleven tiles *alongside*
// a second grid built from the backend's live Composio catalog — two lists, two
// status routes, and a page that could say a provider was both connected and
// not connected. The page now renders one grid (`lib/provider-grid.ts`), whose
// membership is the backend catalog and whose status is `GET …/connections`.
//
// What `CONNECTION_PROVIDERS` still owns is what only the console can know: the
// Composio slug each id maps to and the brand colours.
//
// ## Two routes, one tile (issue #599)
//
// A tile can be connected two ways, and which one is live is a property of the
// *host*, not of the tile:
//
//  - **Composio** — the hosted path. Composio runs the OAuth itself and the
//    resulting connection is a tool belt the agents actually receive.
//  - **Native** — a legacy credential written by the retired host OAuth flow.
//    It is reported as `credentialSource: "static"`, but no agent can use it.
//
// Until #599 every tile hard-routed to native. On a hosted tenant no
// `OPENCOMPANY_OAUTH_*` variable is injected, so all eleven Connect buttons
// 400'd with "provider is not enabled on this host" — a grid of buttons that
// could never succeed. [`connectRoute`] is now the single place that decides,
// and it can answer "neither", so a button that cannot work is never rendered.
//
// ## …and now one route offered, not two (issue #822)
//
// The native arm is gone from that decision. It was the half of #599 that
// *could* succeed and still bought the operator nothing: `oauth/{provider}` is
// written by the callback and read by no agent tool — zero occurrences under
// `src/harness/` (#396) — so a self-hoster could register a provider
// application, complete a real handshake, watch the tile go green, and have
// given their agents no ability whatsoever. A Connect that 400s is a bad
// button; a Connect that succeeds and confers nothing is a false promise, and
// the page should not be making it while #396 is unsettled.
//
// So `connectRoute` answers `composio`, `managed` or `unavailable`, and a host
// whose only route is the native hatch gets the same honest "not available
// here" a host with no route at all gets. Two things deliberately survive:
//
//  - **A credential already stored stays visible.** Removing the offer must not
//    hide a token the company has, so a natively connected provider keeps its
//    tile, its `via: ["native"]` and its Disconnect (`lib/provider-grid.ts`).
//    Retracting the invitation is this issue; releasing what was accepted under
//    it is the operator's call.
//  - **The retirement bridge.** `POST …/connections/{provider}/start` and the
//    callback return an explanation until 2026-09-30, rather than silently
//    404ing a cached pre-#828 bundle. They never write another credential (#838).

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
   * It is also what `disconnectConnection(id)` is called with. The console
   * never starts native OAuth: #838 retains only an explanatory compatibility
   * response for stale bundles, so this id can describe a connection already
   * held but never one newly offered through the native catalog.
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
  /**
   * The provider's own brand colour, for its monogram tile.
   *
   * A raw hex on purpose, and one of only two places in the console where
   * that is correct (the other is `--brand-discord`). These identify
   * *someone else* — Slack's aubergine is Slack's, and a themed
   * approximation of it would be wrong in both themes. They are data about
   * a third party, not a design decision this system gets to make, so they
   * do not belong in the token layer.
   *
   * Anything drawn on top of one must not assume a light or dark ground:
   * these span `#0F0F0F` to `#EA4335`.
   */
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
 * The tile's own id is tried first — a manifest row is keyed that way — then
 * normalized keys, so a reconciled Composio row keyed `googlecalendar` still
 * finds the `google-calendar` tile.
 *
 * **A connected row wins over a disconnected one.** The host can emit *two* rows
 * for one provider when its id and its Composio slug do not normalize to the
 * same key: `toolkit_slug("x")` is `x`, not `twitter`, so a manifest declaring
 * `provider = "x"` produces a disconnected `x` row while Composio's connected
 * `twitter` state arrives as a separate appended row. Taking the first match
 * would report that tile disconnected while the account is in fact connected —
 * the same "two surfaces disagreeing" failure #316 set out to end. Connected
 * beats not-connected for the same reason the host unions `native ||
 * composio_connected` within a single row; this extends that union across the
 * alias it cannot currently see.
 *
 * Direct-id precedence still decides when nothing is connected, so a manifest
 * row remains the authority on a provider the host answered for.
 */
export function connectionStateFor<T extends { provider: string; connected?: boolean }>(
  provider: ConnectionProvider,
  states: Record<string, T>,
): T | undefined {
  const wanted = new Set([toolkitSlug(provider.id), toolkitSlug(provider.toolkit)]);
  const direct = states[provider.id];
  const matches: T[] = [];
  if (direct) matches.push(direct);
  for (const state of Object.values(states)) {
    if (state !== direct && wanted.has(toolkitSlug(state.provider))) matches.push(state);
  }
  return matches.find((state) => state.connected) ?? matches[0];
}

/** What this host offers for Composio, as far as routing a tile is concerned. */
export interface ComposioReach {
  /** Whether the `composio` feature is compiled into this build. */
  inBuild: boolean;
  /**
   * Whether the company explicitly grants the `composio` tool namespace.
   *
   * **Not part of the routing decision** (issue #582) — carried so the grid can
   * caveat a connection the agents cannot use yet. See
   * {@link composioCanAuthorize} for why gating on it was the page's
   * contradiction rather than a safeguard.
   */
  granted: boolean;
  /**
   * Whether a credential of **any** tier resolves; `none` means there is
   * nothing to authorize against.
   *
   * Deliberately a boolean rather than the tier itself. Which credential the
   * host reaches Composio with is the host's business, and the set of tiers
   * grows — #586 adds `company` (the company's own TinyHumans key) alongside
   * `attested` and `static`. Routing on "is there one at all" means a new tier
   * is additive here by construction: a tenant that can only reach Composio
   * through its company key gets the same working Connect as an attested pod,
   * with no edit to this rule.
   */
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
 *
 * ## The `composio` grant is deliberately not consulted (issue #582)
 *
 * It used to be, and that was half of the page's self-contradiction. The grant
 * governs whether **agents receive Composio tools** — it is not a property of
 * whether a sign-in can complete. `POST …/composio/authorize` never checked it
 * (`resolve_tenant` in `src/server/ops/composio.rs` reads the credential and the
 * toolkit allowlist, and nothing else), so the sign-in this predicate refused to
 * offer worked perfectly well from the provider list that ignored the grant.
 * One surface said "connect me", the other said "no route" — for the same
 * account, on the same screen.
 *
 * Reinstating the gate on the other surface instead would have been the wrong
 * repair: it would take away a sign-in operators use today and leave them no
 * way to connect an account before granting the namespace. So the grant stops
 * being a route decision and becomes what it always described — a caveat the
 * grid states plainly ("connected, but agents will not receive its tools until
 * `composio` is granted"), sourced from {@link ComposioReach.granted}.
 */
export function composioCanAuthorize(reach: ComposioReach | null, toolkit: string): boolean {
  if (!reach || !reach.inBuild || !reach.hasCredential) return false;
  if (reach.openMode) return true;
  const wanted = toolkitSlug(toolkit);
  return reach.effectiveToolkits.some((slug) => toolkitSlug(slug) === wanted);
}

/**
 * How a tile's Connect should behave.
 *
 * - `composio` — authorize `toolkit` through Composio's hosted OAuth.
 * - `managed` — the platform runs connections for this instance and there is no
 *   Composio route either; nothing to do here.
 * - `unavailable` — no route this console will offer can succeed, so the tile
 *   says so instead of offering a button.
 *
 * There is deliberately no `native` arm (issue #822). It existed, it worked,
 * and what it bought was a credential no agent reads (#396) — see the note at
 * the top of this file. A host whose only hatch is that one now lands on
 * `unavailable`, which is what the tile already renders without an action.
 */
export type ConnectRoute =
  | { kind: "composio"; toolkit: string }
  | { kind: "managed" }
  | { kind: "unavailable" };

/**
 * Decide the route for one tile — the single rule the grid renders *and* acts
 * on, so the button shown and the call made can never disagree.
 *
 * Precedence, and why:
 *
 * 1. **Composio, when it can authorize this toolkit.** The hosted path, the only
 *    one on a tenant — which is injected no `OPENCOMPANY_OAUTH_*` variable at
 *    all — and now the only one this console offers anywhere. It is also the
 *    only one that makes a connection a *capability*: `src/harness/composio.rs`
 *    turns it into tools the agents receive.
 * 2. **`attested` → managed.** A platform-projected identity, so connections are
 *    the platform's to run and no local Connect could work.
 * 3. **Otherwise unavailable.** Where an unknown provider lands on a host with
 *    no Composio — `credentialSource` is `undefined` because the manifest never
 *    declared it, and a Connect would 400 — and, since #822, where a provider
 *    lands whose *only* route is the native hatch.
 *
 * Step 3 is the bug #599 reports. The grid renders every catalog tile, but
 * `GET …/connections` only answers for providers the manifest declares — so on
 * a tenant that declares none, every tile had `state === undefined`, the
 * `attested` guard never fired (there was no row to read it from), and all
 * eleven fell through to a Connect that 400'd.
 *
 * ## `static` is no longer read (issue #822)
 *
 * It used to take precedence over everything: a host that registered its own
 * provider application, or a company that stored its own token, got the native
 * hatch and Composio was not allowed to override the operator's deliberate
 * configuration. What that reasoning missed is that the hatch confers nothing —
 * the stored `oauth/{provider}` secret is read by no agent tool (#396) — so the
 * arm preserved an operator's configuration by handing them a green tile and no
 * capability. With it gone, such a host takes the Composio route when it has
 * one and reports `unavailable` when it does not.
 *
 * ## One tier name appears here, on purpose
 *
 * `attested` is named because it answers a question about the *local* host: no
 * local handshake can ever complete on it. Every other tier — including
 * `company` from #586 — is a statement about which credential the host presents
 * to Composio, which this function reads through
 * {@link ComposioReach.hasCredential} rather than by name. So the combination
 * "the company credential is set but the provider is not in `states`" needs no
 * case of its own: `state` is `undefined`, `hasCredential` is true, and the tile
 * gets a working Composio Connect — which is exactly the outcome #599 is about.
 */
export function connectRoute(
  // Only the toolkit slug is read, and the merged grid (issue #582) routes tiles
  // that have no `ConnectionProvider` entry at all — every provider the backend
  // catalog offers, not just the eleven with local metadata. Narrowing the
  // parameter to what the rule actually uses is what lets one rule serve both.
  provider: Pick<ConnectionProvider, "toolkit">,
  state: { credentialSource?: string } | undefined,
  reach: ComposioReach | null,
): ConnectRoute {
  if (composioCanAuthorize(reach, provider.toolkit)) {
    return { kind: "composio", toolkit: provider.toolkit };
  }
  if (state?.credentialSource === "attested") return { kind: "managed" };
  return { kind: "unavailable" };
}
