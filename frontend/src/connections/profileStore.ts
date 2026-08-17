// Remembering which hosts this client knows, across reloads.
//
// ## Why this is not optional
//
// A connection id is minted randomly, and every browser-local key in the
// console is scoped by it (see `scopedKey`). If the id were re-minted on each
// page load, then every reload would move the tour state, the last-read
// channel, the draft mail settings and the workspace migration flag to a fresh
// namespace — orphaning the old one. Nothing would error; the console would
// just have amnesia, and the cause would be nowhere near the symptom.
//
// So ids are minted **once per host** and persisted here.
//
// ## What is deliberately not stored
//
// Secrets. A `Credential` may name a keychain handle, never a token: this file
// writes to `localStorage`, which is readable by any script that gets into the
// page. The desktop's keychain holds the actual device token and hands it to
// the Rust core, which is the only thing that ever sees it.
//
// OpenHuman's own `profileStore.ts` concedes in a comment that keeping secrets
// in desktop localStorage is a shortcut it means to undo. This one does not
// inherit the shortcut.

import type { ConnectionId, ConnectionOrigin, Credential } from "./types";

const INDEX_KEY = "oc.connections.v1";

/**
 * The label this client gives the host running inside it.
 *
 * Exported because two places need to agree on it: the registry, which writes
 * it, and {@link embeddedProfiles}, which recognises what older versions wrote.
 */
export const EMBEDDED_LABEL = "This computer";

/** The persisted half of a connection. Status is re-probed. */
export interface ConnectionProfile {
  id: ConnectionId;
  baseUrl: string;
  label: string;
  defaultCompany: string | null;
  credential: Credential;
  /**
   * The host's own `instance_id`, when this client knew it going in.
   *
   * Only the embedded host does: the core reads it off disk and hands it over
   * IPC. For a remote host this stays absent, because the id arrives from
   * `/spec` — after the point where it would have been useful for matching.
   */
  instanceId?: string;
  origin?: ConnectionOrigin;
}

function storage(): Storage | null {
  // Access itself throws in Safari private mode and under "block all cookies".
  // A console that cannot remember its hosts must still render them.
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readProfiles(): ConnectionProfile[] {
  const raw = storage()?.getItem(INDEX_KEY);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    // Validate rather than cast. This is user-writable storage, and a
    // half-written or hand-edited entry must not become a connection whose
    // `id` is `undefined` — which would collapse every scoped key onto one
    // shared namespace, silently.
    return parsed.filter(isProfile);
  } catch {
    return [];
  }
}

function isProfile(value: unknown): value is ConnectionProfile {
  if (typeof value !== "object" || value === null) return false;
  const p = value as Record<string, unknown>;
  return (
    typeof p.id === "string" &&
    p.id.length > 0 &&
    typeof p.baseUrl === "string" &&
    typeof p.label === "string" &&
    (p.defaultCompany === null || typeof p.defaultCompany === "string") &&
    typeof p.credential === "object" &&
    p.credential !== null &&
    // Absent on every profile written before these existed, which is the
    // common case on an upgrade — so missing is valid and only a *wrong* type
    // disqualifies the entry.
    (p.instanceId === undefined || typeof p.instanceId === "string") &&
    (p.origin === undefined || p.origin === "embedded")
  );
}

export function writeProfiles(profiles: ConnectionProfile[]): void {
  try {
    storage()?.setItem(INDEX_KEY, JSON.stringify(profiles));
  } catch {
    /* quota or private mode: the console still works, it just re-mints */
  }
}

/**
 * The stored profile for a host, matched the way a *browser* has to match.
 *
 * Keyed on `(baseUrl, defaultCompany)`. Not on the server's `instance_id`,
 * even though that is the better identity: the match has to happen *before*
 * the host is contacted, because the id is needed to construct the client that
 * would do the contacting. Matching on what we knew going in is the only
 * option available at that moment.
 *
 * `defaultCompany` is part of the key because `?company=a` and `?company=b`
 * against one host are two different consoles, and their view state should not
 * be shared.
 */
export function findProfile(
  baseUrl: string,
  defaultCompany: string | null,
): ConnectionProfile | undefined {
  const normalized = baseUrl.replace(/\/$/, "");
  return readProfiles().find(
    (p) => p.baseUrl === normalized && p.defaultCompany === defaultCompany,
  );
}

/**
 * Every profile that is — or once was — the host running inside this client.
 *
 * `origin` says so outright for anything this version wrote. Older versions
 * recorded nothing that distinguished the embedded host from a host someone
 * typed in, so the second clause recognises them by the signature the bug left
 * behind: the label this client gives its own host, at a loopback address.
 *
 * Narrow on purpose, because the consequence of a false positive is deleting a
 * connection an operator added. A host added by hand is labelled by `hostLabel`
 * — `127.0.0.1:8080`, never this string — and only a profile carrying *neither*
 * new field is old enough to need guessing about at all.
 */
export function embeddedProfiles(): ConnectionProfile[] {
  return readProfiles().filter(
    (p) => p.origin === "embedded" || isLegacyEmbedded(p),
  );
}

/** `http://127.0.0.1:<port>`, the only address the embedded host ever had. */
const LOOPBACK_URL = /^http:\/\/127\.0\.0\.1:\d+$/;

function isLegacyEmbedded(profile: ConnectionProfile): boolean {
  return (
    profile.origin === undefined &&
    profile.instanceId === undefined &&
    profile.label === EMBEDDED_LABEL &&
    LOOPBACK_URL.test(profile.baseUrl)
  );
}

/**
 * The credential in a persisted profile.
 *
 * A platform bearer is redacted to its kind before writing: `localStorage` is
 * readable by any script that reaches the page, and the token is `?token=` URL
 * material re-derived on the next load, never something storage needs to hold.
 *
 * ## The one secret this file does write
 *
 * A `session` credential is persisted **in full** — a deliberate exception to
 * this module's opening rule, and the only one. Redacting it would protect
 * nothing and simply break the feature: unlike a platform bearer there is
 * nowhere to re-derive it from, so a redacted session means signing in to every
 * host again on every page load, and a console that does that is one people
 * stop using.
 *
 * The cost is real and worth stating plainly. Script execution on a hub's
 * origin reaches every host its operator has signed in to, where a same-origin
 * console's `HttpOnly` cookie would have survived it. Two things bound it: the
 * token is a *session*, revocable from the host's device list and expiring on
 * its own, and this credential is only ever chosen for a connection where a
 * cookie could not have worked at all (see `Credential`).
 */
function persistedCredential(credential: Credential): Credential {
  if (credential.kind === "platform") return { kind: "platform" };
  return credential;
}

/** Records a profile, replacing any entry with the same id. */
export function saveProfile(profile: ConnectionProfile): void {
  const rest = readProfiles().filter((p) => p.id !== profile.id);
  writeProfiles([
    ...rest,
    { ...profile, credential: persistedCredential(profile.credential) },
  ]);
}

export function forgetProfile(id: ConnectionId): void {
  writeProfiles(readProfiles().filter((p) => p.id !== id));
}
