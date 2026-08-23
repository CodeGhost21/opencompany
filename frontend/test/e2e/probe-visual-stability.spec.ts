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
  await page.waitForTimeout(2500);
  const samples = [] as string[];
  for (let i = 0; i < 5; i++) {
    samples.push(
      await page.evaluate(() => {
        const svg = document.querySelector('.oc-kg svg');
        const vb = svg?.getAttribute('viewBox') ?? 'none';
        const tickEl = document.querySelector('.oc-kg [data-tick]');
        const circles = Array.from(document.querySelectorAll('.oc-kg svg circle'))
          .slice(0, 8)
          .map((c) => c.getAttribute('cx') + ',' + c.getAttribute('cy'));
        const stage = (window as any).__probeStage;
        return JSON.stringify({ vb, tick: tickEl?.getAttribute('data-tick'), circles: circles.slice(0, 4), stage });
      }),
    );
    await page.waitForTimeout(800);
  }
  console.log("PROBE_SAMPLES=" + JSON.stringify(samples, null, 1));
  expect(false).toBe(true);
});
