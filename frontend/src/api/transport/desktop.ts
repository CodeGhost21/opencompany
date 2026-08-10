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

/** The Tauri entry points this module uses. */
interface DesktopBridge {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
}

/** What `oc_embedded` answers with. Mirrors `EmbeddedInfo` in Rust. */
export interface EmbeddedInfo {
  baseUrl: string;
  dataDir: string;
}

/**
 * How a connection authenticates, in the shape `oc_connect` takes.
 *
 * Deliberately not the console's `Credential`: this module is the boundary
 * where a credential stops being a record the UI passes around and becomes an
 * argument to the core. A `device` credential names a keychain entry rather
 * than holding a token, and nothing resolves one yet — see `registerConnection`.
 */
export interface DesktopCredential {
  deviceSession?: string;
  platformToken?: string;
}

function bridge(): DesktopBridge | null {
  if (typeof window === "undefined") return null;
  const found = (window as { __TAURI__?: DesktopBridge }).__TAURI__;
  return typeof found?.invoke === "function" ? found : null;
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
  const desktop = bridge();
  if (!desktop) return Promise.resolve();

  const pending = desktop
    .invoke<void>("oc_connect", {
      connectionId: id,
      baseUrl,
      deviceSession: credential.deviceSession ?? null,
      platformToken: credential.platformToken ?? null,
    })
    .then(
      () => undefined,
      (error: unknown) => {
        console.error(`[desktop] could not register connection ${id}`, error);
      },
    );
  registrations.set(id, pending);
  return pending;
}

/** Drops a host from the core. */
export function forgetConnection(id: string): void {
  registrations.delete(id);
  void bridge()?.invoke("oc_disconnect", { connectionId: id });
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
  const desktop = bridge();
  if (!desktop) return null;
  try {
    return (await desktop.invoke<EmbeddedInfo | null>("oc_embedded")) ?? null;
  } catch (error) {
    console.error("[desktop] could not read the embedded host", error);
    return null;
  }
}

/** Test seam: forget every registration. */
export function resetDesktopRegistrations(): void {
  registrations.clear();
}
