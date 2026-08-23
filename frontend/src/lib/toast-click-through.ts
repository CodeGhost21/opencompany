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
function beneathAt(x: number, y: number): Element | null {
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
  beneath.focus({ preventScroll: true });
  // `HTMLElement.click()` also runs the element's default action (link
  // navigation, form submission); `SVGElement` has no `click()` in the DOM, so
  // the event is dispatched to reach its handlers instead.
  if (beneath instanceof HTMLElement) {
    beneath.click();
  } else {
    beneath.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  }
}
