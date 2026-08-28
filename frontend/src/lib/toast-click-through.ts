/**
 * Pass clicks on a toast's read-only surface through to the page beneath it.
 *
 * A toast sits in a fixed corner of the viewport, which is where actionable
 * controls end up underneath it (issue #1303). Sonner pauses a toast's
 * dismissal timer while it is hovered — good for reading, but it turns the
 * toast into a trap when the thing the operator is reaching for sits below:
 * moving toward the control keeps the toast alive, and every click lands on
 * the toast. The relays in this module make the toast transparent to the
 * gesture without taking away that hover-to-read pause.
 *
 * `relayToastPointerDown` hands the press itself to the element beneath, so
 * pointer-driven controls react exactly as if the toast were not there — the
 * workflow minimap, itself an SVG under the bottom-right toaster, pans on
 * `pointerdown` rather than `click`. `relayToastClick` covers the click event
 * for anything that did not take the pointer path. A toast's own controls —
 * its close button and any action button — are excluded by `isToastControl`,
 * so a notification can still offer a one-click recovery without eating
 * nearby page controls.
 *
 * A control rendered inside the console's Pages view is a different story:
 * it lives in another *document* (a sandboxed `allow-scripts` iframe), and
 * events dispatched here cannot cross the browsing-context boundary into it.
 * `elementFromPoint` answers with the frame host, and clicking the host does
 * nothing to what is mounted inside — so for a frame the whole gesture is
 * handed to the frame itself over the existing postMessage bridge, and the
 * page SDK turns it back into events on the element beneath the point (see
 * `pages-sdk/client.ts`). Pointer capture cannot reach into another document,
 * so a press relayed into a frame keeps relaying its tail — the
 * `pointermove`, `pointerup` and `pointercancel` that land back here — until
 * the press ends, which is what lets a frame-side drag or press-state control
 * complete the gesture instead of getting stuck.
 */

/** Can this part of a toast handle its own pointer rather than passing it through? */
function isToastControl(target: Element): boolean {
  return target.closest(
    'a, button, input, select, textarea, [role="button"], [role="link"], [contenteditable="true"]',
  ) !== null;
}

/**
 * The page element under a pointer position, ignoring the toaster itself.
 *
 * `elementFromPoint` answers with the topmost element at the point, which is
 * the toast the pointer is over — so the toaster's hit-testing is suspended
 * for the instant of the read. Nothing is left disabled: the original
 * pointer-events values are restored before returning.
 */
function beneathAt(x: number, y: number): (HTMLElement | SVGElement) | null {
  const toasterElements = Array.from(
    document.querySelectorAll<HTMLElement>("[data-sonner-toaster], [data-sonner-toaster] *"),
  );
  const pointerEvents = toasterElements.map((element) => element.style.pointerEvents);
  for (const element of toasterElements) element.style.pointerEvents = "none";
  const beneath = document.elementFromPoint(x, y);
  for (const [index, element] of toasterElements.entries()) {
    element.style.pointerEvents = pointerEvents[index];
  }

  // `elementFromPoint` can hand back any element, not just an `HTMLElement`:
  // the workflow minimap is an SVG sitting under the bottom-right toaster, and
  // an `SVGElement` must be as reachable as a button. Both types carry
  // `closest`, `focus` and pointer capture, so requiring the narrower one
  // would leave every SVG-backed control blocked for nothing.
  if (!(beneath instanceof HTMLElement || beneath instanceof SVGElement)) return null;
  if (beneath.closest("[data-sonner-toaster]")) return null;
  return beneath;
}

/** A gesture handed to a page frame over the postMessage bridge. */
type FrameRelayType =
  | "oc:relay-click"
  | "oc:relay-pointerdown"
  | "oc:relay-pointermove"
  | "oc:relay-pointerup"
  | "oc:relay-pointercancel";

