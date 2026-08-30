// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ContentSurface } from "@/components/content-surface";

/**
 * The card half of the two-layer shell (issue #1178).
 *
 * Every page renders on this one card — there is no full-bleed escape hatch,
 * and the component's own docblock says why.
 *
 * These cases pin the contract: which classes the card carries, and that it
 * keeps the scroll container every view's `overflow-y-auto` depends on. They
 * cannot tell 12px from 1px; that is a fact about pixels, and
 * `test/e2e/shell-two-layer.spec.ts` measures it in a real browser. What they
 * catch is the frame, the radius or the edge quietly going missing, and
 * `min-h-0` being dropped — which moves every view's scroll to the window.
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render() {
  act(() =>
    root.render(
      createElement(ContentSurface, { children: createElement("p", null, "page content") }),
    ),
  );
  const surface = container.querySelector<HTMLElement>('[data-testid="content-surface"]');
  expect(surface).not.toBeNull();
  return surface!;
}

describe("ContentSurface", () => {
  it("frames the page as an inset, rounded card", () => {
    const surface = render();
    const classes = surface.className.split(/\s+/);

    // The inset is on all four sides, and it is ONE quantity — four numbers
    // that happen to agree is what drifts. A three-sided inset flush to one
    // edge is what the closed first attempt shipped, and it produced a sliver
    // rather than a frame.
    expect(classes).toContain("m-(--frame-inset)");
    // And nothing overrides one side of it: an even frame is the contract, so
    // a stray `mt-*`/`pt-*` reintroducing a special case fails here.
    expect(classes.filter((c) => /^m[trbl]-/.test(c))).toEqual([]);
    expect(classes).toContain("rounded-2xl");
    // The edge carries the chrome hairline, and the sheet is opaque: it is the
    // only opaque surface in the shell, so anything a page draws stacks on it.
    expect(classes).toContain("border-chrome-border");
    expect(classes).toContain("bg-background");
    expect(surface.dataset.unframed).toBeUndefined();
  });

  it("is the scroll container every view depends on", () => {
    // A view's own `overflow-y-auto` only scrolls because this box refuses to
    // grow past its share of the shell. Losing `min-h-0` moves the scroll to the
    // window, which is a whole-app regression rather than a styling one.
    const classes = render().className.split(/\s+/);
    expect(classes).toEqual(
      expect.arrayContaining(["flex", "min-h-0", "flex-1", "overflow-hidden"]),
    );
  });

  it("renders its children", () => {
    expect(render().textContent).toBe("page content");
  });
});
