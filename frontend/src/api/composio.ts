// The live Composio API (issue #110, epic #26 Cell D): the console reads the
// company's Composio status and — when a company brings its own account — writes
// a per-company OAuth bearer token through the host's `.../composio` routes
// (REST, camelCase over the wire).
//
// On the hosted platform nothing is pasted: the instance authenticates with a
// platform-minted, audience-bound identity and Composio calls present that, which
// the read shape reports as `credentialSource: "attested"`. The write route is the
// per-company BYO override, and the only option when this repo is run standalone
// — which is UNSUPPORTED. Either way the token is WRITE-ONLY: sent on
// `PUT .../composio/token`, stored in the host's secret store, never returned. The
// read shape carries only `credentialSource` plus non-secret routing (backend URL,
// toolkit allowlist). Standalone functions over the shared client (mirrors
// `api/inference.ts`), so no change to `OpenCompanyClient` is needed.

import type { OpenCompanyClient } from "./client";

/**
 * Where this company's Composio credential comes from.
 *
 * - `attested` — the instance's own platform identity; nothing is stored here.
 * - `static` — a token this company pasted, or a static instance key.
 * - `none` — no credential can be obtained, so agents get no Composio tools.
 */
export type ComposioCredentialSource = "attested" | "static" | "none";

/**
 * One provider in the catalog the host offers, with the backend's own display
 * metadata (issue #600).
 *
 * Every field but `slug` is best-effort and may be empty — a manifest
 * allowlist, a degraded fallback, and a backend predating Composio's dynamic
 * catalog all yield slug-only entries. The console fills those in from
 * `@/lib/composio-catalog`, which is why that local typography table survives
 * rather than being deleted in favour of the backend's names.
 */
export interface ComposioToolkitEntry {
  /** Toolkit slug, e.g. `googlecalendar`. The key every host call is made with. */
  slug: string;
  /** Human-readable name, e.g. `Google Calendar`. Empty when unpublished. */
  name: string;
  /** One-line description. Empty when unpublished. Searched alongside the name. */
  description: string;
  /** Composio-hosted logo URL, or `null` when unpublished. */
  logo: string | null;
  /**
   * Composio's own free-form category names, e.g. `["productivity", "email"]`.
   *
   * Forwarded verbatim by the host and bucketed here by substring — which is
   * what means a Composio integration added tomorrow lands in the right group
   * with no code change on either side of the wire.
   */
  categories: string[];
}

/** The company's Composio status. Never carries the token. */
export interface ComposioStatus {
  /** Whether the `composio` feature is compiled into this build at all. */
  inBuild: boolean;
  /** Whether the company explicitly grants `composio` (a `*` wildcard does not count). */
  granted: boolean;
  /** Which credential this company's Composio calls present — never the credential itself. */
  credentialSource: ComposioCredentialSource;
  /** The effective Composio backend URL (non-secret). */
  backendUrl: string;
  /** The manifest toolkit allowlist verbatim (empty = defer to the backend allowlist). */
  toolkits: string[];
  /**
   * Whether this company is in **open mode** — an empty manifest allowlist,
   * which means the backend's own allowlist governs and every toolkit it
   * permits is reachable (issue #397).
   *
   * The host tells us rather than letting us infer it: an empty `toolkits`
   * means *allow everything*, which is the opposite of what an empty list
   * reads as. Gating provider rows on `toolkits.length > 0` was exactly that
   * misreading, and it left 19 of 20 shipped templates with nothing to click.
   */
  openMode: boolean;
  /**
   * The toolkits to render as provider rows — the manifest list when non-empty,
   * else the backend's live catalog. In open mode this is still not a hard
   * limit: any slug the backend permits can be authorized, which is what the
   * "other provider" field is for.
   */
  effectiveToolkits: string[];
  /**
   * The same providers as {@link effectiveToolkits}, in the same order, each
   * carrying whatever display metadata the backend published for it (issue
   * #600).
   *
   * This is what makes the panel browsable. Before it, the host reduced every
   * catalog entry to a bare slug one layer before the console, so 123 providers
   * could only be a flat list: there was nothing to group by, nothing to brand
   * with, and nothing to search but the slug.
   *
   * Additive rather than a replacement — {@link effectiveToolkits} is still the
   * slug contract, and it is still all an authorize call needs.
   */
  effectiveCatalog: ComposioToolkitEntry[];
  /**
   * Where {@link effectiveToolkits} came from (issue #397).
   *
   * - `manifest` — the company's own allowlist, offered verbatim. Not a
   *   degradation: the company chose this list.
   * - `backend` — Composio's live catalog. Authoritative.
   * - `fallback` — the catalog could not be fetched, so this is a built-in
   *   starter list that may be incomplete. Always paired with
   *   {@link catalogNotice}.
   *
   * Rendering a `fallback` the same way as a `backend` list would waste the
   * distinction: eight providers would look like the whole set, which is the
   * shape of the bug this issue was reopened for.
   */
  catalogSource: "manifest" | "backend" | "fallback";
  /** Why the list is a fallback, for the operator. `null` unless degraded. */
  catalogNotice: string | null;
}

/** A mutating response: the resulting status plus a plain-language note. */
export interface ComposioMutation {
  status: ComposioStatus;
  note: string;
}

/** The `POST …/composio/authorize` response: the hosted connect URL to open. */
export interface ComposioAuthorize {
  /** Composio-hosted OAuth URL the operator opens in a new browser tab. */
  connectUrl: string;
}

/** One toolkit's connected state, as returned by `GET …/composio/connections`. */
export interface ComposioConnection {
  /** Toolkit slug, e.g. `gmail`. */
  toolkit: string;
  /** Whether the company has at least one active connection for this toolkit. */
  connected: boolean;
}

/** The company's Composio status. */
export function getComposioStatus(
  client: OpenCompanyClient,
  company: string | null,
): Promise<ComposioStatus> {
  return client.get<ComposioStatus>(`${client.scopeFor(company)}/composio`);
}

/**
 * Set / rotate / clear this company's own Composio token. A non-empty value
 * rotates it; an empty string clears it, reverting to the instance's identity
 * where there is one.
 */
export function setComposioToken(
  client: OpenCompanyClient,
  company: string | null,
  token: string,
): Promise<ComposioMutation> {
  return client.put<ComposioMutation>(`${client.scopeFor(company)}/composio/token`, { token });
}

/**
 * Begin a per-provider OAuth handoff for `toolkit` (e.g. `gmail`). Returns the
 * Composio-hosted connect URL the console opens in a new tab. Composio runs the
 * OAuth itself — there is no local callback — so the console then polls
 * {@link listComposioConnections} until the toolkit reports connected. 409 when
 * the feature is not in the build, or no per-tenant token is configured yet.
 */
export function startComposioAuthorize(
  client: OpenCompanyClient,
  company: string | null,
  toolkit: string,
): Promise<ComposioAuthorize> {
  return client.post<ComposioAuthorize>(`${client.scopeFor(company)}/composio/authorize`, {
    toolkit,
  });
}

/**
 * The company's per-toolkit connected state — one row per toolkit that has at
 * least one connection. The console cross-references this against the granted
 * `toolkits` to render each provider row's connected / sign-in state. 409 when
 * the feature is not in the build, or no per-tenant token is configured yet.
 */
export function listComposioConnections(
  client: OpenCompanyClient,
  company: string | null,
): Promise<ComposioConnection[]> {
  return client.get<ComposioConnection[]>(`${client.scopeFor(company)}/composio/connections`);
}
