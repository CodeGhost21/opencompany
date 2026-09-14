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
  it("frames the page as a card inset from three edges, flush to the leading one", () => {
    const surface = render();
    const classes = surface.className.split(/\s+/);
    // The margins live on the FRAME, which is the card's parent: the orbiting
    // halo is a sibling positioned against the same rectangle, and the card
    // cannot host it — `overflow-hidden` is what keeps a page's scrolling
    // inside the rounded corners and it clips a pseudo-element just as readily.
    const frame = surface.parentElement!;
    const frameClasses = frame.className.split(/\s+/);

    // An even four-sided inset WAS the contract, when nothing sat above or
    // beside this card. Two things do now, and each takes an edge off:
    //
    //   - the LEADING edge is flush (`ml-0`). The sidebar's groups already
    //     carry their own 12px gutter, so an inset here put 12px of card margin
    //     against 12px of column padding — 24px between the last nav row and
    //     the first pixel of the page, against 12px on the other sides.
    //   - the TOP is thinner (`mt-0.5`). The window title row has its own
    //     bottom padding, so a full inset there stacked the two and read as a
    //     gap twice the size of the others.
    //
    // Stated as the exact set rather than "some margin exists", because the
    // failure this guards is a stray `ml-` creeping back and reopening the
    // double gutter — which looks like a design choice rather than a bug.
    expect(frameClasses).toContain("mr-(--frame-inset)");
    expect(frameClasses).toContain("mb-(--frame-inset)");
    expect(frameClasses).toContain("ml-0");
    expect(frameClasses).toContain("mt-0.5");
    expect(frameClasses).not.toContain("mx-(--frame-inset)");

    // The halo is a sibling of the card, inside the frame, and decorative.
    const halo = frame.querySelector('[aria-hidden="true"].content-orbit');
    expect(halo, "the frame draws no orbiting halo").not.toBeNull();

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
