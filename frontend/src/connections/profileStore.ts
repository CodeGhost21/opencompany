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

import type { ConnectionId, Credential } from "./types";

const INDEX_KEY = "oc.connections.v1";

/** The persisted half of a connection. Status and identity are re-probed. */
export interface ConnectionProfile {
  id: ConnectionId;
  baseUrl: string;
  label: string;
  defaultCompany: string | null;
  credential: Credential;
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
    p.credential !== null
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

/** Records a profile, replacing any entry with the same id. */
export function saveProfile(profile: ConnectionProfile): void {
  const rest = readProfiles().filter((p) => p.id !== profile.id);
  writeProfiles([...rest, profile]);
}

export function forgetProfile(id: ConnectionId): void {
  writeProfiles(readProfiles().filter((p) => p.id !== id));
}
