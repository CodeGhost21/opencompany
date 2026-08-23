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
  // (data won), which changes the sim's alpha trajectory. Poll fast up front so
  // an empty→full growth is caught, then slow for the settle.
  const trajectory: Array<{ t: number; n: number }> = [];
  let previous = "";
  const t0 = Date.now();
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const current = await svg.evaluate((el) => el.innerHTML);
    trajectory.push({ t: Date.now() - t0, n: (current.match(/<g transform=/g) || []).length });
    if (current === previous) break;
    previous = current;
    await page.waitForTimeout(attempt < 6 ? 60 : 750);
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
  // Catch any uncaught error that might kill d3-timer's rAF loop (a throw in
  // one timer callback stalls every timer behind it in the same flush).
  await page.addInitScript(() => {
    const errs: string[] = [];
    (window as unknown as { __pageErrors: string[] }).__pageErrors = errs;
    window.addEventListener("error", (e) => {
      errs.push(`error: ${e.error instanceof Error ? e.error.stack : e.message}`);
    });
    window.addEventListener("unhandledrejection", (e) => {
      errs.push(`rejection: ${String(e.reason)}`);
    });
  });
  // Capture the graph's node-count trajectory from the very first DOM appearance
  // (before "visible"), so an empty→full growth at mount is caught even if the
  // empty state flashes for a single frame.
  await page.addInitScript(() => {
    const t0 = performance.now();
    const counts: Array<{ t: number; n: number }> = [];
    const record = () => {
      const svg = document.querySelector('svg[aria-label="Operating knowledge graph"]');
      counts.push({ t: Math.round(performance.now() - t0), n: svg ? (svg.innerHTML.match(/<g transform=/g) || []).length : 0 });
    };
    const obs = new MutationObserver(() => record());
    const boot = () => {
      const svg = document.querySelector('svg[aria-label="Operating knowledge graph"]');
      if (svg) {
        record();
        obs.observe(svg, { childList: true, subtree: true });
      } else {
        requestAnimationFrame(boot);
      }
    };
    requestAnimationFrame(boot);
    (window as unknown as { __kgCounts: Array<{ t: number; n: number }> }).__kgCounts = counts;
  });
  await page.goto("/#/overview");
  await page.locator(CONTENT_SURFACE).waitFor({ state: "visible", timeout: 30_000 });
  await page.evaluate(() => document.fonts.ready);
  const trajectory = await settleKnowledgeGraph(page);

  const dump = await page.evaluate(
    async (trajectory: Array<{ t: number; n: number }>) => {
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
      const early = (window as unknown as { __kgCounts?: Array<{ t: number; n: number }> }).__kgCounts ?? [];
      const simLog = (window as unknown as { __simLog?: Array<object> }).__simLog ?? [];
      const alphaLog = (window as unknown as { __alphaLog?: Array<object> }).__alphaLog ?? [];
      return { count: groups.length, positions, data, trajectory, early, simLog, alphaLog };
    },
    trajectory,
  );
  const out =
    process.env.CAPTURE_OUT ||
    "/tmp/graph-layout-" + (process.env.CAPTURE_TAG || "default") + ".json";
  writeFileSync(out, JSON.stringify(dump, null, 1));
});