/**
 * Hand a gesture over a page frame to the frame's own document.
 *
 * For a point over an embedded document, `elementFromPoint` answers with the
 * frame host, and a `click()` or `dispatchEvent` on it stays in this document
 * — it can neither reach the frame's content nor, for the console's sandboxed
 * Pages view, be legal (the two documents are not same-origin). The one
 * channel that does cross the boundary is `postMessage`: this posts the
 * gesture's coordinates, frame-relative, and the page SDK's own listener
 * (`pages-sdk/client.ts`) turns them into a real pointer gesture on whatever
 * element is beneath the point *inside* the frame. Coordinates are shifted by
 * the frame's viewport offset so the embedded document sees the same point
 * the parent hit-test would have.
 */
function relayToFrame(
  frame: HTMLIFrameElement,
  gesture: PointerEvent | MouseEvent,
  type: FrameRelayType,
): void {
  const rect = frame.getBoundingClientRect();
  const payload: Record<string, unknown> = {
    type,
    x: gesture.clientX - rect.left,
    y: gesture.clientY - rect.top,
  };
  if (type !== "oc:relay-click") {
    const pointer = gesture as PointerEvent;
    payload.pointerId = pointer.pointerId;
    payload.pointerType = pointer.pointerType;
    payload.isPrimary = pointer.isPrimary;
    payload.button = pointer.button;
    payload.buttons = pointer.buttons;
    payload.detail = pointer.detail;
  } else if ("pointerId" in gesture && typeof gesture.pointerId === "number") {
    // Chromium and other Pointer Events implementations expose the originating
    // pointer id on the compatibility click. Keeping it lets the frame side
    // correlate the click with its press and preserve the captured target
    // without confusing an unrelated direct click.
    payload.pointerId = gesture.pointerId;
  }
  frame.contentWindow?.postMessage(payload, "*");
}

/**
 * The presses this document has handed into a page frame, keyed by pointer id.
 *
 * Pointer capture is what completes a gesture for a control in the same
 * document, but a frame is another document and the browser's capture cannot
 * reach into it: the `pointermove`, `pointerup` and `pointercancel` of a press
 * that started on a toast over a frame land back in *this* document and would
 * otherwise be dropped, leaving a frame-side drag or press-state control
 * stuck after `pointerdown`. This tracks which frame owns the rest of each
 * press until it ends (`relayFramePressTail` posts the remainder over the same
 * bridge, mirroring the capture the same-document path uses).
 */
const framePresses = new Map<number, HTMLIFrameElement>();

/**
 * Forward the tail of a press that was relayed into a page frame.
 *
 * Once a press has been handed to a frame, every following event of that
 * pointer belongs to the same gesture and is posted to the same frame,
 * shifted into its viewport, until the press ends — at which point the
 * tracked gesture is closed out. A frame that left the document mid-gesture
 * has nothing left to deliver to, so its tail is dropped.
 */
function relayFramePressTail(event: PointerEvent): void {
  const frame = framePresses.get(event.pointerId);
  if (!frame) return;
  if (!frame.isConnected) {
    framePresses.delete(event.pointerId);
    return;
  }
  if (event.type === "pointerup" || event.type === "pointercancel") {
    framePresses.delete(event.pointerId);
  }
  // The listener is registered only for the three pointer-tail events, so the
  // relay type is always `oc:relay-pointermove`/`up`/`cancel` here.
  relayToFrame(frame, event, `oc:relay-${event.type}` as FrameRelayType);
}

window.addEventListener("pointermove", relayFramePressTail, true);
window.addEventListener("pointerup", relayFramePressTail, true);
window.addEventListener("pointercancel", relayFramePressTail, true);

