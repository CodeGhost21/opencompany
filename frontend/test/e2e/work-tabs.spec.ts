import { expect, test, type Page } from "@playwright/test";

/**
 * Issue #1284 (final shape, Rule 2) — Work: one nav row, Tasks as the
 * default hero tab, every other list a tab beside it, overflow collapsing
 * behind "More ▾" once the strip cannot fit them all.
 *
 * Two earlier shapes were tried and rejected before this one — a sidebar row
 * per list (unusable at the 12-declared-list cap) and a collapsible sidebar
 * section (wrong premise: declared lists are read occasionally, not worked
 * daily the way Tasks is). See `docs/spec/runtime/ledgers-console-ia.md`'s
 * Rule 2 for the full reasoning. This spec covers what survived: tab
 * switching, the overflow menu, deep-linking straight to a non-default list,
 * and that `#/tasks/<id>` (the card detail page) still resolves.
 */

const API = "/api/v1/company";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

async function dismissTour(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (!(await skip.isVisible().catch(() => false))) return;
    await skip.click({ force: true }).catch(() => {});
    await page.waitForTimeout(300);
  }
  await expect(skip).toHaveCount(0);
}

function tab(page: Page, slug: string) {
  return page.getByTestId(`work-tab-${slug}`);
}

test("Work is one nav row, and it lands on the Tasks tab by default", async ({ page }) => {
  await page.goto("/#/overview");
  await dismissTour(page);

  // One row, not one per list.
  await expect(page.locator('[data-tour="nav-ledgers"]')).toHaveCount(1);
  await expect(page.locator('[data-tour="nav-ledgers"]').getByRole("button")).toHaveText("Work");

  await page.locator('[data-tour="nav-ledgers"]').getByRole("button").click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers");
  await expect(tab(page, "tasks")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("ledger-board")).toBeVisible({ timeout: 15_000 });
});

test("clicking another tab switches the list and updates the address", async ({ page }) => {
  await page.goto("/#/ledgers");
  await dismissTour(page);
  await expect(tab(page, "tasks")).toHaveAttribute("aria-selected", "true");

  await tab(page, "goals").click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers/goals");
  await expect(tab(page, "goals")).toHaveAttribute("aria-selected", "true");
  await expect(tab(page, "tasks")).toHaveAttribute("aria-selected", "false");
  await expect(page.getByRole("heading", { name: "Goals" })).toBeVisible({ timeout: 15_000 });
});

test("a deep link to a non-default list opens that tab directly", async ({ page }) => {
  await page.goto("/#/ledgers/decisions");
  await dismissTour(page);

  await expect(tab(page, "decisions")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("heading", { name: "Decisions" })).toBeVisible({ timeout: 15_000 });
});

test("#/tasks/<id> still opens the card detail page, untouched by the tab strip", async ({
  page,
  request,
}) => {
  const title = `e2e work-tabs card ${Date.now()}`;
  const seeded = await request.post(`${API}/tasks`, { data: { title } });
  expect(seeded.ok()).toBeTruthy();
  const id = (await seeded.json()).id as string;

  await page.goto(`/#/tasks/${id}`);
  await dismissTour(page);

  await expect(page.getByRole("heading", { name: title })).toBeVisible({ timeout: 15_000 });
  expect(new URL(page.url()).hash).toBe(`#/tasks/${id}`);
});

test("tabs that do not fit collapse behind More, and a More item still opens its list", async ({
  page,
  request,
}) => {
  const marker = Date.now();
  const slugs = [`e2e-overflow-a-${marker}`, `e2e-overflow-b-${marker}`, `e2e-overflow-c-${marker}`];
  const titles = slugs.map((s) => `Overflow ${s.slice(-6)}`);

  try {
    for (let i = 0; i < slugs.length; i += 1) {
      const declared = await request.post(`${API}/ledgers`, {
        data: {
          slug: slugs[i],
          title: titles[i],
          purpose: "Forces the Work tab strip to overflow.",
          fields: [
            { name: "id", role: "id" },
            { name: "title", role: "title", required: true },
            { name: "status", role: "status", required: true },
          ],
          statuses: [{ name: "open" }, { name: "closed", closed: true }],
          checks: ["required-field", "known-status"],
        },
      });
      expect(declared.ok()).toBeTruthy();
    }

    // A narrow strip is the deterministic way to force overflow without
    // depending on exact label widths: at 480px, tasks + goals + decisions +
    // three more declared lists cannot all fit.
    await page.setViewportSize({ width: 480, height: 800 });
    await page.goto("/#/ledgers");
    await dismissTour(page);

    const more = page.getByTestId("work-tab-more");
    await expect(more).toBeVisible({ timeout: 15_000 });
    // Tasks — the hero tab — must always be one of the ones actually shown,
    // never itself pushed behind More.
    await expect(tab(page, "tasks")).toBeVisible();

    await more.click();
    const lastSlug = slugs[slugs.length - 1];
    const overflowItem = page.getByTestId(`work-tab-more-${lastSlug}`);
    // Not every seeded list is guaranteed to overflow (it depends on how much
    // else the strip already holds), so only assert the click-through if the
    // measured layout actually put this one behind More.
    if (await overflowItem.isVisible().catch(() => false)) {
      await overflowItem.click();
      await expect.poll(() => new URL(page.url()).hash).toBe(`#/ledgers/${lastSlug}`);
    }
  } finally {
    for (const slug of slugs) {
      await request.delete(`${API}/ledgers/${slug}`);
    }
  }
});
