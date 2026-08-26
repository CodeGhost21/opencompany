// The tool-grant API (issue #1796): the console reads and writes the company's
// effective `[tools].allow` through the host's `.../tools/grants` routes.
//
// Connecting an integration and granting its tool namespace are two separate
// steps, and until this existed only the first one had a write path. Every
// connect surface could store a credential and then had to tell the operator
// that the grant "cannot be fixed from this page" — so five integrations shipped
// looking wired and reached nobody.
//
// Like `api/policy.ts`, the console never writes the manifest: a rebuild
// re-persists that from the seed, and for `[tools]` that is a security property.
// A grant here is a durable, attributed *override* the host folds ahead of the
// manifest, and version control still wins — a `[tools]` edit in `company.toml`
// clears every console grant.
//
// Standalone functions over the shared client (mirrors `api/policy.ts`), so no
// change to `OpenCompanyClient` or the shared `api/types.ts` is needed.

import type { OpenCompanyClient } from "./client";

/** The company's effective tool grants, and what the console may add. */
export interface ToolGrants {
  /** The grant list in force — the manifest's, plus what was granted here. */
  allow: string[];
  /** The manifest's own list, so the console can name what a reset restores. */
  manifestAllow: string[];
  /** The namespaces granted from the console, in the order they were granted. */
  added: string[];
  /**
   * Every namespace the host will accept.
   *
   * Read rather than hard-coded: the closed list lives in the host (it is a
   * security boundary, not a UI preference), so a build that widens or narrows
   * it needs no console change — and a console built against an older host
   * offers exactly what that host can honour.
   */
  grantable: string[];
  /** Who granted, if anything was granted here. */
  setBy?: string;
  /** When (epoch millis), if anything was granted here. */
  setAtMillis?: number;
  /**
   * When a grant starts working, in the host's words.
   *
   * Rendered rather than paraphrased: a grant lands on the company's NEXT turn,
   * so a turn already running finishes with the belt it started with. "Make this
   * work now" is what an operator comes here to do, so that gap is worth stating
   * rather than leaving them to discover.
   */
  takesEffect: string;
}

/** Reads the company's effective tool grants. */
export function getToolGrants(
  client: OpenCompanyClient,
  company: string | null,
): Promise<ToolGrants> {
  return client.get<ToolGrants>(`${client.scopeFor(company)}/tools/grants`);
}

/**
 * Grants one namespace. Admin-only, attributed, in force on the next turn.
 *
 * One namespace per call rather than a list, because that is the shape of the
 * action: an operator on the Chargebee panel is granting `chargebee`. A
 * list-shaped body would make "add this one" express itself as "replace the set
 * with these", which is a different and revocable operation.
 */
export function grantTool(
  client: OpenCompanyClient,
  company: string | null,
  namespace: string,
): Promise<ToolGrants> {
  return client.put<ToolGrants>(`${client.scopeFor(company)}/tools/grants`, {
    namespace,
  });
}

/**
 * Withdraws one console grant, or all of them when `namespace` is omitted.
 *
 * A manifest grant is untouchable here by construction — this removes only what
 * the console added.
 */
export function revokeTool(
  client: OpenCompanyClient,
  company: string | null,
  namespace?: string,
): Promise<ToolGrants> {
  const base = `${client.scopeFor(company)}/tools/grants`;
  return client.del<ToolGrants>(
    namespace ? `${base}?namespace=${encodeURIComponent(namespace)}` : base,
  );
}
