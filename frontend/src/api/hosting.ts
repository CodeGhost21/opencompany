// The hosting configuration API: where a company's hosting provider API key is
// stored so its agents can deploy a workspace to the public internet.
//
// The key is write-only: it is sent on save into the host's secret store and is
// never returned. The read shape carries booleans, the provider slug and the
// team scope only, so there is no field on this type that could leak a key into
// a rendered page.
//
// Standalone functions over the shared client, mirroring `api/billing.ts`, so
// `OpenCompanyClient` needs no new methods.

import type { OpenCompanyClient } from "./client";

/**
 * The non-secret view of a company's hosting configuration.
 *
 * Three separate flags rather than one `connected`, because they fail
 * differently and a single boolean sends an operator to the wrong place for two
 * of them — see `HostingView` for how each is worded.
 */
export interface HostingStatus {
  /** Whether an API key is stored. Never the key. */
  apiKeyConfigured: boolean;
  /** The provider the key belongs to, e.g. `vercel`. Not secret. */
  provider: string;
  /** The team/organization scope, or null for a personal account. */
  team: string | null;
  /** Whether the company's manifest explicitly grants `hosting`. */
  granted: boolean;
  /** Whether the running host has the hosting tools compiled in. */
  inBuild: boolean;
  /** The providers a key can be stored for in this build. */
  supportedProviders: string[];
}

/** The write-only save body. Omitted fields keep their stored value. */
export interface HostingConfig {
  /** Write-only. Omit to leave the stored key unchanged. */
  apiKey?: string;
  /** The provider slug. Omit to leave it unchanged. */
  provider?: string;
  /** The team/organization scope. Omit to leave it unchanged. */
  team?: string;
}

/** Reads the company's hosting configuration status. */
export async function getHosting(
  client: OpenCompanyClient,
  company: string | null,
): Promise<HostingStatus> {
  return client.get<HostingStatus>(`${client.scopeFor(company)}/hosting`);
}

/**
 * Saves whatever is supplied, and returns the resulting status.
 *
 * A patch, not a replace: the host applies only the fields present and
 * non-empty, so correcting the team never means re-typing the API key — which an
 * operator cannot do anyway, since it is never shown back to them.
 */
export async function saveHosting(
  client: OpenCompanyClient,
  company: string | null,
  config: HostingConfig,
): Promise<HostingStatus> {
  return client.put<HostingStatus>(`${client.scopeFor(company)}/hosting`, config);
}

/** Clears every stored hosting credential. */
export async function clearHosting(
  client: OpenCompanyClient,
  company: string | null,
): Promise<HostingStatus> {
  return client.del<HostingStatus>(`${client.scopeFor(company)}/hosting/key`);
}
