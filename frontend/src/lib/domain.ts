// Custom domain + SMTP setup for a company.
//
// The NON-SECRET half of the form is remembered per company in localStorage so
// a half-filled card survives a reload. The SMTP **password** never is, and
// this module is the single place that guarantees it (issue #1460).
//
// It used to be: the card held the whole `MailSettings` in one `useState` and
// an effect wrote all of it back on every change, so the password landed in
// browser storage keystroke by keystroke — readable by any script on the
// origin, and still there after sign-out. A sending credential does not belong
// in a store with no expiry and no encryption; every other credential on this
// surface (Billing, Hosting, MCP, the company credential) goes write-only into
// the host's secret store instead.
//
// So the persisted shape is `StoredMailSettings`, which structurally has no
// `password` field. `saveMailSettings` cannot write one because it does not
// receive one, and `loadMailSettings` always hands back an empty password. The
// password lives in React state for the life of the page and nowhere else.
//
// `purgeStoredSmtpPasswords` cleans up after the old behaviour on the way in —
// see its docstring.

import { type LocalScope, scopedKeyAdoptingLegacy } from "@/connections/types";

export interface DomainConfig {
  domain: string;
  verified: boolean;
}

export type SmtpSecurity = "none" | "starttls" | "ssl";

export interface SmtpConfig {
  host: string;
  port: string;
  security: SmtpSecurity;
  username: string;
  password: string;
  fromName: string;
  fromEmail: string;
}

export interface MailSettings {
  domain: DomainConfig;
  smtp: SmtpConfig;
}

/**
 * The subset of {@link MailSettings} that is allowed into localStorage.
 *
 * `password` is omitted **structurally**, not by convention: `saveMailSettings`
 * takes this type, so a future caller that tries to persist a credential does
 * not compile. That is the point — a comment saying "don't store the password"
 * rots, a type does not.
 */
export interface StoredMailSettings {
  domain: DomainConfig;
  smtp: Omit<SmtpConfig, "password"> & {
    /**
     * Never present. Typed `never` rather than simply omitted because omission
     * alone would not stop anything: TypeScript is structural, so a full
     * {@link MailSettings} — which has every field this shape asks for, plus a
     * password — is assignable to a plain `Omit<…>` by width subtyping, and the
     * old leaking call would still compile. `password?: never` rejects it,
     * because `string` is not assignable to `never`.
     */
    password?: never;
  };
}

export function emptyMailSettings(): MailSettings {
  return {
    domain: { domain: "", verified: false },
    smtp: { host: "", port: "587", security: "starttls", username: "", password: "", fromName: "", fromEmail: "" },
  };
}

/** The platform host records point at. In production this comes from the host. */
const PLATFORM_TARGET = "mail.opencompany.host";

export interface DnsRecord {
  type: "CNAME" | "TXT";
  name: string;
  value: string;
  ttl: string;
}

/** A short, stable verification token derived from the domain. */
function verifyToken(domain: string): string {
  let hash = 0;
  for (let i = 0; i < domain.length; i++) hash = (hash * 31 + domain.charCodeAt(i)) | 0;
  return Math.abs(hash).toString(16).padStart(8, "0");
}

/** The DNS records a user must add to point a custom domain at the platform
 *  and let it send email (verification + CNAME + DKIM + SPF). */
export function dnsRecords(domain: string): DnsRecord[] {
  const d = domain.trim().replace(/\.$/, "");
  if (!d) return [];
  return [
    { type: "TXT", name: `_opencompany.${d}`, value: `oc-verify=${verifyToken(d)}`, ttl: "3600" },
    { type: "CNAME", name: d, value: PLATFORM_TARGET, ttl: "3600" },
    { type: "CNAME", name: `oc1._domainkey.${d}`, value: `oc1.dkim.opencompany.host`, ttl: "3600" },
    { type: "CNAME", name: `oc2._domainkey.${d}`, value: `oc2.dkim.opencompany.host`, ttl: "3600" },
    { type: "TXT", name: d, value: "v=spf1 include:spf.opencompany.host ~all", ttl: "3600" },
  ];
}

