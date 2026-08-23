import { expect, test, type Page } from "@playwright/test";

// Throwaway probe (delete after use): does the Overview knowledge graph hold
// still under the visual lane's `reducedMotion: "reduce"` well enough that two
// consecutive full-page screenshots are byte-identical — WITHOUT any mask?

// Replicate the visual lane's pinned media-query settings for this probe.
test.use({
  viewport: { width: 1280, height: 720 },
  deviceScaleFactor: 1,
  reducedMotion: "reduce",
});

const CONTENT_SURFACE = '[data-testid="content-surface"]';

async function skipTour(page: Page) {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
}

async function open(page: Page) {
  await skipTour(page);
  await page.addInitScript((value) => {
    window.localStorage.setItem("theme", value);
  }, "dark");
  await page.goto("/#/overview");
  await expect(page.locator(CONTENT_SURFACE)).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("html")).toHaveClass(/\bdark\b/);
  await expect(page.locator(`${CONTENT_SURFACE} >> text=Drawing the graph…`)).toHaveCount(0, {
    timeout: 30_000,
  });
  await page.evaluate(() => document.fonts.ready);
  await page.addStyleTag({
    content: `
      *::-webkit-scrollbar { display: none !important; }
      * { scrollbar-width: none !important; }
    `,
  });
}

test("overview holds still without the mask (probe)", async ({ page }) => {
  await open(page);
  // Let the d3 sim fully cool to sleep (~8s at alphaDecay 0.015) before testing.
  await page.waitForTimeout(12_000);
  const a = await page.screenshot({ fullPage: true, animations: "disabled", caret: "hide" });
  await page.waitForTimeout(2000);
  const b = await page.screenshot({ fullPage: true, animations: "disabled", caret: "hide" });
  await import("node:fs/promises").then(async (fs) => {
    await fs.writeFile("/tmp/probe-a.png", a);
    await fs.writeFile("/tmp/probe-b.png", b);
  });
  const same = a.equals(b);
  console.log(`PROBE overview stable without mask after cool-down: ${same} (${a.length} bytes)`);
  expect(same).toBe(true);
});
