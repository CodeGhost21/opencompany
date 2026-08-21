// The custom-domain API: the domain a company sends and receives on, and the
// DNS records that have to exist for it to work.
//
// **The records are the host's answer. The console derives nothing.**
//
// This module exists because the console used to fabricate them: a
// `dnsRecords(domain)` helper in `src/lib/domain.ts` hashed the domain into a
// verification token and pasted it into a hardcoded `opencompany.host` target,
// client-side. Every value it produced was a guess. An operator who copied
// those five rows into their registrar had added five records the host had
// never heard of, and the card's "Pending" badge — which nothing could ever
// clear, because nothing was ever checked — told them to keep waiting.
//
// So: `records` comes off `DomainStatus`, `verified` comes off `DomainStatus`,
// and `checks` comes off `DomainStatus`. If a row is on screen, the host put it
// there. Do not reintroduce a local generator, not even as a fallback for an
// empty list — a fallback is the same lie with a narrower blast radius.
//
// Standalone functions over the shared client, mirroring `api/hosting.ts`, so
// `OpenCompanyClient` needs no new methods.

import type { OpenCompanyClient } from "./client";

/** One DNS record the operator must create at their registrar. */
export interface DnsRecord {
  type: "CNAME" | "TXT";
  name: string;
  value: string;
  /** A string, not a number — the host sends what a registrar form expects. */
  ttl: string;
}

/**
 * The result of looking one record up, from the most recent verify pass.
 *
 * Matched back to its record by `(name, type)` and never by position: the host
 * is free to return checks in a different order, or to check a subset, and an
 * index-matched join would silently move a tick onto the wrong row.
 */
export interface RecordCheck {
  name: string;
  type: "CNAME" | "TXT";
  /** Whether the expected value was found in DNS. */
  found: boolean;
}

/**
 * A company's custom-domain configuration as the host holds it.
 *
 * `domain: ""` is the unconfigured sentinel — the read can also answer `null`
 * before anything has ever been set, and removal is `PUT { domain: "" }`. There
 * is no DELETE.
 */
export interface DomainStatus {
  domain: string;
  verified: boolean;
  /** Authoritative. See the module header. */
  records: DnsRecord[];
  /**
   * Absent until a verify pass has run — which is a different thing from "ran
   * and found nothing", and the card words the two differently.
   */
  checks?: RecordCheck[];
}

/** Reads the company's domain configuration, or null if none was ever set. */
export async function getDomain(
  client: OpenCompanyClient,
  company: string | null,
): Promise<DomainStatus | null> {
  return client.get<DomainStatus | null>(`${client.scopeFor(company)}/domain`);
}

/**
 * Sets the domain and returns the records the host wants created for it.
 *
 * Admin-only on the host. The response — not the request — is what the card
 * renders next.
 */
export async function saveDomain(
  client: OpenCompanyClient,
  company: string | null,
  domain: string,
): Promise<DomainStatus> {
  return client.put<DomainStatus>(`${client.scopeFor(company)}/domain`, { domain });
}

/**
 * Removes the configured domain.
 *
 * An empty-string PUT, because that is the host's contract: the domain field is
 * the whole resource, so clearing it is a write of the empty value rather than
 * a DELETE of the route.
 */
export async function clearDomain(
  client: OpenCompanyClient,
  company: string | null,
): Promise<DomainStatus> {
  return saveDomain(client, company, "");
}

/**
 * Asks the host to look the records up in DNS now.
 *
 * Rejects with `404 not_wired` on a host built without the `dns` feature — a
 * build fact, not an outage, and the card says so rather than showing an error —
 * and with a `400` when no domain is configured, which is why callers must test
 * for the `not_wired` code rather than for the status.
 */
export async function verifyDomain(
  client: OpenCompanyClient,
  company: string | null,
): Promise<DomainStatus> {
  return client.post<DomainStatus>(`${client.scopeFor(company)}/domain/verify`);
}