/**
 * Relay the pointerdown of a press on a toast's read-only surface to the page
 * beneath it.
 *
 * A synthetic `pointerdown` alone would not be enough for pointer-driven
 * controls: they react to the gesture, not just the event. So the real
 * pointer is captured to the element beneath, which makes the browser deliver
 * the remainder of the press — `pointermove`, `pointerup` — to it as if the
 * toast were never there, and the synthetic `pointerdown` carries the original
 * coordinates and the live pointer id so its handler runs with the same
 * gesture it would have seen.
 *
 * Click-driven controls need no special handling here: their `click` still
 * reaches them through `relayToastClick` (or, in browsers whose compatibility
 * mouse events follow the pointer capture, directly).
 */
export function relayToastPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || !(event.target instanceof Element)) return;
  if (!event.target.closest("[data-sonner-toast]") || isToastControl(event.target)) return;

  const beneath = beneathAt(event.clientX, event.clientY);
  if (!beneath) return;

  // A page frame is another document: pointer capture here cannot reach its
  // content, so the press is handed to the frame itself over the bridge, and
  // the rest of the gesture is relayed to it as it lands back here
  // (`relayFramePressTail`). A frame with no loaded document receives the
  // press but cannot hold a gesture, so nothing is tracked for it.
  if (beneath instanceof HTMLIFrameElement) {
    relayToFrame(beneath, event, "oc:relay-pointerdown");
    if (beneath.contentWindow) framePresses.set(event.pointerId, beneath);
    return;
  }

  // Best-effort: an element that declines capture still receives the
  // synthetic pointerdown; only the drag tail of the gesture is lost. jsdom
  // does not implement pointer capture and throws here.
  try {
    beneath.setPointerCapture(event.pointerId);
  } catch {
    /* capture routes the rest of the gesture; the press itself is enough */
  }

  beneath.dispatchEvent(
    new PointerEvent("pointerdown", {
      clientX: event.clientX,
      clientY: event.clientY,
      pointerId: event.pointerId,
      pointerType: event.pointerType,
      isPrimary: event.isPrimary,
      detail: event.detail,
      button: 0,
      buttons: event.buttons,
      bubbles: true,
      cancelable: true,
    }),
  );
}

/**
 * Relay a click on a toast's read-only surface to the page beneath it.
 *
 * The fallback to the pointer relay above: it covers the click event itself,
 * which is the whole gesture for click-driven controls, and is what a
 * keyboard-initiated click on a focused toast reaches the page with.
 */
export function relayToastClick(event: MouseEvent): void {
  if (event.button !== 0 || !(event.target instanceof Element)) return;
  if (!event.target.closest("[data-sonner-toast]") || isToastControl(event.target)) return;

  const beneath = beneathAt(event.clientX, event.clientY);
  if (!beneath) return;

  event.preventDefault();
  event.stopPropagation();

  // A page frame is another document: `click()` on the host element does
  // nothing to what is rendered inside, and focusing it would steal keyboard
  // focus into the frame. Hand the click to the frame over the bridge; its
  // page SDK dispatches it on the element under the point.
  if (beneath instanceof HTMLIFrameElement) {
    relayToFrame(beneath, event, "oc:relay-click");
    return;
  }

  // Focus the control the click activates, not the leaf the point landed on:
  // `elementFromPoint` hands back the icon `<path>` or `<span>` inside a
  // button, and `focus()` on that leaf is a no-op — the button is clicked but
  // keyboard focus is left behind, where a later Enter/Space operates the
  // wrong element. A native click focuses the nearest focusable ancestor, so
  // the relay does the same.
  const focusTarget = beneath.closest(
    'button, a[href], input, select, textarea, [contenteditable="true"], [tabindex]',
  ) as HTMLElement | SVGElement | null;
  (focusTarget ?? beneath).focus({ preventScroll: true });
  // `HTMLElement.click()` also runs the element's default action (link
  // navigation, form submission); `SVGElement` has no `click()` in the DOM, so
  // the event is dispatched to reach its handlers instead.
  if (beneath instanceof HTMLElement) {
    beneath.click();
  } else {
    beneath.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  }
}
