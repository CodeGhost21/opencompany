/**
 * Temporary diagnostics spec — captures the settled graph layout AND the raw
 * API data that feeds it, so two fresh-host runs can be diffed end to end.
 * NOT part of the suite; deleted after use.
 */
import { writeFileSync } from "node:fs";

import { test, type Page } from "@playwright/test";

const CONTENT_SURFACE = '[data-testid="content-surface"]';

async function settleKnowledgeGraph(page: Page) {
  const svg = page.getByRole("img", { name: "Operating knowledge graph" });
  await svg.waitFor({ state: "visible", timeout: 30_000 });
  // Record the node-count trajectory from first visibility — detects whether the
  // graph mounted empty (chunk won the load race) and grew, or mounted full
  // (data won), which changes the sim's alpha trajectory.
  const trajectory: Array<{ t: number; n: number }> = [];
  let previous = "";
  const t0 = Date.now();
  for (let attempt = 0; attempt < 24; attempt += 1) {
    const current = await svg.evaluate((el) => el.innerHTML);
    trajectory.push({ t: Date.now() - t0, n: (current.match(/<g transform=/g) || []).length });
    if (current === previous) break;
    previous = current;
    await page.waitForTimeout(750);
  }
  return trajectory;
}

test("capture settled graph layout + API data", async ({ page }) => {
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

  const dump = await page.evaluate(async () => {
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
    // Raw API data feeding the graph — same reads Overview makes.
    const paths = [
      "/api/v1/company/tasks",
      "/api/v1/company/team",
      "/api/v1/company/desks",
      "/api/v1/company/memory",
      "/api/v1/company/workflows",
    ];
    const data: Record<string, unknown> = {};
    for (const p of paths) {
      try {
        const r = await fetch(p);
        data[p] = r.ok ? await r.json() : { http: r.status };
      } catch (e) {
        data[p] = { fetchError: String(e) };
      }
    }
    return { count: groups.length, positions, data };
  });
  const out =
    process.env.CAPTURE_OUT ||
    "/tmp/graph-layout-" + (process.env.CAPTURE_TAG || "default") + ".json";
  writeFileSync(out, JSON.stringify(dump, null, 1));
});
