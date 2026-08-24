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

/**
 * A gesture the console relayed into this frame (issue #1303): a click (or a
 * press) on one of its own toasts that a page control underneath would have
 * received, forwarded because DOM events cannot cross the sandboxed-frame
 * boundary from the parent document. The parent shifted the coordinates into
 * this document's viewport, so `elementFromPoint` here sees the page's own
 * tree. A press is the first event of a gesture, not the whole of it: the
 * parent relays the remainder — `pointermove`, `pointerup`, `pointercancel`
 * — the same way, so a drag or press-state control sees a complete sequence
 * instead of a `pointerdown` it can never release.
 */
interface RelayMessage {
  type:
    | "oc:relay-click"
    | "oc:relay-pointerdown"
    | "oc:relay-pointermove"
    | "oc:relay-pointerup"
    | "oc:relay-pointercancel";
  x: number;
  y: number;
  pointerId?: number;
  pointerType?: string;
  isPrimary?: boolean;
  button?: number;
  buttons?: number;
  detail?: number;
}

function isRelayMessage(value: unknown): value is RelayMessage {
  if (typeof value !== "object" || value === null) return false;
  const message = value as Record<string, unknown>;
  return (
    (message.type === "oc:relay-click" ||
      message.type === "oc:relay-pointerdown" ||
      message.type === "oc:relay-pointermove" ||
      message.type === "oc:relay-pointerup" ||
      message.type === "oc:relay-pointercancel") &&
    typeof message.x === "number" &&
    typeof message.y === "number"
  );
}

// Only the console frame that hosts this page may relay gestures into it. The
// source check is what makes that true: a frame the page embeds itself would
// surface with `event.source` set to its own window, not `window.parent`.
//
// The event dispatched below is programmatic, so it is untrusted: like the
// console's own synthetic clicks, it carries no transient user activation,
// and a browser will not transfer activation across the sandbox boundary
// (that is the clickjacking defense). A control that requires activation — a
// file input, `showPicker()`, `window.open()` — is therefore not reachable
// through an overlay; the relay targets ordinary click- and pointer-driven
// controls, which is what a toast-over-page gesture is for.

/**
 * The elements a relayed press is currently on, keyed by pointer id.
 *
 * The parent relays a whole press: `pointerdown` followed by the
 * `pointermove`/`pointerup`/`pointercancel` that land in its own document.
 * Each continuation is routed to the element that took the press, not the one
 * under the point now — the same retargeting pointer capture gives a
 * same-document control, so a drag keeps tracking its handle when the pointer
 * moves over a sibling element. The entry is cleared when the press ends.
 */
const relayPressTargets = new Map<number, Element>();

/** The elements a relayed press last ended on, keyed by pointer id. */
const lastPressTargets = new Map<number, Element>();

/**
 * Remember the element that owned a completed press for its compatibility click.
 *
 * The parent relay includes the pointer id on pointer-originated click events.
 * Keeping the target after `pointerup` preserves pointer-capture semantics: a
 * release over a sibling must not make the compatibility click activate that
 * sibling instead of the element that took the press.
 */
function rememberPressTarget(pointerId: number, target: Element): void {
  lastPressTargets.set(pointerId, target);
}

function dispatchRelayedPointer(message: RelayMessage, target: Element): void {
  target.dispatchEvent(
    new PointerEvent(message.type.slice("oc:relay-".length), {
      clientX: message.x,
      clientY: message.y,
      pointerId: message.pointerId ?? 0,
      pointerType: message.pointerType ?? "mouse",
      isPrimary: message.isPrimary ?? true,
      button: message.button ?? 0,
      buttons: message.buttons ?? 1,
      detail: message.detail ?? 1,
      bubbles: true,
      cancelable: true,
    }),
  );
}

window.addEventListener("message", function onRelay(event: MessageEvent) {
  if (event.source !== window.parent) return;
  if (!isRelayMessage(event.data)) return;

  if (event.data.type === "oc:relay-click") {
    // A pointer-originated click carries the pointer id. Preserve the target
    // that took that press (pointer-capture semantics), rather than activating
    // whatever element happens to be under the release point now. Clicks with
    // no pointer id are direct/keyboard clicks and use the point as usual.
    const pointerId = event.data.pointerId;
    const pressed = pointerId === undefined ? undefined : lastPressTargets.get(pointerId);
    if (pointerId !== undefined) lastPressTargets.delete(pointerId);
    const element = pressed?.isConnected
      ? pressed
      : document.elementFromPoint(event.data.x, event.data.y);
    if (!(element instanceof HTMLElement || element instanceof SVGElement)) return;
    // Focus the control the click activates, not the leaf the point landed on:
    // `elementFromPoint` hands back the icon `<path>` or `<span>` inside a
    // button, and `focus()` on that leaf is a no-op — the button is clicked but
    // keyboard focus is left behind, where a later Enter/Space operates the
    // wrong element. A native click focuses the nearest focusable ancestor, so
    // the relay does the same.
    const focusTarget = element.closest(
      'button, a[href], input, select, textarea, [contenteditable="true"], [tabindex]',
    ) as HTMLElement | SVGElement | null;
    // Dispatch the click with the relayed coordinates: a canvas, chart or
    // image-style control reads them, and a dispatched `MouseEvent` still runs
    // an element's click default actions (link navigation, form submission,
    // checkbox toggle), so ordinary controls behave as if clicked directly.
    (focusTarget ?? element).focus({ preventScroll: true });
    element.dispatchEvent(
      new MouseEvent("click", {
        clientX: event.data.x,
        clientY: event.data.y,
        bubbles: true,
        cancelable: true,
      }),
    );
    return;
  }

  const pointerId = event.data.pointerId ?? 0;

  // The remainder of a press goes to the element that took the press, mirroring
  // pointer capture; a stale target (the page re-rendered mid-gesture) falls
  // back to the point, and pointerup/pointercancel close the press out either
  // way so a later gesture with the same id starts from `elementFromPoint`.
  const pressed = relayPressTargets.get(pointerId);
  if (event.data.type !== "oc:relay-pointerdown" && pressed?.isConnected) {
    dispatchRelayedPointer(event.data, pressed);
    if (
      event.data.type === "oc:relay-pointerup" ||
      event.data.type === "oc:relay-pointercancel"
    ) {
      if (event.data.type === "oc:relay-pointerup") {
        rememberPressTarget(pointerId, pressed);
      } else {
        lastPressTargets.delete(pointerId);
      }
      relayPressTargets.delete(pointerId);
    }
    return;
  }
  if (
    event.data.type === "oc:relay-pointerup" ||
    event.data.type === "oc:relay-pointercancel"
  ) {
    lastPressTargets.delete(pointerId);
    relayPressTargets.delete(pointerId);
  }

  const element = document.elementFromPoint(event.data.x, event.data.y);
  if (!(element instanceof HTMLElement || element instanceof SVGElement)) return;

  if (event.data.type === "oc:relay-pointerdown") {
    relayPressTargets.set(pointerId, element);
  }
  dispatchRelayedPointer(event.data, element);
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
