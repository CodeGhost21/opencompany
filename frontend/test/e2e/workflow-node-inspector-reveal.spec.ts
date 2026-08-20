import { expect, test, type Locator, type Page } from "@playwright/test";

import { openWorkflow } from "./workflows";

/**
 * Issue #1231: the node inspector must not open on top of the node it describes.
 *
 * Pinned as geometry in a real browser, because geometry is the whole bug and
 * nothing short of a layout engine measures it. The panel is the canvas's right
 * overlay (`absolute right-3 top-3 bottom-3`, `w-72 sm:w-80`), so it owns a
 * fixed strip of the canvas's right edge; the canvas used to have no idea that
 * strip existed — `fitViewOptions={{ padding: 0.2 }}` is symmetric and runs
 * once, at load — so whatever occupied it stayed occupied, ghosting through the
 * panel's translucent surface.
 *
 * Each case asserts BOTH halves, and the first half is the one that keeps this
 * spec honest: the clicked node has to have been under the strip before the
 * click, or "it is visible afterwards" proves nothing at all.
 *
 * The unit test (`workflow-node-reveal.test.ts`) pins the arithmetic. This pins
 * that the arithmetic is wired to the canvas an operator actually clicks.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

/** The source-defined fixture — three nodes in a left-to-right chain, with ids
 * and a name the suite already pins (`companies/e2e_harness/workflows/committed.toml`).
 * Named rather than "whichever workflow is first" because this spec depends on
 * the graph's shape: it needs a node far enough right to be covered. */
const FIXTURE = "Committed flow";

/** The inspector's own numbers, from `node-reveal.ts` — `sm:w-80` (320) plus
 * `right-3` (12). The gap is not included: the assertion is "not covered", and
 * demanding the daylight too would pin a comfort constant from a browser. */
const PANEL_STRIP = 320 + 12;

/** Dismisses the first-run tour if it is up; its overlay swallows pointer
 * events. Tolerates its absence. */
async function dismissTour(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  try {
    await skip.waitFor({ state: "visible", timeout: 10_000 });
  } catch {
    return;
  }
  await skip.click();
  await expect(skip).toBeHidden();
}

/** The canvas ReactFlow paints into — the box the overlay is positioned against. */
function canvas(page: Page) {
  return page.locator(".react-flow").first();
}

function inspector(page: Page) {
  return page.getByTestId("workflow-node-detail");
}

async function box(locator: Locator) {
  const b = await locator.boundingBox();
  expect(b, "element has no box").not.toBeNull();
  return b!;
}

/** The rightmost node on the canvas — the one the overlay lands on. */
async function rightmostNode(page: Page) {
  const nodes = page.locator(".react-flow__node");
  await expect(nodes.first()).toBeVisible({ timeout: 30_000 });
  const count = await nodes.count();
  let best: { node: Locator; right: number } | null = null;
  for (let i = 0; i < count; i++) {
    const node = nodes.nth(i);
    const b = await box(node);
    const right = b.x + b.width;
    if (!best || right > best.right) best = { node, right };
  }
  expect(best, "the canvas rendered no nodes").not.toBeNull();
  return best!.node;
}

/**
 * Opens the fixture and returns the rightmost node together with the strip the
 * inspector is about to occupy, asserting first that the two overlap.
 *
 * That assertion is the regression itself, stated forwards: on this graph, at
 * this width, the node an operator clicks IS under the panel. Without it the
 * rest of the case could pass on a graph that never reproduced the bug.
 */
async function setup(page: Page, width: number, height = 900) {
  await page.setViewportSize({ width, height });
  await page.goto("/#/workflows");
  await dismissTour(page);
  await openWorkflow(page, FIXTURE);

  const flow = canvas(page);
  await expect(flow).toBeVisible({ timeout: 30_000 });
  const node = await rightmostNode(page);

  const graph = await box(flow);
  const before = await box(node);
  const stripLeft = graph.x + graph.width - PANEL_STRIP;
  expect(
    before.x + before.width,
    `the fixture must put a node under the inspector's strip at ${width}px, ` +
      "or this case proves nothing",
  ).toBeGreaterThan(stripLeft);

  return { flow, node, before };
}

for (const width of [1440, 1024]) {
  test(`at ${width}px the inspector pans its node out from under itself`, async ({
    page,
  }) => {
    const { node, before } = await setup(page, width);

    await node.click();
    const panel = inspector(page);
    await expect(panel).toBeVisible();

    // The canvas animates, so measure once it has settled rather than racing it.
    await expect(async () => {
      const shown = await box(node);
      const overlay = await box(panel);
      expect(
        shown.x + shown.width,
        "the inspected node must clear the panel that describes it",
      ).toBeLessThanOrEqual(overlay.x + 1);
      // Cleared by moving left, not by falling off the other side.
      expect(shown.x, "the node must stay on the canvas").toBeGreaterThan(0);
    }).toPass({ timeout: 5_000 });

    // The pan is a pan: the node moved left and kept its size, so nothing was
    // rescaled underneath the operator (#1261's `minZoom` floor is untouched).
    const shown = await box(node);
    expect(shown.x).toBeLessThan(before.x);
    expect(Math.abs(shown.width - before.width)).toBeLessThan(2);
    expect(Math.abs(shown.y - before.y)).toBeLessThan(2);
  });
}

test("closing the inspector puts the canvas back where it was", async ({ page }) => {
  // `CanvasShell`'s convention for this slot promises the canvas is restored
  // instantly on close. Panning on open would quietly break that promise, so
  // the undo is part of the fix rather than a nicety.
  //
  // Note that this closes the panel as soon as the canvas has visibly started
  // moving, which lands MID-ANIMATION — and that is the case worth having.
  // React Flow animates a viewport change through d3-zoom's smooth
  // interpolation, which arcs through a lower zoom than either end, so a
  // restore that decided from the live viewport read a zoom matching neither
  // and gave up. See `RevealSelectedNode`.
  const { node, before } = await setup(page, 1440);

  await node.click();
  await expect(inspector(page)).toBeVisible();
  await expect(async () => {
    expect((await box(node)).x).toBeLessThan(before.x - 1);
  }).toPass({ timeout: 5_000 });

  await page.getByTestId("workflow-node-detail").getByRole("button", { name: "Close" }).click();
  await expect(inspector(page)).toBeHidden();

  await expect(async () => {
    const back = await box(node);
    expect(Math.abs(back.x - before.x), "the canvas returns to where it was").toBeLessThan(2);
    expect(Math.abs(back.y - before.y)).toBeLessThan(2);
  }).toPass({ timeout: 5_000 });
});

test("a node nowhere near the panel does not move the canvas at all", async ({
  page,
}) => {
  // The other half of "just far enough": most clicks land on a node that is
  // already visible, and a pan on every one of them would be its own bug.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/workflows");
  await dismissTour(page);
  await openWorkflow(page, FIXTURE);

  const flow = canvas(page);
  await expect(flow).toBeVisible({ timeout: 30_000 });
  const first = page.locator(".react-flow__node").first();
  await expect(first).toBeVisible({ timeout: 30_000 });

  const graph = await box(flow);
  const before = await box(first);
  expect(
    before.x + before.width,
    "the leftmost node must start clear of the strip for this case to mean anything",
  ).toBeLessThan(graph.x + graph.width - PANEL_STRIP);

  await first.click();
  await expect(inspector(page)).toBeVisible();
  // Given a beat to move, if it were going to.
  await expect(async () => {
    const after = await box(first);
    expect(Math.abs(after.x - before.x), "the canvas must not twitch").toBeLessThan(2);
  }).toPass({ timeout: 2_000 });
});
