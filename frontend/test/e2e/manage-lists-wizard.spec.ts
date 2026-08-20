import { expect, test, type Page } from "@playwright/test";

/**
 * Issue #1284 — Manage Lists and the declare wizard, end to end.
 *
 * Declaring a list used to mean editing a JSON `fields[]`/`statuses[]`
 * template in a dialog reached from the Ledgers screen's own toolbar. This
 * drives the real replacement against a live host: Company → Manage lists →
 * New list → the four plain-language steps and a review → Create, then
 * confirms the result is a real, retireable list — reachable both from Manage
 * Lists and as its own sidebar row — and that retiring it asks first, the same
 * confirm-before-destroy assertion `ledger-retire-confirm.test.ts` makes at
 * unit level.
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

async function openManageLists(page: Page) {
  await page.goto("/#/overview");
  await dismissTour(page);
  await page.locator('[data-tour="nav-company"]').getByRole("button").click();
  await page.getByTestId("company-manage-lists").click();
  await expect(page.getByRole("heading", { name: "Manage lists" })).toBeVisible({
    timeout: 15_000,
  });
}

test("the wizard declares a real list, which appears in Manage Lists and the sidebar", async ({
  page,
  request,
}) => {
  const title = `E2E promises ${Date.now()}`;
  await openManageLists(page);

  await page.getByRole("button", { name: "New list" }).click();
  await expect(page.getByRole("heading", { name: "New list" })).toBeVisible();

  // Step 1 — what to track.
  await page.getByLabel("What do you want to track?").fill("What we promised, and whether we kept it.");
  await page.getByRole("button", { name: "Next" }).click();

  // Step 2 — name it. The slug derives from the title.
  await page.getByLabel("Name it").fill(title);
  await expect(page.getByLabel("Short id")).not.toHaveValue("");
  const slug = await page.getByLabel("Short id").inputValue();
  await page.getByRole("button", { name: "Next" }).click();

  // Step 3 — stages. Open/Closed is a preset, needs no typing.
  await page.getByRole("button", { name: "Open / Closed" }).click();
  await page.getByRole("button", { name: "Next" }).click();

  // Step 4 — row details. Owner, toggled on.
  await page.locator("label", { hasText: "Owner" }).getByRole("switch").click();
  await page.getByRole("button", { name: "Next" }).click();

  // Step 5 — review, then submit. Scoped to `main`: the review text and the
  // eventual Manage Lists row both carry the title, and once this list also
  // has a sidebar row (checked below) the title exists on screen twice —
  // `getByText` unscoped would then match both and fail Playwright's
  // single-match requirement.
  const main = page.getByRole("main");
  await expect(main.getByText(title)).toBeVisible();
  await expect(main.getByText(/open → closed/)).toBeVisible();
  await page.getByRole("button", { name: "Create list" }).click();

  try {
    // Lands back on Manage Lists with the new row live — no reload needed.
    await expect(page.getByRole("heading", { name: "Manage lists" })).toBeVisible({
      timeout: 15_000,
    });
    await expect(main.getByText(title)).toBeVisible({ timeout: 15_000 });

    // And it is a live sidebar row the moment it exists (issue #1284's core
    // claim: no list is one click further from the sidebar than any other).
    await page.getByTestId("lists-breadcrumb-company").click();
    await expect(
      page.locator(`[data-tour="nav-ledgers-${slug}"]`).getByRole("button"),
    ).toBeVisible({ timeout: 15_000 });
  } finally {
    await request.delete(`${API}/ledgers/${slug}`);
  }
});

test("retiring a declared list asks before it deletes, then removes it everywhere", async ({
  page,
  request,
}) => {
  const slug = `e2e-retire-${Date.now()}`;
  const declared = await request.post(`${API}/ledgers`, {
    data: {
      slug,
      title: "E2E retire target",
      purpose: "A list this spec retires through the UI.",
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

  const main = page.getByRole("main");
  await openManageLists(page);
  await expect(main.getByText("E2E retire target")).toBeVisible({ timeout: 15_000 });
  // The sidebar is shell chrome, present on every route — it already picked
  // this list up from the seed above, before the confirm below removes it.
  const sidebarRow = page.locator(`[data-tour="nav-ledgers-${slug}"]`).getByRole("button");
  await expect(sidebarRow).toBeVisible({ timeout: 15_000 });

  // Scoped to the one Card carrying this list's title, not any `div` that
  // happens to contain the text — several ancestor divs do, and only the
  // card itself has the Retire button as a sibling rather than a stranger.
  const row = page.locator('[data-slot="card"]', { hasText: "E2E retire target" });
  await row.getByRole("button", { name: "Retire" }).click();

  const confirm = page.getByTestId("ledger-retire-confirm");
  await expect(confirm).toBeVisible();
  // Not gone yet — the confirm has not been pressed. `toBeAttached`, not
  // `toBeVisible`: the open AlertDialog dims and inerts the rest of the page
  // (confirmed live — the row and the sidebar are still in the DOM, just
  // occluded), so a visibility check here would be asserting the modal's own
  // backdrop rather than whether the retire actually happened.
  await expect(main.getByText("E2E retire target")).toBeAttached();
  await expect(sidebarRow).toBeAttached();

  await confirm.click();
  await expect(main.getByText("E2E retire target")).toHaveCount(0, { timeout: 15_000 });
  await expect(sidebarRow).toHaveCount(0);
});
