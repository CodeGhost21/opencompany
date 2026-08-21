// The outbound-mail API: the SMTP server a company sends through.
//
// The password is write-only. It goes into the host's secret store on save and
// there is no field on {@link SmtpStatus} that could carry it back — not an
// empty string, not a mask. It is also never written to browser storage; before
// issue #1460 the card persisted the whole form to `localStorage` on every
// keystroke, which is the bug this surface exists to have fixed.
//
// # Casing: these wire shapes are snake_case, deliberately
//
// `from_name` and `from_email` are NOT typos and NOT an oversight. The host's
// `SmtpStatus` and `SmtpConfig` structs carry no `#[serde(rename_all =
// "camelCase")]`, so serde emits Rust field names verbatim. `api/hosting.ts`
// next door is camelCase (`apiKeyConfigured`) because ITS structs do carry the
// attribute. The two files disagree because the host disagrees with itself.
//
// "Fixing" these to `fromName`/`fromEmail` compiles, type-checks, and silently
// stops working: the host deserializes a body with a missing `from_email` and
// the read renders an undefined From address. If you change them, change the
// Rust struct in the same commit.
//
// Standalone functions over the shared client, mirroring `api/hosting.ts`.

import type { OpenCompanyClient } from "./client";

/** How the connection to the SMTP server is secured. */
export type SmtpSecurity = "none" | "starttls" | "ssl";

/**
 * The non-secret view of a company's SMTP configuration.
 *
 * Optional fields are `?:` rather than `| null`: the host uses
 * `skip_serializing_if`, so an unset field is **absent** from the JSON, not
 * present-and-null. There is no password field, by construction.
 */
export interface SmtpStatus {
  /** Whether a complete configuration — including a password — is stored. */
  configured: boolean;
  host?: string;
  port?: number;
  security?: SmtpSecurity;
  username?: string;
  /** snake_case on the wire. See the module header before renaming. */
  from_name?: string;
  /** snake_case on the wire. See the module header before renaming. */
  from_email?: string;
}

/**
 * The save body.
 *
 * `port` is a number because the host takes a `u16`; a string, or an
 * out-of-range value, fails deserialization with a message no operator can act
 * on, so callers validate before sending.
 */
export interface SmtpConfig {
  host: string;
  port: number;
  security: SmtpSecurity;
  username: string;
  /** snake_case on the wire. See the module header before renaming. */
  from_email: string;
  /** snake_case on the wire. See the module header before renaming. */
  from_name?: string;
  /** Write-only. **Omit** to keep the password already stored. */
  password?: string;
}

/** The host's own verdict on a test send, in its own words. */
export interface SmtpTestResult {
  ok: boolean;
  /**
   * Rendered verbatim on both branches. The host knows whether the server
   * refused the credentials, timed out, or rejected the From address; a generic
   * "Couldn't send" thrown over the top of that is strictly less useful.
   */
  message: string;
}

/** Reads the company's SMTP configuration status. Never the password. */
export async function getSmtp(
  client: OpenCompanyClient,
  company: string | null,
): Promise<SmtpStatus> {
  return client.get<SmtpStatus>(`${client.scopeFor(company)}/smtp`);
}

/**
 * Saves the configuration and returns the resulting status.
 *
 * Admin-only on the host. Omitting `password` leaves the stored one in place,
 * so correcting the port never means re-typing a credential the operator cannot
 * see.
 */
export async function saveSmtp(
  client: OpenCompanyClient,
  company: string | null,
  config: SmtpConfig,
): Promise<SmtpStatus> {
  return client.put<SmtpStatus>(`${client.scopeFor(company)}/smtp`, config);
}

/**
 * Asks the host to send a test message through the stored configuration.
 *
 * Rejects with `404 not_wired` on a host built without the `smtp` feature — the
 * credentials are still stored and a build that has the feature will use them.
 */
export async function testSmtp(
  client: OpenCompanyClient,
  company: string | null,
  to?: string,
): Promise<SmtpTestResult> {
  return client.post<SmtpTestResult>(
    `${client.scopeFor(company)}/smtp/test`,
    to ? { to } : {},
  );
}
