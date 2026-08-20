// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ContentSurface } from "@/components/content-surface";
import { isFullBleed } from "@/lib/shell-frame";

/**
 * The card half of the two-layer shell (issue #1178).
 *
 * Two properties, and they are covered at different levels on purpose:
 *
 *   1. **Framed and unframed differ, and in the right direction.** The cases
 *      below pin the contract — which classes each mode carries, and that both
 *      keep the same scroll container. They cannot tell 12px from 1px; that is
 *      a fact about pixels, and `test/e2e/shell-two-layer.spec.ts` measures it
 *      in a real browser. What they do catch is the frame, the radius or the
 *      edge quietly going missing, and `min-h-0` being dropped — which moves
 *      every view's scroll to the window.
 *   2. **The right surfaces get each mode.** `isFullBleed` is asked of the
 *      whole address, not the view, because Workflows is a document at
 *      `#/workflows` and a canvas at `#/workflows/<id>`. Getting that wrong
 *      crops the React Flow canvas, which is exactly what #1259 and #1261 were
 *      filed for. That one IS a property, and it is tested as one.
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

function render(props: { unframed?: boolean }) {
  act(() =>
    root.render(
      createElement(ContentSurface, { ...props, children: createElement("p", null, "page content") }),
    ),
  );
  const surface = container.querySelector<HTMLElement>('[data-testid="content-surface"]');
  expect(surface).not.toBeNull();
  return surface!;
}

describe("ContentSurface", () => {
  it("frames the page as an inset, rounded card by default", () => {
    const surface = render({});
    const classes = surface.className.split(/\s+/);

    // The inset is on all four sides. A three-sided inset flush to one edge is
    // what the closed first attempt shipped, and it produced a sliver rather
    // than a frame.
    expect(classes).toContain("m-3");
    expect(classes).toContain("rounded-2xl");
    // The edge carries the chrome hairline, and the sheet is opaque: it is the
    // only opaque surface in the shell, so anything a page draws stacks on it.
    expect(classes).toContain("border-chrome-border");
    expect(classes).toContain("bg-background");
    expect(surface.dataset.unframed).toBeUndefined();
  });

  it("renders edge to edge when unframed, with no margin, radius or edge", () => {
    const surface = render({ unframed: true });
    const classes = surface.className.split(/\s+/);

    expect(classes).not.toContain("m-3");
    expect(classes).not.toContain("rounded-2xl");
    expect(classes).not.toContain("border-chrome-border");
    // Still the opaque sheet — full bleed changes the shape, not the fill.
    expect(classes).toContain("bg-background");
    // The hook an e2e spec keys the framed/unframed distinction off.
    expect(surface.dataset.unframed).toBe("true");
  });

  it("keeps the same scroll container in both modes", () => {
    // A view's own `overflow-y-auto` only scrolls because this box refuses to
    // grow past its share of the shell. Losing `min-h-0` moves the scroll to the
    // window, which is a whole-app regression rather than a styling one.
    for (const unframed of [false, true]) {
      const classes = render({ unframed }).className.split(/\s+/);
      expect(classes).toEqual(expect.arrayContaining(["flex", "min-h-0", "flex-1", "overflow-hidden"]));
    }
  });

  it("renders its children", () => {
    expect(render({}).textContent).toBe("page content");
  });
});

describe("isFullBleed", () => {
  it("gives the knowledge graph the whole pane, whatever the sub-page", () => {
    expect(isFullBleed("overview", null)).toBe(true);
    expect(isFullBleed("overview", "anything")).toBe(true);
  });

  it("frames the workflow browse list and unframes the canvas", () => {
    // `#/workflows` is a list of cards — a document, and it looked like a page
    // with its edges missing when it was lumped in with the canvas.
    expect(isFullBleed("workflows", null)).toBe(false);
    // `#/workflows/<id>` is React Flow, whose viewport and minimap are computed
    // from the container's measured rect.
    expect(isFullBleed("workflows", "feature_pipeline")).toBe(true);
  });

  it("frames every other view, including ones that carry a sub-page", () => {
    for (const view of ["ledgers", "chat", "settings", "company", "workspace"] as const) {
      expect(isFullBleed(view, null)).toBe(false);
      expect(isFullBleed(view, "sub")).toBe(false);
    }
  });
});
