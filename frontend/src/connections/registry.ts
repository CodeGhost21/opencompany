// The set of hosts this console is talking to, and their live status.
//
// A module-level store read through React 18's `useSyncExternalStore`. No state
// library, deliberately: the console has none today, and the absence of a
// global query cache is the single biggest thing protecting this refactor. A
// cache keyed on anything less than (connection, company) is exactly how two
// hosts' data gets mixed, and the way to not have that bug is to not have the
// cache.
//
// Note what is *not* here: an "active connection". Selecting a connection in
// the UI is a rendering choice, not a state change in this module — every
// connection stays probed and, later, streamed. A single-valued active
// connection is precisely what stops buzz from holding more than one relay, and
// reintroducing it here would undo the whole slice.

import { useSyncExternalStore } from "react";

import { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import { defaultTransport } from "@/api/transport";
import type { Transport } from "@/api/transport";
import { findProfile, forgetProfile, readProfiles, saveProfile } from "./profileStore";
import {
  type Connection,
  type ConnectionId,
  type Credential,
  type InstanceIdentity,
  connectionConfig,
} from "./types";

/** The alphabet a generated connection id uses. Excludes `:` — see `scopedKey`. */
const ID_ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789";

function mintId(): ConnectionId {
  let out = "";
  const bytes = new Uint8Array(12);
  if (typeof crypto !== "undefined" && "getRandomValues" in crypto) {
    crypto.getRandomValues(bytes);
  } else {
    // Node, under the unit tests. Uniqueness within one process is all that is
    // needed there; this never runs in a browser.
    for (let i = 0; i < bytes.length; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  }
  for (const byte of bytes) out += ID_ALPHABET[byte % ID_ALPHABET.length];
  return out;
}

interface Entry {
  connection: Connection;
  client: OpenCompanyClient;
}

let entries: Entry[] = [];
let listeners: Array<() => void> = [];
/**
 * The snapshot handed to React.
 *
 * Cached and only replaced when something actually changes, because
 * `useSyncExternalStore` compares snapshots by identity and would loop forever
 * on a fresh array every call.
 */
let snapshot: Connection[] = [];
/**
 * Connections with a probe in flight.
 *
 * Without this, probing is self-perpetuating: `probe` sets the status to
 * `connecting`, that emits a new snapshot, and any effect watching the
 * connection list and looking for `connecting` fires again — probing forever
 * and taking the tab down with it. Making the guard a property of the registry
 * rather than of the caller means every future caller inherits it.
 */
const probing = new Set<ConnectionId>();

function emit(): void {
  snapshot = entries.map((e) => e.connection);
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.push(listener);
  return () => {
    listeners = listeners.filter((l) => l !== listener);
  };
}

function getSnapshot(): Connection[] {
  return snapshot;
}

function patch(id: ConnectionId, change: Partial<Connection>): void {
  let touched = false;
  entries = entries.map((entry) => {
    if (entry.connection.id !== id) return entry;
    touched = true;
    return { ...entry, connection: { ...entry.connection, ...change } };
  });
  if (touched) emit();
}

/** Every connection, for rendering. */
export function useConnections(): Connection[] {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/** One connection, or `undefined` once it has been removed. */
export function useConnection(id: ConnectionId | null): Connection | undefined {
  const all = useConnections();
  return id === null ? undefined : all.find((c) => c.id === id);
}

export function listConnections(): Connection[] {
  return snapshot;
}

export function getConnection(id: ConnectionId): Connection | undefined {
  return entries.find((e) => e.connection.id === id)?.connection;
}

/**
 * The client for a connection.
 *
 * One per connection and reused, so that `useEffect` dependencies keyed on the
 * client do not re-fire on every render — and so a connection's `onUnauthorized`
 * hook belongs to that connection alone.
 */
export function clientFor(id: ConnectionId): OpenCompanyClient | undefined {
  return entries.find((e) => e.connection.id === id)?.client;
}

export interface AddConnection {
  baseUrl: string;
  label?: string;
  /** The company the client addresses by default; `null` for the alias form. */
  defaultCompany?: string | null;
  credential?: Credential;
  /** Injected in tests, and by the desktop shell. */
  transport?: Transport;
}

/**
 * Registers a host, reusing its remembered id when there is one.
 *
 * **Reuse is not a nicety.** Every browser-local key is scoped by the
 * connection id, so a freshly minted id on each page load would orphan the tour
 * state, the last-read channel and the mail draft on every reload — with no
 * error anywhere. `findProfile` is what makes the id stable for a host across
 * reloads; see `profileStore.ts`.
 *
 * Does not contact the host — call {@link probe}.
 */
export function addConnection(input: AddConnection): ConnectionId {
  const baseUrl = input.baseUrl.replace(/\/$/, "");
  const defaultCompany = input.defaultCompany ?? null;
  const remembered = findProfile(baseUrl, defaultCompany);

  // Already registered this session (StrictMode double-invokes, and the web
  // build adds its bootstrap connection from a `useMemo`). Hand back the
  // existing entry rather than a duplicate row for one host.
  if (remembered) {
    const existing = entries.find((e) => e.connection.id === remembered.id);
    if (existing) return existing.connection.id;
  }

  const id = remembered?.id ?? mintId();
  const connection: Connection = {
    id,
    label: input.label ?? remembered?.label ?? hostLabel(baseUrl),
    baseUrl,
    defaultCompany,
    credential: input.credential ?? { kind: "cookie" },
    status: "connecting",
    identity: null,
    companies: [],
  };
  // The desktop routes this connection through its own core; the browser
  // build keeps `fetch`. `defaultTransport` decides, so neither the registry
  // nor the client has to know which shell it is in.
  const client = new OpenCompanyClient(
    connectionConfig(connection),
    input.transport ?? defaultTransport(id),
  );
  // Per connection, so one host refusing this client's credential marks that
  // row and leaves the other N-1 alone. The globally-fatal version of this is
  // what made a single expired session blank the whole console.
  client.onUnauthorized = () => patch(id, { status: "unauthenticated" });
  entries = [...entries, { connection, client }];
  saveProfile({
    id,
    baseUrl,
    label: connection.label,
    defaultCompany,
    credential: connection.credential,
  });
  emit();
  return id;
}

/**
 * Registers every host remembered from a previous session.
 *
 * Without this the profile store would only ever *stabilise the id* of a
 * connection something else re-added — so the bootstrap host would come back on
 * reload and every host the operator added by hand would quietly not. Restoring
 * is what makes "connected to N hosts" a property of the client rather than of
 * one page load.
 *
 * Idempotent: `addConnection` returns the existing entry for a host already
 * registered, so calling this alongside the bootstrap add cannot double up.
 */
export function restoreConnections(transport?: Transport): ConnectionId[] {
  return readProfiles().map((profile) =>
    addConnection({
      baseUrl: profile.baseUrl,
      label: profile.label,
      defaultCompany: profile.defaultCompany,
      credential: profile.credential,
      transport,
    }),
  );
}

export function removeConnection(id: ConnectionId): void {
  entries = entries.filter((e) => e.connection.id !== id);
  forgetProfile(id);
  emit();
}

/** Drops every connection. Tests only — the app never empties the registry. */
export function resetConnections(): void {
  entries = [];
  probing.clear();
  emit();
}

/**
 * Contacts a host and records what it found.
 *
 * This is `App`'s old `boot()`, moved here so that discovering the second host
 * is the same code as discovering the first. It resolves rather than throws:
 * a connection that cannot be reached is a *state*, not an exception, because
 * the other connections carry on regardless.
 */
export async function probe(id: ConnectionId): Promise<void> {
  const client = clientFor(id);
  if (!client || probing.has(id)) return;
  probing.add(id);
  patch(id, { status: "connecting", error: undefined });
  try {
    await runProbe(id, client);
  } finally {
    probing.delete(id);
  }
}

async function runProbe(id: ConnectionId, client: OpenCompanyClient): Promise<void> {

  const identity = await readIdentity(client);
  if (identity) patch(id, { identity, label: identity.displayName ?? labelOf(id) });

  try {
    const companies = await client.listCompanies();
    patch(id, { status: "live", companies: companies.map((c) => c.id) });
    return;
  } catch (listErr) {
    // A single-company (prosumer) host has no `/api/v1/companies`; its sole
    // company answers on the alias instead. Falling back rather than failing is
    // what lets one client hold a platform host and a prosumer host at once.
    try {
      await client.status(null);
      patch(id, { status: "live", companies: [] });
      return;
    } catch (statusErr) {
      patch(id, statusFromError(statusErr ?? listErr));
    }
  }
}

/** Reads `/spec`, tolerating a host that has no identity fields yet. */
async function readIdentity(client: OpenCompanyClient): Promise<InstanceIdentity | null> {
  try {
    const spec = await client.get<Record<string, unknown>>("/spec");
    return {
      instanceId: typeof spec.instance_id === "string" ? spec.instance_id : undefined,
      displayName: typeof spec.display_name === "string" ? spec.display_name : undefined,
      // `undefined` is meaningfully different from `[]`: an older host omits
      // the field, and the client must read that as "assume REST only" rather
      // than as "supports nothing".
      capabilities: Array.isArray(spec.capabilities)
        ? (spec.capabilities as string[])
        : undefined,
      storage: typeof spec.storage === "string" ? spec.storage : undefined,
    };
  } catch {
    return null;
  }
}

function statusFromError(err: unknown): Partial<Connection> {
  if (err instanceof ApiError && err.status === 401) {
    return { status: "unauthenticated", error: "this host refused the credential" };
  }
  const message = err instanceof ApiError ? err.message : "could not be reached";
  return { status: "down", error: message };
}

function labelOf(id: ConnectionId): string {
  return getConnection(id)?.label ?? id;
}

/** A readable name for a host before it has told us its own. */
export function hostLabel(baseUrl: string): string {
  if (!baseUrl) return "This host";
  try {
    return new URL(baseUrl).host;
  } catch {
    return baseUrl;
  }
}
