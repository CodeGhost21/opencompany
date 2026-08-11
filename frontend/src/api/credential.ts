// The company's own TinyHumans credential (issue #586): the one key its admin
// sets on this tenant, and the identity every surface the platform brokers
// presents on the company's behalf.
//
// Set it once and Composio rides it — no separate Composio token, no per-tenant
// provider app to register. Rotate it and every brokered surface moves together,
// because they all resolve through one seam on the host rather than each keeping
// its own copy.
//
// WRITE-ONLY, like every other credential the console handles: the key goes out
// on `PUT .../credential`, lands in the host's secret store, and is never
// returned. The read shape carries only `configured` plus the non-secret tier
// name. Standalone functions over the shared client (mirrors `api/composio.ts`).
//
// NOT the same thing as the inference key in `api/inference.ts`. That one holds
// whatever the company's *declared provider* wants — an OpenRouter key, a raw
// BYOK token — so it is provider-scoped, not an identity, and handing it to the
// TinyHumans backend would present one vendor's credential to another.

import type { OpenCompanyClient } from "./client";

/**
 * Which identity this company's brokered calls present right now.
 *
 * - `company` — this company's own key. What setting one buys you.
 * - `attested` / `static` — no company key set, so calls fall back to the
 *   instance's platform identity.
 * - `none` — neither, so providers cannot be connected or used at all. The
 *   honest degraded state, and the one the picker must not paper over.
 */
export type CompanyCredentialSource = "company" | "attested" | "static" | "none";

/** The company's credential status. Never carries the key. */
export interface CompanyCredentialStatus {
  /**
   * Whether this company has its **own** key stored. `false` does not mean no
   * credential — read {@link source} for what calls actually present.
   */
  configured: boolean;
  /** Which identity a brokered call presents right now. */
  source: CompanyCredentialSource;
  /**
   * The consequence of setting this key, or the degraded state when nothing can
   * be presented. Rendered verbatim: the host words it, so the console cannot
   * drift from what the host actually does.
   */
  notice: string;
}

/** A mutating response: the resulting status plus a plain-language note. */
export interface CompanyCredentialMutation {
  status: CompanyCredentialStatus;
  note: string;
}

/** Whether this company has its own credential, and which identity it presents. */
export function getCompanyCredential(
  client: OpenCompanyClient,
  company: string | null,
): Promise<CompanyCredentialStatus> {
  return client.get<CompanyCredentialStatus>(`${client.scopeFor(company)}/credential`);
}

/**
 * Set / rotate / clear the company's TinyHumans credential. A non-empty value
 * sets or rotates it; an empty string clears it, falling back to the instance's
 * platform identity where there is one. Admin-only — a member gets a 403.
 */
export function setCompanyCredential(
  client: OpenCompanyClient,
  company: string | null,
  key: string,
): Promise<CompanyCredentialMutation> {
  return client.put<CompanyCredentialMutation>(`${client.scopeFor(company)}/credential`, { key });
}
