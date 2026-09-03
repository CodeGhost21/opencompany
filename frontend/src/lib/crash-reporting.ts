// The pure half of console crash reporting: the enable/disable decision, the
// secret scrubber, and the event sanitizer.
//
// Kept apart from `@/lib/sentry` — which is the only module that imports the
// SDK — for the reason `src/observability/config.rs` is kept apart from
// `src/observability/mod.rs` on the host: these are the parts that have to be
// provably right, and a module with no runtime import of `@sentry/react` can be
// exercised by the fast `vitest` unit suite (`test/unit/`, `environment: node`)
// rather than only by a browser.
//
// The contract this implements is `docs/spec/runtime/crash-reporting.md`.
// The scrubber is a deliberate port of `src/observability/redaction.rs` — same
// token-wise algorithm, same prefix and key vocabularies — so the doc can
// describe one behaviour rather than two that drift.

import type { ErrorEvent } from "@sentry/react";

/** What a redacted span is replaced with, on both surfaces. */
export const REDACTED = "[redacted]";

/** The Sentry surface tag every event from this app carries. */
export const SURFACE = "console";

/** The message body of the one-shot smoke event. See `VITE_SENTRY_SMOKE_TEST`. */
export const SMOKE_TEST_MESSAGE = "console-sentry-smoke-test";

/**
 * Prefixes that identify a credential on their own, whatever surrounds them.
 * Case-sensitive: `AKIA` is an AWS key id and `akia` is a word.
 *
 * Mirrors `SECRET_PREFIXES` in `src/observability/redaction.rs`.
 */
const SECRET_PREFIXES = [
  "sk-",
  "sk_",
  "rk_",
  "pk_",
  "ghp_",
  "gho_",
  "ghs_",
  "ghu_",
  "ghr_",
  "github_pat_",
  "glpat-",
  "xoxb-",
  "xoxp-",
  "xoxa-",
  "xoxs-",
  "xoxe-",
  "xapp-",
  "AKIA",
  "ASIA",
  "th_",
  "shpat_",
  "shpss_",
  "npm_",
  "dop_v1_",
  "SG.",
];

/** The shortest a prefixed token may be before it is treated as a credential. */
const MIN_PREFIXED_LENGTH = 12;

/** Keys whose *value* is a credential, compared after `normalizeKey`. */
const SECRET_KEYS = new Set([
  "token",
  "accesstoken",
  "refreshtoken",
  "idtoken",
  "authtoken",
  "sessiontoken",
  "bearertoken",
  "apikey",
  "apitoken",
  "apisecret",
  "secret",
  "secretkey",
  "clientsecret",
  "password",
  "passwd",
  "pwd",
  "passphrase",
  "authorization",
  "credential",
  "credentials",
  "privatekey",
  "signingkey",
  "dsn",
]);

/**
 * Auth schemes as *values*: exempt, because redacting `Bearer` and leaving the
 * credential standing is worse than doing nothing — the message then looks
 * scrubbed.
 */
const AUTH_SCHEMES = new Set(["bearer", "basic", "digest", "negotiate", "token"]);

/**
 * The subset of `AUTH_SCHEMES` that, as a *key*, licenses redacting the next
 * token across a bare space. `token` is excluded on purpose: as an English word
 * it is far too common ("the token was rejected" would lose `was`), and a real
 * `Authorization: token <pat>` is caught by the prefix rule anyway.
 */
const SCHEME_KEYS = new Set(["bearer", "basic", "digest", "negotiate"]);

/**
 * One token: a maximal run of the characters a credential is written from.
 *
 * `=` and `/` are excluded even though base64 uses both — including `=` would
 * swallow the `token=value` separator this pass depends on, and including `/`
 * would make a whole URL one token. Trailing `==` padding left behind reveals
 * nothing.
 */
const TOKEN_PATTERN = /[A-Za-z0-9_.+~-]+/g;

