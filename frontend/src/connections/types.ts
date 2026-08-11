// What the console knows about the hosts it is connected to.
//
// The console used to resolve exactly one host at startup and never reconsider.
// A desktop client holds several at once, so "which host" becomes a dimension
// of almost everything: which client issues a request, which event stream a
// frame arrived on, which browser-local state a view reads.
//
// The failure mode this file exists to prevent is the one block/buzz shipped:
// a single-valued `activeConnection` in app state, plus local storage keyed by
// *company* rather than by connection. Two connections that both host a company
// called `acme` then silently share their tour progress, their last-read
// channel, and their draft mail settings. Nothing errors; the state is just
// quietly wrong.

import type { ConsoleConfig } from "@/config";

/**
 * A connection's identity within this client.
 *
 * **Minted locally and never derived from the URL.** A host moves between
 * `localhost:8080`, a LAN address and a tunnel over one afternoon, and two
 * different hosts can occupy one URL over time. Deriving the id from the URL
 * would lose a connection's history the moment it moved, and would merge two
 * hosts that happened to share an address.
 *
 * Distinct from the server's own `instance_id` (see `/spec`), which identifies
 * the *host*. Two connection entries may legitimately point at one instance —
 * the same server reached with a personal session and with a device token, say
 * — so the client's key cannot be the server's.
 */
export type ConnectionId = string;

/** How this client proves who it is to one host. */
export type Credential =
  /** A browser session cookie. Same-origin only; the web build's normal case. */
  | { kind: "cookie" }
  /**
   * A paired device session, presented as a header.
   *
   * `ref` is a handle into the OS keychain, **never the secret**. Connection
   * records are persisted and passed around the UI; a token in one would end up
   * in local storage and in every React devtools inspection.
   */
  | { kind: "device"; ref: string }
  /**
   * A platform bearer, from `?token=`. Machine credentials, not a person.
   *
   * `token` is optional in a *persisted* profile: `saveProfile` redacts it, so
   * `localStorage` never holds the secret. A token-less `{ kind: "platform" }`
   * is a marker meaning "this host authenticates as the platform" — the live
   * token is re-derived from the URL on the next load.
   */
  | { kind: "platform"; token?: string };

/**
 * Where a connection stands right now.
 *
 * Per connection, deliberately: the whole point of holding several is that one
 * being unreachable is not the app being broken. A global status would collapse
 * that back into the single-connection world.
 */
export type ConnectionStatus =
  | "connecting"
  | "live"
  /** Reachable, but something it advertised is not answering. */
  | "degraded"
  | "down"
  /** Reached, but this client's credential was refused. */
  | "unauthenticated";

/** What a host said about itself at `/spec`. */
export interface InstanceIdentity {
  /** The host's own stable id. Absent on a host predating it. */
  instanceId?: string;
  displayName?: string;
  /**
   * Flat feature names. **Absent means "assume REST only"** — an older host
   * omits the field entirely, so `undefined` and `[]` must not be conflated.
   */
  capabilities?: string[];
  storage?: string;
}

/**
 * Where a connection came from, when that is not "someone typed an address".
 *
 * Only `embedded` so far: the host running inside this application. It is
 * singular in a way no other connection is — there is exactly one per client,
 * this client both starts it and knows its identity without asking, and its
 * address is regenerated on every launch. That last property is why the
 * marker has to be durable: recognising last launch's row as *this* host is
 * what stops a dead one accumulating per run.
 */
export type ConnectionOrigin = "embedded";

export interface Connection {
  id: ConnectionId;
  /**
   * The company this connection's client addresses when a caller names none.
   *
   * `null` selects the host's single-company aliases. Carried on the connection
   * because it is a property of how this client was configured for this host —
   * `?company=` names one, a prosumer host has none — not of the host itself.
   */
  defaultCompany: string | null;
  /** What to call this connection in the UI, before `/spec` answers. */
  label: string;
  /** Empty string means same-origin. */
  baseUrl: string;
  credential: Credential;
  status: ConnectionStatus;
  /** `null` until `/spec` answers, and on a host that has no identity surface. */
  identity: InstanceIdentity | null;
  /** Companies this connection serves, once discovered. */
  companies: string[];
  /** Why it is `down` or `unauthenticated`, for the connection's own row. */
  error?: string;
  origin?: ConnectionOrigin;
}

/** The fields needed to construct a connection's client. */
export function connectionConfig(
  connection: Connection,
): Pick<ConsoleConfig, "baseUrl" | "company" | "operatorToken"> {
  return {
    baseUrl: connection.baseUrl,
    company: connection.defaultCompany,
    operatorToken:
      connection.credential.kind === "platform"
        ? (connection.credential.token ?? null)
        : null,
  };
}

/**
 * Identifies whose browser-local state a key belongs to.
 *
 * Both halves are required. Company alone is what buzz keyed on, and it is
 * wrong as soon as two connections serve a company of the same name; connection
 * alone is wrong as soon as one connection serves two companies.
 */
export interface LocalScope {
  connection: ConnectionId;
  /** `null` for a single-company host, which has no id to name. */
  company: string | null;
}

/**
 * A `localStorage` key for one connection's view of one company.
 *
 * Every browser-local key in the console goes through here, so there is one
 * place that decides what "whose state is this" means. `::` separates the two
 * halves because a connection id is generated from a fixed alphabet that
 * excludes it, so the split is unambiguous.
 */
export function scopedKey(prefix: string, scope: LocalScope): string {
  return `${prefix}:${scope.connection}::${scope.company ?? "single"}`;
}

/**
 * The scoped key, having first adopted whatever the pre-connection console left
 * under `legacyName`.
 *
 * Every one of these keys existed before connections did, keyed on company
 * alone. Simply moving to a scoped key would have silently discarded all of it:
 * an operator's tour progress and last-read channel would reset on upgrade, and
 * — worse — `readLegacyLocalNodes` would stop finding the retired workspace
 * scratchpad it exists to migrate, so notes someone typed would become
 * unreachable with nothing reporting it.
 *
 * Adoption is a copy, not a move, and it happens per connection. Old state
 * cannot be attributed to a host that did not exist when it was written, so
 * every connection that looks inherits it once and they diverge from there.
 * That is the honest reading of "this is what you had before".
 *
 * The legacy key is never written again, so a second run is a no-op.
 */
export function scopedKeyAdoptingLegacy(
  prefix: string,
  scope: LocalScope,
  legacyName: string,
): string {
  const key = scopedKey(prefix, scope);
  try {
    const store = window.localStorage;
    if (store.getItem(key) === null) {
      const inherited = store.getItem(legacyName);
      if (inherited !== null) store.setItem(key, inherited);
    }
  } catch {
    // Private mode or a blocked store. The caller degrades to no memory, which
    // is what it did before this key existed.
  }
  return key;
}
