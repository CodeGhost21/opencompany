// Telling the Rust core which hosts this console talks to.
//
// `ProxyTransport` addresses a host by connection id, and the core resolves
// that id against `ProxyRegistry`. Nothing in the console registered anything,
// so every proxied request came back `no such connection: <id>` — the desktop
// could not complete one round trip. This module is the missing half.
//
// ## Registration is awaited, not fired and forgotten
//
// `addConnection` is synchronous (React renders off it) and `oc_connect` is
// not. Kicking the command off and hoping it lands first is a race the console
// loses on a fast probe, and the symptom — an unreachable host that becomes
// reachable on retry — reads like a network fault rather than an ordering bug.
//
// So each registration is kept as a promise and `ProxyTransport` awaits it
// before its first call. After that the promise is already resolved and the
// await costs a microtask.
//
// ## No implicit current connection
//
// Every entry point here takes an explicit id, for the same reason the Rust
// side does: a single "active connection" is what stops comparable clients
// from holding more than one host, and a convenience default here would
// reintroduce it above the seam instead of below it.

import { tauriCore } from "./bridge";

/** What `oc_embedded` answers with. Mirrors `EmbeddedInfo` in Rust. */
export interface EmbeddedInfo {
  baseUrl: string;
  dataDir: string;
  /**
   * Who is listening at `baseUrl`, as opposed to where.
   *
   * Optional here though the core always sends it, because this is the one
   * field a *stale* build of the shell would omit: the console is bundled into
   * the binary, but a developer running `pnpm dev` against an older `cargo`
   * build is an ordinary Tuesday. Absent degrades to the pre-identity
   * behaviour rather than to a connection keyed on `undefined`.
   */
  instanceId?: string;
  /**
   * The address this host will sign a person in as (#632).
   *
   * A desktop install has one standing admin and no mail transport, so nobody
   * could guess what to type and every other address gets the same silent
   * acknowledgement. Optional for the same reason as `instanceId`: an older
   * shell does not send it, and a blank sign-in form is the honest degrade.
   */
  operatorEmail?: string;
}

/**
 * How a connection authenticates, in the shape `oc_connect` takes.
 *
 * Note what is absent: any device token. The core resolves a paired device's
 * session from the keychain by connection id, so the console has nothing to
 * pass and — more to the point — nothing to leak. Only the platform bearer
 * travels, because that one genuinely arrives in the URL.
 */
export interface DesktopCredential {
  platformToken?: string;
}

/** What pairing tells the console. Carries no secret. */
export interface PairedDevice {
  company: string;
  deviceId: string;
  expiresAtMillis: number;
}

/** Connections the core has been told about, by id. */
const registrations = new Map<string, Promise<void>>();

/**
 * Registers a host with the core, replacing any previous registration for `id`.
 *
 * Resolves when the core has the connection. A failure is swallowed into a
 * resolved promise rather than left rejected: the transport awaits this on
 * every call, and an unhandled rejection stored in a module-level map would
 * resurface on each one. The request that follows fails on its own merits with
 * `no such connection`, which is the honest error and the one the console
 * already renders per row.
 */
export function registerConnection(
  id: string,
  baseUrl: string,
  credential: DesktopCredential = {},
): Promise<void> {
  const desktop = tauriCore();
  if (!desktop) return Promise.resolve();

  // Chained onto whatever is already parked under this id, in both directions.
  // `forgetConnection` sequences its disconnect behind a pending registration;
  // without the same courtesy here, a re-register racing an in-flight
  // `oc_disconnect` can land first and then be disconnected by it — the same
  // ordering bug, mirrored. Whichever call came last wins, which is what a
  // caller means by calling it.
  const previous = registrations.get(id) ?? Promise.resolve();
  const pending = previous
    .then(() =>
      desktop.invoke<void>("oc_connect", {
        connectionId: id,
        baseUrl,
        platformToken: credential.platformToken ?? null,
      }),
    )
    .then(
      () => undefined,
      (error: unknown) => {
        console.error(`[desktop] could not register connection ${id}`, error);
      },
    );
  registrations.set(id, pending);
  return pending;
}

/**
 * Drops a host from the core.
 *
 * Sequenced after any registration still in flight. This and
 * `registerConnection` race the same way a request does: `oc_connect` resolving
 * *after* `oc_disconnect` landed would leave the connection registered in the
 * core while the console believes it is gone — a dangling entry that the next
 * `registerConnection` for a reused id may or may not overwrite cleanly.
 *
 * Stays synchronous because callers are React event handlers, so the ordering
 * is expressed by chaining onto the pending promise rather than by awaiting it.
 * The map entry is cleared only once the disconnect has been issued; deleting
 * it up front is what dropped the ordering in the first place.
 */
export function forgetConnection(id: string): void {
  const desktop = tauriCore();
  if (!desktop) {
    registrations.delete(id);
    return;
  }
  const pending = registrations.get(id) ?? Promise.resolve();
  const disconnected: Promise<void> = pending
    .then(() => desktop.invoke<void>("oc_disconnect", { connectionId: id }))
    .then(
      () => undefined,
      (error: unknown) => {
        console.error(`[desktop] could not drop connection ${id}`, error);
      },
    )
    .finally(() => {
      // Only remove the entry this call created. A `registerConnection` that
      // landed while the disconnect was in flight owns the id now, and clearing
      // its promise would let a request run before its registration.
      if (registrations.get(id) === disconnected) registrations.delete(id);
    });
  // Parked under the same id so a `connectionReady` in between waits for the
  // disconnect rather than racing past it.
  registrations.set(id, disconnected);
}

/**
 * Resolves once `id` is registered, immediately when there is nothing pending.
 *
 * The "nothing pending" case is not an error: it covers the browser build and
 * any caller that registered before this module was involved.
 */
export function connectionReady(id: string): Promise<void> {
  return registrations.get(id) ?? Promise.resolve();
}

/**
 * The in-process host, when this build has one.
 *
 * `null` in a browser, and also in a desktop whose embedded host failed to
 * start — most often because another instance holds the data root. That is a
 * state the console shows rather than a reason to have no console: the point of
 * the desktop is that it can talk to remote hosts too.
 */
export async function embeddedHost(): Promise<EmbeddedInfo | null> {
  const desktop = tauriCore();
  if (!desktop) return null;
  try {
    return (await desktop.invoke<EmbeddedInfo | null>("oc_embedded")) ?? null;
  } catch (error) {
    console.error("[desktop] could not read the embedded host", error);
    return null;
  }
}

/**
 * Redeems a pairing code for this machine.
 *
 * The token never comes back. It exists for one HTTP response, and the core
 * keeps that response to itself: it writes the session to the OS keychain and
 * answers with only what a person needs to see. A console that received the
 * token — even to hand it straight back — would be a console an injected script
 * could read it from.
 */
export async function pairDevice(
  id: string,
  baseUrl: string,
  code: string,
  label?: string,
): Promise<PairedDevice> {
  const desktop = tauriCore();
  if (!desktop) throw new Error("pairing a device needs the desktop application");
  return desktop.invoke<PairedDevice>("oc_pair_device", {
    connectionId: id,
    baseUrl,
    code,
    label: label ?? null,
  });
}

/**
 * Forgets this machine's stored session for a connection.
 *
 * Local only: the session still exists on the host until someone revokes it
 * from the devices list there. Conflating the two would mean unpairing one
 * laptop cut off another.
 */
export async function forgetDevice(id: string): Promise<void> {
  await tauriCore()?.invoke("oc_forget_device", { connectionId: id });
}

/** Test seam: forget every registration. */
export function resetDesktopRegistrations(): void {
  registrations.clear();
}
