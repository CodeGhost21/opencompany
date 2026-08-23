/**
 * Temporary diagnostics spec — compares the settled graph layout across two
 * page reloads in the SAME browser/host session. NOT part of the suite.
 */
import { writeFileSync } from "node:fs";

import { test, type Page } from "@playwright/test";

const CONTENT_SURFACE = '[data-testid="content-surface"]';

async function skipTour(page: Page) {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
}

async function settle(page: Page) {
  const svg = page.getByRole("img", { name: "Operating knowledge graph" });
  await svg.waitFor({ state: "visible", timeout: 30_000 });
  let previous = "";
  for (let attempt = 0; attempt < 24; attempt += 1) {
    const current = await svg.evaluate((el) => el.innerHTML);
    if (current === previous) return;
    previous = current;
    await page.waitForTimeout(750);
  }
}

async function capture(page: Page) {
  return page.evaluate(() => {
    const svg = document.querySelector('svg[aria-label="Operating knowledge graph"]');
    if (!svg) return {};
    const positions: Record<string, number[]> = {};
    for (const el of svg.querySelectorAll("g[transform]")) {
      const m = el.getAttribute("transform")?.match(/translate\((-?[\d.]+)\s*[, ]\s*(-?[\d.]+)\)/);
      if (!m) continue;
      const title = el.querySelector("title");
      const label = title ? title.textContent : "?";
      positions[label + "@" + el.querySelectorAll("*").length] = [
        Math.round(+m[1] * 10) / 10,
        Math.round(+m[2] * 10) / 10,
      ];
    }
    return positions;
  });
}

test("layout across two reloads in one session", async ({ page }) => {
  await skipTour(page);
  await page.addInitScript(() => window.localStorage.setItem("theme", "dark"));
  await page.goto("/#/overview");
  await page.locator(CONTENT_SURFACE).waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  const first = await capture(page);

  await page.reload();
  await page.locator(CONTENT_SURFACE).waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  await settle(page);
  const second = await capture(page);

  const keys = Object.keys(first);
  const common = keys.filter((k) => k in second);
  let moved = 0;
  let maxD = 0;
  for (const k of common) {
    const d = Math.hypot(first[k][0] - second[k][0], first[k][1] - second[k][1]);
    if (d > 0.5) { moved++; maxD = Math.max(maxD, d); }
  }
  writeFileSync("/tmp/reload-compare.json", JSON.stringify({
    count: keys.length, common: common.length,
    keysOnlyFirst: keys.filter((k) => !(k in second)),
    keysOnlySecond: Object.keys(second).filter((k) => !(k in first)),
    moved, maxD,
    first, second,
  }, null, 1));
});
