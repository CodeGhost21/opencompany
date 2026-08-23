/**
 * Temporary diagnostics spec — captures the settled knowledge-graph layout so
 * two fresh-host runs can be compared for determinism. NOT part of the suite;
 * deleted after use.
 */
import { writeFileSync } from "node:fs";

import { test, type Page } from "@playwright/test";

const CONTENT_SURFACE = '[data-testid="content-surface"]';

async function settleKnowledgeGraph(page: Page) {
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

test("capture settled graph layout", async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
  await page.addInitScript(() => {
    window.localStorage.setItem("theme", "dark");
  });
  await page.goto("/#/overview");
  await page.locator(CONTENT_SURFACE).waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  await settleKnowledgeGraph(page);

  const dump = await page.evaluate(() => {
    const svg = document.querySelector('svg[aria-label="Operating knowledge graph"]');
    if (!svg) return { error: "no svg" };
    // Every node is a <g> carrying transform="translate(x,y)" and a text label.
    const positions: Record<string, number[]> = {};
    for (const el of svg.querySelectorAll("g[transform]")) {
      const m = el.getAttribute("transform")?.match(/translate\((-?[\d.]+)\s*[, ]\s*(-?[\d.]+)\)/);
      if (m) {
        const label = (el.textContent || "").trim().slice(0, 40) || el.getAttribute("id") || "?";
        positions[label] = [Math.round(+m[1] * 10) / 10, Math.round(+m[2] * 10) / 10];
      }
    }
    return { count: Object.keys(positions).length, positions };
  });
  console.log("GRAPH_LAYOUT_JSON=" + JSON.stringify(dump));
  const out =
    process.env.CAPTURE_OUT ||
    "/tmp/graph-layout-" + (process.env.CAPTURE_TAG || "default") + ".json";
  writeFileSync(out, JSON.stringify(dump, null, 1));
});