export function isValidDomain(domain: string): boolean {
  return /^(?!-)[a-z0-9-]+(\.[a-z0-9-]+)+$/i.test(domain.trim());
}

const KEY = (scope: LocalScope) =>
  scopedKeyAdoptingLegacy("oc-mail", scope, `oc-mail:${scope.company ?? "single"}`);

/**
 * Reads the remembered non-secret half back, with an empty password.
 *
 * The empty password is unconditional and deliberate: even if a stored blob
 * still carries one — a key written by an older build, in a tab that has not
 * reloaded since the purge ran — it is not read back into the form. Nothing
 * downstream of this function can resurrect a credential from storage.
 */
export function loadMailSettings(scope: LocalScope): MailSettings {
  try {
    const raw = localStorage.getItem(KEY(scope));
    if (raw) {
      const stored = JSON.parse(raw) as Partial<MailSettings>;
      const merged = { ...emptyMailSettings(), ...stored };
      return { ...merged, smtp: { ...merged.smtp, password: "" } };
    }
  } catch {
    /* fall through */
  }
  return emptyMailSettings();
}

/**
 * Strips the password off a full settings object, yielding the storable shape.
 *
 * Destructuring rather than deleting a key, so the result never transiently
 * holds the credential and `JSON.stringify` has nothing to find.
 */
export function withoutSmtpPassword(settings: MailSettings): StoredMailSettings {
  const { password: _password, ...smtp } = settings.smtp;
  return { domain: settings.domain, smtp };
}

/**
 * Persists the non-secret half of the card.
 *
 * Takes {@link StoredMailSettings}, so there is no password to write. Callers
 * holding a full {@link MailSettings} pass it through
 * {@link withoutSmtpPassword} first.
 */
export function saveMailSettings(scope: LocalScope, settings: StoredMailSettings): void {
  try {
    localStorage.setItem(KEY(scope), JSON.stringify(settings));
  } catch {
    /* storage unavailable */
  }
}

/** Every localStorage key this module has ever written a password under. */
const MAIL_KEY_PREFIX = "oc-mail";

/**
 * Removes SMTP passwords left in localStorage by the pre-#1460 console.
 *
 * Stopping new writes only half-solves it: an operator who typed a password
 * before upgrading still has it sitting in their browser, and it stays there
 * until something deletes it. This runs once at boot (see `main.tsx`) and
 * sweeps **every** `oc-mail*` key — scoped and legacy, every connection and
 * company, not just the scope the current page happens to be looking at,
 * because the operator is not required to visit Settings for the credential to
 * need to be gone.
 *
 * It rewrites rather than deletes: host, port, security, username and the from
 * fields are not secret and are work the operator did, so they survive. Only
 * the password is dropped. A key holding unparseable JSON is removed outright —
 * it cannot be shown to be password-free, and this function's contract is that
 * afterwards no password remains.
 *
 * Returns the number of keys it rewrote or removed, which is what the
 * regression test asserts on.
 */
export function purgeStoredSmtpPasswords(): number {
  let cleaned = 0;
  try {
    const store = window.localStorage;
    const keys: string[] = [];
    for (let i = 0; i < store.length; i++) {
      const key = store.key(i);
      if (key !== null && key.startsWith(MAIL_KEY_PREFIX)) keys.push(key);
    }
    for (const key of keys) {
      const raw = store.getItem(key);
      if (raw === null) continue;
      if (!raw.includes('"password"')) continue;
      let parsed: unknown;
      try {
        parsed = JSON.parse(raw);
      } catch {
        // Unreadable, but it contains the word. Cannot be proven clean, so it
        // goes — a stale draft is a far smaller loss than a retained secret.
        store.removeItem(key);
        cleaned++;
        continue;
      }
      const settings = { ...emptyMailSettings(), ...(parsed as Partial<MailSettings>) };
      store.setItem(key, JSON.stringify(withoutSmtpPassword(settings)));
      cleaned++;
    }
  } catch {
    /* storage unavailable — nothing was ever stored to clean */
  }
  return cleaned;
}
