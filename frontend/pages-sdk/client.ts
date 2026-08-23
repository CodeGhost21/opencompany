// The postMessage bridge to the parent console tab (docs/spec/runtime/pages.md
// §6, plan §6). A page never holds a credential of its own — every read or
// write it wants to run against the company's data goes through this bridge
// to the parent frame, which executes it with the operator's own
// authenticated session and posts the result back. Both queries and
// mutations travel the same way: GraphQL's own operation type is what
// distinguishes them, not this client.

const TIMEOUT_MS = 15_000;

// The per-document bridge capability handed to us by the console on load
// (`PagesView.tsx` mints a fresh one for every iframe document). Every
// `oc:graphql` message carries it, so the console can tell this exact
// document apart from one the page navigated itself to. Opaque-origin frames
// cannot share real storage or identity, so a document that replaces us has no
// way to learn this value.
let capability: string | null = null;
let capabilityWaiters: Array<(cap: string) => void> = [];

function waitForCapability(): Promise<string> {
  if (capability) return Promise.resolve(capability);
  return new Promise((resolve) => {
    capabilityWaiters.push(resolve);
  });
}

window.addEventListener("message", function onInit(event: MessageEvent) {
  const data = event.data as { type?: unknown; capability?: unknown } | null;
  if (data && data.type === "oc:init" && typeof data.capability === "string") {
    capability = data.capability;
    const waiters = capabilityWaiters;
    capabilityWaiters = [];
    for (const resolve of waiters) resolve(capability);
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

/** How far a relayed press may move before it counts as a drag rather than a click. */
const DRAG_THRESHOLD_PX = 10;

/** Where each relayed press started, keyed by pointer id, for the drag test. */
const pressOrigins = new Map<number, { x: number; y: number }>();

/** Every relayed press that has started so far, to tie a click to its own press. */
let relayedPressCount = 0;

/**
 * The point the most recent relayed press ended at, and whether it was a drag.
 *
 * The parent relays a compatibility click after a press's `pointerup`, but the
 * click message carries coordinates and no pointer id, so this document cannot
 * address the press it belongs to directly. A click always immediately follows
 * the `pointerup` of its own gesture, so the most recently ended press is the
 * one to correlate against — gated on no newer press having started. A click
 * that ends a drag must not land on whatever is under the release point: the
 * press's tail was routed to the element that took it (capture semantics), and
 * a drag produces no click.
 */
let lastPressEnd: { x: number; y: number; dragged: boolean; press: number } | null = null;

/** Close a relayed press out, recording whether it was a drag for the click test. */
function endRelayedPress(pointerId: number, x: number, y: number, canceled: boolean): void {
  const origin = pressOrigins.get(pointerId);
  pressOrigins.delete(pointerId);
  const dragged =
    canceled ||
    (origin !== undefined && Math.hypot(x - origin.x, y - origin.y) > DRAG_THRESHOLD_PX);
  lastPressEnd = { x, y, dragged, press: relayedPressCount };
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
    const element = document.elementFromPoint(event.data.x, event.data.y);
    if (!(element instanceof HTMLElement || element instanceof SVGElement)) return;
    // Mirror the parent relay's own handling (`toast-click-through.ts`):
    // `HTMLElement.click()` runs default actions, and an `SVGElement` has no
    // `click()` in the DOM, so the event is dispatched to reach its handlers.
    element.focus({ preventScroll: true });
    if (element instanceof HTMLElement) {
      element.click();
    } else {
      element.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    }
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
      relayPressTargets.delete(pointerId);
    }
    return;
  }
  if (
    event.data.type === "oc:relay-pointerup" ||
    event.data.type === "oc:relay-pointercancel"
  ) {
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

/**
 * Runs one GraphQL operation — query or mutation — against the console's own
 * GraphQL endpoint, by way of the parent frame.
 *
 * Internally: generates a random correlation `id`, posts
 * `{type: "oc:graphql", id, capability, query, variables}` to `window.parent`, and
 * resolves when a matching `{type: "oc:graphql:result", id, ...}` reply
 * arrives — a one-shot listener that removes itself either way. The `id` is
 * what lets several concurrent calls share the same `window` without their
 * replies crossing.
 *
 * `targetOrigin` is deliberately `"*"` on the outgoing post, not this
 * document's real parent origin — because this document cannot know it. It
 * runs inside a `sandbox="allow-scripts"` iframe with no `allow-same-origin`
 * (docs/spec/runtime/pages.md §5), so its own origin is the opaque string
 * `"null"` and there is no legitimate origin to address the parent by. That
 * is intentional, not a shortcut: the real trust boundary is enforced on the
 * *parent* side (`PagesView.tsx`'s message listener), which only ever acts
 * on an event whose `source` is this exact iframe's own `contentWindow` —
 * something a page itself has no way to spoof.
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
      window.removeEventListener("message", onMessage);
      reject(new Error("oc:graphql timed out waiting for a reply from the console"));
    }, TIMEOUT_MS);

    function onMessage(event: MessageEvent) {
      if (!isBridgeResult(event.data) || event.data.id !== id) return;
      window.clearTimeout(timeout);
      window.removeEventListener("message", onMessage);
      resolve({ data: event.data.data as T | undefined, errors: event.data.errors });
    }

    window.addEventListener("message", onMessage);
    waitForCapability().then((cap) => {
      window.parent.postMessage(
        { type: "oc:graphql", id, capability: cap, query: document, variables },
        "*",
      );
    });
  });
}

/** The one live-data surface a page has: `client.query(document, variables)`. */
export const client = { query };
