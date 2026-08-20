import { expect, test, type Page } from "@playwright/test";

/**
 * Issue #1284 — every list gets its own sidebar row, at the same tier as
 * Tasks, with no intermediate picker.
 *
 * Before this, `tasks` had its own nav entry and every other list — `goals`,
 * `decisions`, and anything a company declared — was buried one level down
 * inside a single "Ledgers" item, picked from an in-page column of rows. This
 * spec drives a live host (every default company ships `tasks`, `goals` and
 * `decisions` — `src/ledger/registry.rs`) and asserts the sidebar itself now
 * carries a row per list, and that clicking one lands directly on that list's
 * own screen.
 */

const API = "/api/v1/company";

test.beforeEach(async ({ page }) => {
  // The first-run tour opens a modal over the sidebar and swallows clicks.
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

function navRow(page: Page, slug: string) {
  return page.locator(`[data-tour="nav-ledgers-${slug}"]`).getByRole("button");
}

test("there is no single Ledgers row — Tasks, Goals and Decisions sit as peers", async ({
  page,
}) => {
  await page.goto("/#/overview");
  await dismissTour(page);

  // The old single item is gone outright.
  await expect(
    page.getByRole("button", { name: "Ledgers", exact: true }),
  ).toHaveCount(0);

  // Every built-in list has its own row, each reachable directly.
  await expect(navRow(page, "tasks")).toBeVisible({ timeout: 15_000 });
  await expect(navRow(page, "goals")).toBeVisible();
  await expect(navRow(page, "decisions")).toBeVisible();

  // They read by their own name, never "Ledger".
  await expect(navRow(page, "goals")).toHaveText("Goals");
  await expect(navRow(page, "decisions")).toHaveText("Decisions");
});

test("clicking a list's row opens that list directly — no picker in between", async ({
  page,
}) => {
  await page.goto("/#/overview");
  await dismissTour(page);

  await navRow(page, "goals").click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers/goals");
  await expect(page.getByRole("heading", { name: "Goals" })).toBeVisible({
    timeout: 15_000,
  });
  // The in-page column of every list — once part of this same screen, picked
  // from like a set of cards — is gone; "Decisions" only exists in the
  // sidebar now, not as a second control inside the main content area.
  await expect(
    page.getByRole("main").getByRole("button", { name: "Decisions", exact: true }),
  ).toHaveCount(0);

  await navRow(page, "decisions").click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers/decisions");
  await expect(page.getByRole("heading", { name: "Decisions" })).toBeVisible({
    timeout: 15_000,
  });
});

test("the sidebar picks up a newly declared list without a reload", async ({
  page,
  request,
}) => {
  const slug = `e2e-sidebar-${Date.now()}`;
  await page.goto("/#/overview");
  await dismissTour(page);
  await expect(navRow(page, "goals")).toBeVisible({ timeout: 15_000 });

  try {
    const declared = await request.post(`${API}/ledgers`, {
      data: {
        slug,
        title: "E2E sidebar list",
        purpose: "Proves the sidebar reads live lists.",
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

    // Reached through Manage Lists' own refresh path (Company → Manage lists),
    // which is the one place a declare actually happens from in the console —
    // this only proves the sidebar's *read* keeps up once it does.
    //
    // Both Manage Lists' own row and the sidebar's row (shell chrome, present
    // on every route) carry this exact title once the shared read lands, so
    // an unscoped text lookup would match two elements — scoped to `main`
    // here to mean "on the page itself", separate from `navRow` below.
    await page.locator('[data-tour="nav-company"]').getByRole("button").click();
    await page.getByTestId("company-manage-lists").click();
    await expect(page.getByRole("main").getByText("E2E sidebar list")).toBeVisible({
      timeout: 15_000,
    });

    await page.getByTestId("lists-breadcrumb-company").click();

    await expect(navRow(page, slug)).toBeVisible({ timeout: 15_000 });
  } finally {
    await request.delete(`${API}/ledgers/${slug}`);
  }
});
