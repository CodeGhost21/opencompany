/**
 * Temporary diagnostics spec — captures the full settled SVG structure (node
 * titles, node positions, edge paths) so two fresh-host runs can be diffed.
 * NOT part of the suite; deleted after use.
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
    const groups = svg.querySelectorAll("g[transform]");
    const positions: Record<string, number[]> = {};
    for (const el of groups) {
      const m = el.getAttribute("transform")?.match(/translate\((-?[\d.]+)\s*[, ]\s*(-?[\d.]+)\)/);
      if (!m) continue;
      const title = el.querySelector("title");
      const label = title ? title.textContent : "";
      const key = label || `noid@${Math.round(+m[1])},${Math.round(+m[2])}`;
      positions[key] = [Math.round(+m[1] * 10) / 10, Math.round(+m[2] * 10) / 10];
    }
    // Edge paths: capture the d attribute of every <path> — this encodes the
    // full link topology (which nodes connect, via their coordinates).
    const edgePaths = Array.from(svg.querySelectorAll("path[d]")).map((p) => p.getAttribute("d"));
    // The full innerHTML hash for reference.
    const htmlLen = svg.innerHTML.length;
    return { count: groups.length, positions, edgePathCount: edgePaths.length, edgePaths, htmlLen };
  });
  const out =
    process.env.CAPTURE_OUT ||
    "/tmp/graph-layout-" + (process.env.CAPTURE_TAG || "default") + ".json";
  writeFileSync(out, JSON.stringify(dump, null, 1));
});
