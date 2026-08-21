import { describe, expect, it } from "vitest";

import type { WorkflowGraph } from "@/api/workflows";
import {
  centredFitX,
  contentBounds,
  layout,
  LEGIBLE_FIT_ZOOM,
  startAnchoredFit,
} from "@/views/workflows/graph";

/**
 * Issue #1361: a long pipeline must not open below the zoom at which its node
 * titles are still words, and once it is held at that floor it must open at its
 * START rather than its middle.
 *
 * The measurements these tests encode, taken in a real browser against
 * `feature_pipeline` (ten nodes, one per depth layer — the shape every shipped
 * company template has) before the fix:
 *
 *   1440px window → canvas pane 1224x782 → `fitView` zoom **0.353**
 *   1100px window → canvas pane  884x702 → `fitView` zoom **0.28**
 *
 * A node title is `text-sm` (14px), so 0.353 renders it at 4.9px. The console's
 * own type scale bottoms out at 10px (`text-3xs`), which is what fixes the
 * floor at 0.75 — see `LEGIBLE_FIT_ZOOM`.
 *
 * The arithmetic is tested rather than the canvas because it is arithmetic:
 * `FitGraphToPane` is thirty lines of wiring around these two functions, and a
 * jsdom canvas has no layout to measure.
 */

function graph(
  nodes: { id: string; kind: string; name: string }[],
  edges: { from: string; to: string }[],
): WorkflowGraph {
  return { id: "w", name: "W", version: "v1", nodes, edges } as unknown as WorkflowGraph;
}

/** A purely linear chain of `n` nodes — one per depth layer. */
function chain(n: number) {
  const nodes = Array.from({ length: n }, (_, i) => ({
    id: `n${i}`,
    kind: i === 0 ? "trigger" : "agent",
    name: `Step ${i}`,
  }));
  const edges = nodes.slice(1).map((node, i) => ({ from: `n${i}`, to: node.id }));
  return layout(graph(nodes, edges)).nodes;
}

/**
 * The canvas pane at a 1440px window: the app sidebar and the detail header
 * take the rest. Measured in a browser, not assumed.
 *
 * These numbers MOVE, and are expected to. The pane was 1224x782 when #1361 was
 * measured; #1297's inset card and the shell's content-surface work have since
 * taken it to 1198x756. That drift is the argument FOR a constant floor rather
 * than a computed one: `LEGIBLE_FIT_ZOOM` is derived from the type scale, not
 * from the pane, so every assertion below holds at both sizes and at any other
 * the shell arrives at. If one of these ever fails because the pane changed,
 * the pane got small enough to be its own bug — re-measure and say so here.
 */
const PANE_1440 = { width: 1198, height: 756 };
/** The same at 1100px, where the unclamped fit was worse (0.28). */
const PANE_1100 = { width: 858, height: 756 };

