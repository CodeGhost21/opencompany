import { expect, test, type Locator, type Page } from "@playwright/test";

import { openWorkflow } from "./workflows";

/**
 * Issue #1361: a long pipeline must open readable, and at its start.
 *
 * `workflow-canvas-fit.test.ts` pins the arithmetic. This pins that the
 * arithmetic reaches the canvas an operator actually looks at — which is the
 * half that took three attempts, because the correction is a CHILD of
 * `<ReactFlow>` and had to be sequenced against the parent's own `fitView`. A
 * unit test could not have caught either failed attempt: both produced the
 * right numbers and the wrong canvas.
 *
 * The measurements this replaces, taken by hand against `feature_pipeline`
 * before the fix — zoom **0.353** at 1440px and **0.28** at 1100px, first node
 * at x = -256 and last at x = 1769, both off screen, with a 14px node title
 * rendering at 5px.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

/** The harness's ten-node fixture (`companies/e2e_harness/workflows/long_pipeline.toml`).
 * Named rather than "whichever workflow is longest" because the whole spec
 * depends on the graph's shape: it has to be too long to fit legibly. */
const LONG = "Long pipeline";

/** The harness's three-node fixture, which fits whole and must stay untouched
 * — the control case for "this only applies where it is needed". */
const SHORT = "Committed flow";

/** `LEGIBLE_FIT_ZOOM` in `views/workflows/graph.ts`. Duplicated rather than
 * imported: this spec is the outside view, and a constant that changed on both
 * sides at once would assert nothing. */
const LEGIBLE_FIT_ZOOM = 0.75;

/** The console's smallest type size (`--text-3xs`, `src/index.css`) and a node
 * title's own size (`text-sm`) — the two numbers the floor is derived from. */
const MIN_TYPE_PX = 10;
const NODE_TITLE_PX = 14;

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

function canvas(page: Page) {
  return page.locator(".react-flow").first();
}

async function box(locator: Locator) {
  const b = await locator.boundingBox();
  expect(b, "element has no box").not.toBeNull();
  return b!;
}

/** The canvas's current zoom, read off the transform React Flow writes. */
async function zoomOf(page: Page): Promise<number> {
  const transform = await page
    .locator(".react-flow__viewport")
    .first()
    .evaluate((el) => (el as HTMLElement).style.transform);
  const match = /scale\(([\d.]+)\)/.exec(transform);
  expect(match, `no scale in transform ${JSON.stringify(transform)}`).not.toBeNull();
  return Number(match![1]);
}

async function open(page: Page, name: string, width: number, height = 900) {
  await page.setViewportSize({ width, height });
  await page.goto("/#/workflows");
  await dismissTour(page);
  await openWorkflow(page, name);
  await expect(canvas(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(".react-flow__node").first()).toBeVisible({
    timeout: 30_000,
  });
}

for (const width of [1440, 1100]) {
  test(`at ${width}px a ten-node pipeline opens legible and at its start`, async ({
    page,
  }) => {
    await open(page, LONG, width);

    // Settled, not raced: React Flow fits, and the correction lands on the
    // frame after.
    await expect(async () => {
      expect(await zoomOf(page)).toBe(LEGIBLE_FIT_ZOOM);
    }).toPass({ timeout: 10_000 });

    // The claim the floor is for, stated as the thing an operator cares about.
    expect(NODE_TITLE_PX * (await zoomOf(page))).toBeGreaterThanOrEqual(MIN_TYPE_PX);

    // The graph starts on screen, near the canvas's left edge — not centred
    // with both ends cut off, which is what the unclamped fit did.
    const flow = await box(canvas(page));
    const first = await box(page.locator(".react-flow__node").first());
    expect(
      first.x,
      "the first node must be on the canvas, not off its left edge",
    ).toBeGreaterThanOrEqual(flow.x);
    expect(
      first.x - flow.x,
      "the first node must be AT the start, not merely somewhere on screen",
    ).toBeLessThan(96);

    // And the graph genuinely does not fit at this zoom — otherwise the two
    // assertions above would hold for a fit that never needed correcting, and
    // this spec would be measuring nothing.
    const nodes = page.locator(".react-flow__node");
    const last = await box(nodes.nth((await nodes.count()) - 1));
    expect(last.x + last.width).toBeGreaterThan(flow.x + flow.width);
  });
}

test("a short workflow still fits whole, untouched", async ({ page }) => {
  await open(page, SHORT, 1440);

  const flow = await box(canvas(page));
  const nodes = page.locator(".react-flow__node");
  const count = await nodes.count();
  for (let i = 0; i < count; i++) {
    const node = await box(nodes.nth(i));
    expect(node.x, `node ${i} fell off the left edge`).toBeGreaterThanOrEqual(flow.x - 1);
    expect(
      node.x + node.width,
      `node ${i} fell off the right edge`,
    ).toBeLessThanOrEqual(flow.x + flow.width + 1);
  }
});

test("Zoom Out still reaches below the fit floor (#1261)", async ({ page }) => {
  // The floor this issue adds is on the FIT. `<ReactFlow minZoom>` is 0.1 and
  // must stay there: #1261 set it so an operator can zoom out to see a whole
  // pipeline's shape, and so the Zoom Out control is not disabled from load.
  await open(page, LONG, 1440);
  await expect(async () => {
    expect(await zoomOf(page)).toBe(LEGIBLE_FIT_ZOOM);
  }).toPass({ timeout: 10_000 });

  const zoomReadout = page.getByTestId("workflow-zoom-readout");
  await expect(zoomReadout).toHaveText("75%");

  const zoomOut = page.getByRole("button", { name: "Zoom Out" });
  await expect(zoomOut).toBeEnabled();
  for (let i = 0; i < 3; i++) await zoomOut.click();

  expect(await zoomOf(page)).toBeLessThan(LEGIBLE_FIT_ZOOM);
  await expect(zoomReadout).not.toHaveText("75%");
});
