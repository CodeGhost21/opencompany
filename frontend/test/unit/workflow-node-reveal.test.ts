import { describe, expect, it } from "vitest";

import {
  INSPECTOR_RESERVED,
  revealViewport,
  sameViewport,
  shouldRestore,
  type Viewport,
} from "@/views/workflows/node-reveal";
import { NODE_H, NODE_W } from "@/views/workflows/graph";

/**
 * Issue #1231: the node inspector opened on top of the node it describes.
 *
 * Asserted as arithmetic on two rectangles, because that is what was wrong. The
 * panel is `absolute right-3 top-3 bottom-3 w-72 sm:w-80`, so it owns a fixed
 * strip of the canvas's right edge; nothing in the view knew that strip existed
 * (`fitViewOptions={{ padding: 0.2 }}` is symmetric and runs once, at load), so
 * whatever occupied it stayed occupied.
 *
 * The measurements in the issue are the fixture below: `customer-escalation` at
 * 1440px, panel at x=1108..1428, the last node at x=1159..1338 — a node
 * entirely inside the panel's footprint.
 */

/** The canvas region's own width in the issue's repro: the panel's right edge
 * (1428) plus its 12px inset, with the canvas starting at the app's sidebar. */
const PANE_WIDTH = 1440 - 200;

const RESTING: Viewport = { x: 0, y: 0, zoom: 1 };

function node(x: number, y = 0) {
  return { x, y, width: NODE_W, height: NODE_H };
}

describe("revealViewport", () => {
  it("leaves the canvas alone when the node is already clear of the panel", () => {
    // Comfortably left of `paneWidth - INSPECTOR_RESERVED`. The common click is
    // still a click: a pan on every selection would be its own bug.
    const clear = node(100);
    expect(
      revealViewport({ node: clear, viewport: RESTING, paneWidth: PANE_WIDTH }),
    ).toBeNull();
  });

  it("leaves it alone when the node's right edge lands exactly on the limit", () => {
    const limit = PANE_WIDTH - INSPECTOR_RESERVED;
    const flush = node(limit - NODE_W);
    expect(
      revealViewport({ node: flush, viewport: RESTING, paneWidth: PANE_WIDTH }),
    ).toBeNull();
  });

  it("pans exactly far enough to clear a node under the panel, and no further", () => {
    // The issue's own case: the last node of the chain, under the overlay.
    const covered = node(1159 - 200);
    const next = revealViewport({
      node: covered,
      viewport: RESTING,
      paneWidth: PANE_WIDTH,
    })!;
    expect(next).not.toBeNull();

    const right = (covered.x + covered.width) * next.zoom + next.x;
    // Cleared…
    expect(right).toBeLessThanOrEqual(PANE_WIDTH - INSPECTOR_RESERVED + 0.001);
    // …and cleared by exactly the reserved strip, not by an arbitrary jump.
    expect(right).toBeCloseTo(PANE_WIDTH - INSPECTOR_RESERVED, 6);
  });

  it("pans horizontally only, and never rescales the graph", () => {
    // Composing with #1261's `minZoom={0.1}` and with whatever the operator has
    // zoomed to is the whole reason this is a pan: the overlay spans the full
    // height of the canvas, so no vertical move helps, and a silent rescale
    // would be a second surprise on top of the one being fixed.
    const viewport: Viewport = { x: -40, y: 33, zoom: 0.42 };
    const next = revealViewport({
      node: node(4000, 900),
      viewport,
      paneWidth: PANE_WIDTH,
    })!;
    expect(next.zoom).toBe(viewport.zoom);
    expect(next.y).toBe(viewport.y);
    expect(next.x).toBeLessThan(viewport.x);
  });

  it("honours the zoom when deciding how far to pan", () => {
    // Half scale, so a node twice as far out in graph space needs the same
    // screen-space shift. The target is a function of graph position and zoom.
    const zoomed: Viewport = { x: 0, y: 0, zoom: 0.5 };
    const next = revealViewport({
      node: node(2400),
      viewport: zoomed,
      paneWidth: PANE_WIDTH,
    })!;
    expect((2400 + NODE_W) * 0.5 + next.x).toBeCloseTo(
      PANE_WIDTH - INSPECTOR_RESERVED,
      6,
    );
  });

  it("keeps the node's left edge on screen when it cannot fully clear the panel", () => {
    // Only reachable at a zoom where one node fills most of the canvas. Half a
    // visible node beats a node shoved off the other side.
    const huge = { x: 0, y: 0, width: NODE_W, height: NODE_H };
    const zoomed: Viewport = { x: 500, y: 0, zoom: 8 };
    const next = revealViewport({
      node: huge,
      viewport: zoomed,
      paneWidth: 600,
    })!;
    expect(next.x).toBeGreaterThanOrEqual(12 - 0.001);
    expect(next.x).toBeLessThan(zoomed.x);
  });

  it("refuses to act on a canvas that has not been measured yet", () => {
    // React Flow reports width 0 before the pane is laid out; panning off that
    // would throw the viewport somewhere arbitrary on the first paint.
    expect(
      revealViewport({ node: node(1200), viewport: RESTING, paneWidth: 0 }),
    ).toBeNull();
    expect(
      revealViewport({
        node: node(1200),
        viewport: { x: 0, y: 0, zoom: 0 },
        paneWidth: PANE_WIDTH,
      }),
    ).toBeNull();
  });

  it("reserves the panel's width plus its inset and a gap", () => {
    // `sm:w-80` (320) + `right-3` (12) + daylight (12). Pinned because the
    // number is the contract between this module and `NodeDetailPanel`'s
    // classes — if those change, this has to.
    expect(INSPECTOR_RESERVED).toBe(344);
  });
});

describe("shouldRestore", () => {
  const from: Viewport = { x: 0, y: 0, zoom: 1 };
  const to: Viewport = { x: -300, y: 0, zoom: 1 };

  it("restores when the canvas is still where the reveal put it", () => {
    expect(shouldRestore(to, from, to)).toBe(true);
  });

  it("restores from anywhere along the reveal's own pan", () => {
    // The reveal is animated, so closing the panel a moment after opening it
    // lands mid-flight. Exact equality would strand the canvas half way.
    expect(shouldRestore({ x: -140, y: 0, zoom: 1 }, from, to)).toBe(true);
  });

  it("leaves a view the operator panned themselves alone", () => {
    expect(shouldRestore({ x: -900, y: 0, zoom: 1 }, from, to)).toBe(false);
    expect(shouldRestore({ x: 400, y: 0, zoom: 1 }, from, to)).toBe(false);
    expect(shouldRestore({ x: -300, y: -220, zoom: 1 }, from, to)).toBe(false);
  });

  it("leaves a view the operator zoomed alone", () => {
    // Their zoom is theirs — including one reached with #1261's lower floor.
    expect(shouldRestore({ x: -300, y: 0, zoom: 0.1 }, from, to)).toBe(false);
  });
});

describe("sameViewport", () => {
  it("tolerates sub-pixel drift but not a real move", () => {
    expect(sameViewport({ x: 10, y: 4, zoom: 1 }, { x: 10.4, y: 4, zoom: 1 })).toBe(
      true,
    );
    expect(sameViewport({ x: 10, y: 4, zoom: 1 }, { x: 24, y: 4, zoom: 1 })).toBe(
      false,
    );
    expect(sameViewport({ x: 10, y: 4, zoom: 1 }, { x: 10, y: 4, zoom: 1.2 })).toBe(
      false,
    );
  });
});