/** Only the punctuation an assignment is written with may separate a pair. */
const SEPARATOR_PATTERN = /^[ \t:="'>]*$/;

/**
 * The comparable form of a key token: everything after the last `.`, with `-`
 * and `_` removed, lower-cased.
 *
 * This is what makes `cancellation_token` safe without a word-boundary hack:
 * it is one token and normalizes to `cancellationtoken`, which is not a key.
 */
function normalizeKey(token: string): string {
  return token
    .slice(token.lastIndexOf(".") + 1)
    .replace(/[-_]/g, "")
    .toLowerCase();
}

/** Whether a token names itself as a credential. */
function looksLikeASecret(token: string): boolean {
  if (
    token.length >= MIN_PREFIXED_LENGTH &&
    SECRET_PREFIXES.some((prefix) => token.startsWith(prefix))
  ) {
    return true;
  }
  // A JWT: three base64url segments whose header always begins `eyJ`. Session
  // headers on this surface are JWTs and carry no issuer prefix.
  return token.length >= 20 && token.startsWith("eyJ") && token.split(".").length >= 3;
}

/** Whether `key`, followed by `separator`, means the next token is its value. */
function keyDirectsASecret(key: string, separator: string): boolean {
  // A separator that crosses a line is not an assignment; it is two unrelated
  // log lines that happen to be adjacent.
  if (/[\n\r]/.test(separator) || !SEPARATOR_PATTERN.test(separator)) return false;
  const normalized = normalizeKey(key);
  if (SCHEME_KEYS.has(normalized)) return true;
  if (!SECRET_KEYS.has(normalized)) return false;
  return separator.includes("=") || separator.includes(":") || key.startsWith("-");
}

/** The token pass: split into words, then ask whole-word questions. */
function scrubTokens(text: string): string {
  let out = "";
  let copied = 0;
  let changed = false;
  let previous: string | null = null;
  let separatorStart = 0;

  for (const match of text.matchAll(TOKEN_PATTERN)) {
    const start = match.index ?? 0;
    const token = match[0];
    const end = start + token.length;
    const directed =
      previous !== null &&
      keyDirectsASecret(previous, text.slice(separatorStart, start)) &&
      !AUTH_SCHEMES.has(normalizeKey(token));

    if (looksLikeASecret(token) || directed) {
      out += text.slice(copied, start) + REDACTED;
      copied = end;
      changed = true;
    }
    previous = token;
    separatorStart = end;
  }

  return changed ? out + text.slice(copied) : text;
}

/**
 * The URL pass: `https://user:pass@host/path` loses its userinfo.
 *
 * Separate from the token pass because `:` and `@` are exactly the characters
 * that pass uses as separators, so a URL's credential is invisible to it.
 */
function scrubUrlUserinfo(text: string): string {
  let out = "";
  let copied = 0;
  let changed = false;
  let index = 0;

  for (;;) {
    const offset = text.indexOf("://", index);
    if (offset < 0) break;
    const authorityStart = offset + 3;
    const delimiter = text.slice(authorityStart).search(/[/?#"'<>),;\s]/);
    const authorityEnd = delimiter < 0 ? text.length : authorityStart + delimiter;
    // `lastIndexOf`, not `indexOf`: a password may itself contain an `@`, and
    // the last one is the delimiter the URL grammar means.
    const at = text.slice(authorityStart, authorityEnd).lastIndexOf("@");
    if (at >= 0) {
      out += text.slice(copied, authorityStart) + REDACTED;
      // The `@` stays, so the result still reads as a URL.
      copied = authorityStart + at;
      changed = true;
    }
    index = authorityEnd;
    if (index >= text.length) break;
  }

  return changed ? out + text.slice(copied) : text;
}

/**
 * Removes credentials from `text`.
 *
 * A last line of defence, not the first. The first is not putting a credential
 * in a message at all — a scrubber is heuristic by construction and cannot
 * recognise a secret that looks like a word.
 */
export function scrubSecrets(text: string): string {
  return scrubTokens(scrubUrlUserinfo(text));
}

/**
 * What leaves the browser, and what does not. Wired as `beforeSend`.
 *
 * Mutates and returns the event rather than rebuilding it, because the SDK
 * hands over ownership and a rebuild would silently drop any field added by a
 * future SDK version — the opposite of the safe direction.
 *
 * Never returns `null`: this is a scrubber, not a filter. Deciding which errors
 * are worth seeing belongs in the operator's own Sentry project, where they can
 * see what they are suppressing.
 */
export function sanitizeEvent(event: ErrorEvent): ErrorEvent {
  // Identity. `sendDefaultPii: false` already withholds the IP address; this
  // covers the fields an integration could populate later. `user` is never set
  // by this app: the only stable per-person identifier the console holds is an
  // email address, and that is exactly what must not leave.
  event.server_name = undefined;
  event.user = undefined;

  // The request envelope, narrowed to the one header that earns its place.
  // Sentry derives `os` / `browser` / `device` server-side by parsing the
  // User-Agent, so dropping the envelope outright loses all platform context
  // (the lesson `httpContextIntegration` is re-added for). The URL, its query
  // string — where a magic-link `?code=` lives — and any cookie go.
  const userAgent = (event.request?.headers as Record<string, string> | undefined)?.[
    "User-Agent"
  ];
  event.request = userAgent ? { headers: { "User-Agent": userAgent } } : undefined;

  // `extra` is the free-form bag anything can be attached to, and nothing in
  // this app attaches to it deliberately.
  delete event.extra;

  // Contexts, allow-listed. Anything the SDK did not derive from the platform
  // is dropped rather than scrubbed, because an unknown context has unknown
  // shape and an allow-list fails in the safe direction.
  event.contexts = {
    os: event.contexts?.os,
    browser: event.contexts?.browser,
    device: event.contexts?.device,
  };

  if (event.message) event.message = scrubSecrets(event.message);

  for (const exception of event.exception?.values ?? []) {
    if (exception.value) exception.value = scrubSecrets(exception.value);
    if (exception.mechanism) delete exception.mechanism.data;
    // Frame locals and source snippets: a captured local is a captured
    // credential, and source context is this repository's own code, which the
    // operator already has.
    for (const frame of exception.stacktrace?.frames ?? []) {
      delete frame.vars;
      delete frame.context_line;
      delete frame.pre_context;
      delete frame.post_context;
    }
  }

  // Breadcrumbs are kept, unlike the vendored runtime's console, which drops
  // them wholesale. They are the single most useful thing in a browser crash
  // report — which route, which request, which status — and the integrations
  // that carry *content* (`console`, `dom`) are switched off at the source in
  // `@/lib/sentry` instead. What is left is a method, a URL and a status, and
  // the URL goes through the same scrubber as everything else.
  for (const breadcrumb of event.breadcrumbs ?? []) {
    if (breadcrumb.message) breadcrumb.message = scrubSecrets(breadcrumb.message);
    for (const [key, value] of Object.entries(breadcrumb.data ?? {})) {
      if (typeof value === "string" && breadcrumb.data) {
        breadcrumb.data[key] = scrubSecrets(value);
      }
    }
  }

  const tags: Record<string, string> = {};
  for (const [key, value] of Object.entries(event.tags ?? {})) {
    if (typeof value === "string") tags[key] = scrubSecrets(value);
  }
  event.tags = { ...tags, surface: SURFACE };

  return event;
}

// ---------------------------------------------------------------------------
// The enable/disable decision
// ---------------------------------------------------------------------------

/** The build-time variables this surface reads. */
export interface CrashReportingEnv {
  /** The DSN. Unset, blank or unusable means silence. */
  VITE_SENTRY_DSN?: string;
  /** Overrides the `environment` tag. */
  VITE_SENTRY_ENVIRONMENT?: string;
  /** `true` fires one smoke event at init. Anything else does nothing. */
  VITE_SENTRY_SMOKE_TEST?: string;
  /** Vite's own dev-server flag, which names the default environment. */
  DEV?: boolean;
}

/** Everything `Sentry.init` needs, once the decision is to report. */
export interface CrashReportingConfig {
  dsn: string;
  environment: string;
  release: string;
  smokeTest: boolean;
}

/**
 * Whether a string is a Sentry DSN.
 *
 * The same four checks `observability::config::parse_dsn` makes on the host,
 * through the platform's own URL parser rather than a second hand-rolled
 * reader: an `http`/`https` scheme, a public key, a host, and a project id. A
 * URL that parses but is not a DSN is precisely the input that resolves to
 * "reporting" and then never delivers.
 *
 * A DSN carrying a password is refused rather than repaired: the
 * `https://key:secret@…` form has not been accepted by any ingest since 2016,
 * so it is either a stale copy or a credential pasted into the wrong variable.
 */
function isUsableDsn(candidate: string): boolean {
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return false;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return false;
  if (!url.username || url.password) return false;
  if (!url.hostname) return false;
  return url.pathname.split("/").pop() !== "";
}

/**
 * Resolves what this bundle will do, or `null` for silence.
 *
 * `release` is passed in rather than computed here: it is built once in
 * `vite.config.ts`, where the source-map upload needs the same string, and a
 * release the bundle and the upload disagree about is un-symbolicated stack
 * traces with no error to explain them.
 */
export function resolveCrashReporting(
  env: CrashReportingEnv,
  release: string,
): CrashReportingConfig | null {
  const dsn = (env.VITE_SENTRY_DSN ?? "").trim();
  if (!dsn || !isUsableDsn(dsn)) return null;
  const environment = (env.VITE_SENTRY_ENVIRONMENT ?? "").trim().toLowerCase();
  return {
    dsn,
    // The host defaults this to its deployment kind (`self-hosted`,
    // `hosted-tenant`, `desktop`), which a browser bundle cannot know. Set
    // `VITE_SENTRY_ENVIRONMENT` to the host's value when the two surfaces
    // should line up in one Sentry filter.
    environment: environment || (env.DEV ? "development" : "production"),
    release,
    // Exactly `true`. Vite env values are always strings, so a loose check
    // would make `VITE_SENTRY_SMOKE_TEST=false` fire the event.
    smokeTest: env.VITE_SENTRY_SMOKE_TEST === "true",
  };
}
