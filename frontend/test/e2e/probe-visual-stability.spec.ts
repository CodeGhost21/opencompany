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
        const html = svg?.innerHTML ?? '';
        let h = 0;
        for (const el of svg?.querySelectorAll('[transform]') ?? []) {
          for (const ch of el.getAttribute('transform') ?? '') h = (h * 31 + ch.charCodeAt(0)) | 0;
        }
        let h2 = 0;
        for (const el of svg?.querySelectorAll('circle, path, ellipse') ?? []) {
          const s = el.getAttribute('transform') ?? el.getAttribute('d') ?? '';
          for (const ch of s.slice(0, 64)) h2 = (h2 * 31 + ch.charCodeAt(0)) | 0;
        }
        const circles = Array.from(svg?.querySelectorAll('circle') ?? []);
        let cxs = 0, cys = 0;
        for (const c of circles) {
          cxs += (parseFloat(c.getAttribute('cx') ?? '0') * 1000) | 0;
          cys += (parseFloat(c.getAttribute('cy') ?? '0') * 1000) | 0;
        }
        return JSON.stringify({ len: html.length, thash: h, dhash: h2, cxs, cys, n: circles.length });
      }),
    );
    await page.waitForTimeout(800);
  }
  console.log("PROBE_SAMPLES=" + JSON.stringify(samples, null, 1));
  expect(false).toBe(true);
});