describe("the opening viewport for a workflow canvas", () => {
  it("holds a ten-node pipeline at the legible floor rather than fitting it", () => {
    const fit = startAnchoredFit(contentBounds(chain(10)), PANE_1440.width, PANE_1440.height);
    expect(fit).not.toBeNull();
    expect(fit!.zoom).toBe(LEGIBLE_FIT_ZOOM);
  });

  it("renders a 14px node title at no less than the console's 10px minimum", () => {
    // The whole derivation of the constant, as an assertion rather than a
    // comment: if someone lowers the floor to fit more nodes on screen, this is
    // what says what they spent.
    expect(14 * LEGIBLE_FIT_ZOOM).toBeGreaterThanOrEqual(10);
  });

  it("puts the first node at the canvas's left edge, not its middle", () => {
    const nodes = chain(10);
    const bounds = contentBounds(nodes);
    const fit = startAnchoredFit(bounds, PANE_1440.width, PANE_1440.height)!;

    // The graph's left edge, in screen pixels, under the returned viewport.
    const firstNodeX = fit.x + bounds.minX * fit.zoom;
    expect(firstNodeX).toBeGreaterThan(0);
    expect(firstNodeX).toBeLessThan(64);

    // And it is a real correction: React Flow's own centring would have put it
    // off the left edge entirely. This is the -256px measured in the browser.
    expect(centredFitX(bounds, PANE_1440.width, fit.zoom) + bounds.minX * fit.zoom)
      .toBeLessThan(0);
  });

  it("still leaves the graph running off to the right, where its arrows point", () => {
    const nodes = chain(10);
    const bounds = contentBounds(nodes);
    const fit = startAnchoredFit(bounds, PANE_1440.width, PANE_1440.height)!;
    // Not a claim that clipping is good — a claim that the trade is real and
    // deliberate: at a legible zoom this graph does not fit, and the operator
    // pans. If a future layout change made it fit, this test should fail and be
    // deleted rather than adjusted.
    expect(fit.x + (bounds.minX + bounds.width) * fit.zoom).toBeGreaterThan(
      PANE_1440.width,
    );
  });

  it("centres the graph vertically", () => {
    const nodes = chain(10);
    const bounds = contentBounds(nodes);
    const fit = startAnchoredFit(bounds, PANE_1440.width, PANE_1440.height)!;
    const top = fit.y + bounds.minY * fit.zoom;
    const bottom = top + bounds.height * fit.zoom;
    expect(top).toBeCloseTo(PANE_1440.height - bottom, 5);
  });

  it("leaves a short workflow entirely alone", () => {
    // Four nodes — `nightly_digest`. Its natural fit is already above the
    // floor, so React Flow's own fit is correct and must not be second-guessed.
    expect(
      startAnchoredFit(contentBounds(chain(4)), PANE_1440.width, PANE_1440.height),
    ).toBeNull();
  });

  it("corrects at 1100px too, where the unclamped fit was worse", () => {
    const nodes = chain(10);
    const bounds = contentBounds(nodes);
    const fit = startAnchoredFit(bounds, PANE_1100.width, PANE_1100.height)!;
    expect(fit.zoom).toBe(LEGIBLE_FIT_ZOOM);
    expect(fit.x + bounds.minX * fit.zoom).toBeGreaterThan(0);
  });

  it("declines to answer for an empty graph or an unmeasured pane", () => {
    expect(startAnchoredFit(contentBounds([]), 1224, 782)).toBeNull();
    expect(startAnchoredFit(contentBounds(chain(10)), 0, 0)).toBeNull();
    // A pane narrower than the gutters it would leave has no usable width; the
    // answer is "no opinion", never a negative-zoom viewport.
    expect(startAnchoredFit(contentBounds(chain(10)), 40, 40)).toBeNull();
  });
});

describe("recognising React Flow's own fit", () => {
  /**
   * The correction fires on "the viewport is at the floor AND placed exactly
   * where the fit would place it". The second half is load-bearing: React
   * Flow's zoom controls step by a fixed ratio, so an operator who zooms in
   * from a clamped fit and back out lands on exactly `LEGIBLE_FIT_ZOOM` — and a
   * canvas that jumped to the graph's start under their hands would be a worse
   * bug than the one being fixed.
   */
  it("reproduces the centred placement the fit produces", () => {
    const bounds = contentBounds(chain(10));
    const x = centredFitX(bounds, PANE_1440.width, LEGIBLE_FIT_ZOOM);
    const centreOnScreen = x + (bounds.minX + bounds.width / 2) * LEGIBLE_FIT_ZOOM;
    expect(centreOnScreen).toBeCloseTo(PANE_1440.width / 2, 5);
  });

  it("does not match the viewport the correction itself installs", () => {
    const bounds = contentBounds(chain(10));
    const corrected = startAnchoredFit(bounds, PANE_1440.width, PANE_1440.height)!;
    // Otherwise the correction would re-arm against its own result.
    expect(
      Math.abs(corrected.x - centredFitX(bounds, PANE_1440.width, corrected.zoom)),
    ).toBeGreaterThan(0.5);
  });
});
