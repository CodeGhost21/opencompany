/** Can this part of a toast handle its own click rather than passing it through? */
function isToastControl(target: Element): boolean {
  return target.closest(
    'a, button, input, select, textarea, [role="button"], [role="link"], [contenteditable="true"]',
  ) !== null;
}

/**
 * Relay a click from a toast's read-only surface to the page beneath it.
 *
 * The toast keeps receiving pointer movement, preserving sonner's hover-to-read
 * pause. Its close and action controls remain native click targets.
 */
export function relayToastClick(event: MouseEvent): void {
  if (event.button !== 0 || !(event.target instanceof Element)) return;
  if (!event.target.closest("[data-sonner-toast]") || isToastControl(event.target)) return;

  const toasterElements = Array.from(
    document.querySelectorAll<HTMLElement>("[data-sonner-toaster], [data-sonner-toaster] *"),
  );
  const pointerEvents = toasterElements.map((element) => element.style.pointerEvents);
  for (const element of toasterElements) element.style.pointerEvents = "none";
  const beneath = document.elementFromPoint(event.clientX, event.clientY);
  for (const [index, element] of toasterElements.entries()) {
    element.style.pointerEvents = pointerEvents[index];
  }

  // `elementFromPoint` can hand back any element, not just an `HTMLElement`:
  // the workflow minimap is an SVG sitting under the bottom-right toaster, and
  // an `SVGElement` must be as reachable as a button. `closest` and `click`
  // both live on `Element`, and `focus` is on `SVGElement` too, so requiring
  // the narrower type would leave every SVG-backed control blocked for nothing.
  if (!(beneath instanceof Element) || beneath.closest("[data-sonner-toaster]")) return;

  event.preventDefault();
  event.stopPropagation();
  beneath.focus({ preventScroll: true });
  beneath.click();
}
