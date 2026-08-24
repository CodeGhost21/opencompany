import { expect, test } from "@playwright/test";

/**
 * Issue #1385: the Overview legend was wider than a phone viewport and its
 * side paddles remained 128px by 48px at every width. These checks measure the
 * browser's boxes rather than the Tailwind class names: the canvas is clipped,
 * so an in-bounds legend is only useful if its right edge is truly reachable.
 */

const VIEWPORTS = [
  { width: 390, height: 844, paddle: { width: 32, height: 56, inset: 8 } },
  { width: 768, height: 900, paddle: { width: 40, height: 80, inset: 12 } },
] as const;

test.beforeEach(async ({ page }) => {
  // The first-run tour is unrelated chrome that can cover the graph.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

for (const viewport of VIEWPORTS) {
  test(`keeps graph chrome reachable at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/#/company/graph");

    const legend = page.getByTestId("kg-legend");
    const previous = page.getByRole("button", { name: "Previous desk" });
    const next = page.getByRole("button", { name: "Next desk" });
    await expect(legend).toBeVisible();
    await expect(previous).toBeVisible();
    await expect(next).toBeVisible();

    const [legendBox, previousBox, nextBox, canvasBox] = await Promise.all([
      legend.boundingBox(),
      previous.boundingBox(),
      next.boundingBox(),
      legend.evaluate((el) => {
        const canvas = el.parentElement;
        if (!canvas) return null;
        const { left, right } = canvas.getBoundingClientRect();
        return { left, right };
      }),
    ]);
    expect(legendBox, "the legend must have a rendered box").not.toBeNull();
    expect(previousBox, "the previous paddle must have a rendered box").not.toBeNull();
    expect(nextBox, "the next paddle must have a rendered box").not.toBeNull();
    expect(canvasBox, "the legend must be placed in the graph canvas").not.toBeNull();

    // The legend starts at the canvas inset and finishes inside the canvas —
    // in particular, none of its kinds are hidden past the clipped right edge.
    expect(legendBox!.x).toBeGreaterThanOrEqual(canvasBox!.left);
    expect(legendBox!.x + legendBox!.width).toBeLessThanOrEqual(canvasBox!.right + 1);

    for (const paddle of [previousBox!, nextBox!]) {
      expect(paddle.width).toBeLessThanOrEqual(viewport.paddle.width);
      expect(paddle.height).toBeLessThanOrEqual(viewport.paddle.height);
    }
    expect(previousBox!.x - canvasBox!.left).toBeGreaterThanOrEqual(viewport.paddle.inset - 1);
    expect(canvasBox!.right - (nextBox!.x + nextBox!.width)).toBeGreaterThanOrEqual(viewport.paddle.inset - 1);
  });
}
