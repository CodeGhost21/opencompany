// The postMessage bridge to the parent console tab (docs/spec/runtime/pages.md
// §6, plan §6). A page never holds a credential of its own — every read or
// write it wants to run against the company's data goes through this bridge
// to the parent frame, which executes it with the operator's own
// authenticated session and posts the result back. Both queries and
// mutations travel the same way: GraphQL's own operation type is what
// distinguishes them, not this client.
//
// The channel is the bridge's credential. The console transfers one half of a
// `MessageChannel` to this document on load, and every request and its reply
// travel over that port. The port is document-bound: a document the page
// navigates itself to never receives it, so it cannot speak through the
// bridge (or observe a reply) no matter what it can capture from the page
// that was here before it.

const TIMEOUT_MS = 15_000;

// The per-document bridge capability handed to us by the console on load
// (`PagesView.tsx` mints a fresh one for every iframe document). Every
// `oc:graphql` message carries it, so the console has a second, redundant
// check on top of the port it transferred with the same `oc:init` message.
let capability: string | null = null;
// The document-bound port the console transferred with `oc:init`. Requests go
// out over it and replies come back on it; possession of the port is the
// actual authorization, which is why the capability is only a backstop.
let port: MessagePort | null = null;
let initWaiters: Array<() => void> = [];

function waitForInit(): Promise<void> {
  if (port) return Promise.resolve();
  return new Promise((resolve) => {
    initWaiters.push(resolve);
  });
}

window.addEventListener("message", function onInit(event: MessageEvent) {
  const data = event.data as { type?: unknown; capability?: unknown } | null;
  if (data && data.type === "oc:init" && typeof data.capability === "string") {
    capability = data.capability;
    port = event.ports[0] ?? null;
    if (port) port.onmessage = onResultMessage;
    const waiters = initWaiters;
    initWaiters = [];
    for (const resolve of waiters) resolve();
  }
});

/** The shape a GraphQL round trip resolves to, mirroring the server's own envelope. */
export interface GraphQLResult<T = unknown> {
  data?: T;
  errors?: unknown;
}

interface BridgeResultMessage {
  type: "oc:graphql:result";
  id: string;
  data?: unknown;
  errors?: unknown;
}

function isBridgeResult(value: unknown): value is BridgeResultMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { type?: unknown }).type === "oc:graphql:result" &&
    typeof (value as { id?: unknown }).id === "string"
  );
}

/** In-flight round trips, keyed by their correlation `id`. */
const pending = new Map<string, (result: GraphQLResult) => void>();

function onResultMessage(event: MessageEvent) {
  const data = event.data as BridgeResultMessage | null;
  if (!data || !isBridgeResult(data)) return;
  const resolve = pending.get(data.id);
  if (!resolve) return;
  pending.delete(data.id);
  resolve({ data: data.data as unknown, errors: data.errors });
}

/**
 * Runs one GraphQL operation — query or mutation — against the console's own
 * GraphQL endpoint, by way of the parent frame.
 *
 * Internally: generates a random correlation `id`, posts
 * `{type: "oc:graphql", id, capability, query, variables}` over the
 * document-bound port, and resolves when a matching
 * `{type: "oc:graphql:result", id, ...}` reply arrives on the same port. The
 * `id` is what lets several concurrent calls share the port without their
 * replies crossing.
 */
function query<T = unknown>(
  document: string,
  variables?: Record<string, unknown>,
): Promise<GraphQLResult<T>> {
  return new Promise((resolve, reject) => {
    const id =
      typeof crypto !== "undefined" && "randomUUID" in crypto
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const timeout = window.setTimeout(() => {
      pending.delete(id);
      reject(new Error("oc:graphql timed out waiting for a reply from the console"));
    }, TIMEOUT_MS);

    pending.set(id, (result: GraphQLResult) => {
      window.clearTimeout(timeout);
      // The reply's payload is opaque to the bridge — it is whatever GraphQL
      // returned for this document — so the unknown only becomes `T` here, at
      // the point the caller's generic names it. This is the one cast.
      resolve(result as GraphQLResult<T>);
    });

    // `targetOrigin` is deliberately `"*"` on the outgoing `oc:init` that
    // delivered this port, and the port itself needs no origin: the console
    // minted the channel and transferred one half to exactly this document,
    // so any message arriving on the other half is from here by construction.
    waitForInit().then(() => {
      port?.postMessage({ type: "oc:graphql", id, capability, query: document, variables });
    });
  });
}

/** The one live-data surface a page has: `client.query(document, variables)`. */
export const client = { query };
