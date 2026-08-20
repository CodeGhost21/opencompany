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

  // Step 5 — review, then submit. Unscoped here on purpose: the wizard is a
  // Dialog portaled onto `document.body`, so its own review text is the only
  // place `title` exists on screen at this point — no ambiguity yet, since
  // neither the Manage Lists row nor the sidebar row exist until Create
  // actually lands.
  await expect(page.getByText(title)).toBeVisible();
  await expect(page.getByText(/open → closed/)).toBeVisible();
  await page.getByRole("button", { name: "Create list" }).click();

  // Once the list exists, its title is on screen twice — the Manage Lists
  // card and the sidebar's own row (shell chrome, present on every route) —
  // so `main` scopes every check below to "on this page", leaving the
  // sidebar to `navRow`-style locators instead. `page.locator("main")`
  // (a plain element query), not `getByRole("main")`: the retire confirm
  // below opens an AlertDialog that inerts the rest of the page for
  // accessibility, which prunes `<main>` from the accessibility tree — a
  // role-based query would then find nothing while a role-agnostic one still
  // sees the (occluded but present) DOM.
  const main = page.locator("main");

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
  // Unique per run, title included: a title fixed across runs would collide
  // with whatever a previous *failed* run of this same test left behind (this
  // test declares and then deletes its own fixture, so nothing should
  // survive it — but a run that fails between the two must not poison every
  // run after it with a duplicate "on screen twice" match).
  const marker = Date.now();
  const slug = `e2e-retire-${marker}`;
  const title = `E2E retire target ${marker}`;
  const declared = await request.post(`${API}/ledgers`, {
    data: {
      slug,
      title,
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

  try {
    // `page.locator("main")`, not `getByRole("main")`: the retire confirm
    // below opens an AlertDialog that inerts the rest of the page for
    // accessibility, which prunes `<main>` from the accessibility tree
    // entirely — a role-based query would find nothing while a role-agnostic
    // one still sees the (occluded but present) DOM.
    const main = page.locator("main");
    await openManageLists(page);
    await expect(main.getByText(title)).toBeVisible({ timeout: 15_000 });
    // The sidebar is shell chrome, present on every route — it already
    // picked this list up from the seed above, before the confirm below
    // removes it. A plain CSS descendant (`button`), not `.getByRole`: the
    // same aria-hidden pruning `main` needs a role-agnostic locator for
    // below applies here too once the confirm dialog inerts the sidebar.
    const sidebarRow = page.locator(`[data-tour="nav-ledgers-${slug}"] button`);
    await expect(sidebarRow).toBeVisible({ timeout: 15_000 });

    // Scoped to the one Card carrying this list's title, not any `div` that
    // happens to contain the text — several ancestor divs do, and only the
    // card itself has the Retire button as a sibling rather than a stranger.
    const row = page.locator('[data-slot="card"]', { hasText: title });
    await row.getByRole("button", { name: "Retire" }).click();

    const confirm = page.getByTestId("ledger-retire-confirm");
    await expect(confirm).toBeVisible();
    // Not gone yet — the confirm has not been pressed. `toBeAttached`, not
    // `toBeVisible`: the open AlertDialog dims and inerts the rest of the
    // page (confirmed live — the row and the sidebar are still in the DOM,
    // just occluded), so a visibility check here would be asserting the
    // modal's own backdrop rather than whether the retire actually happened.
    await expect(main.getByText(title)).toBeAttached();
    await expect(sidebarRow).toBeAttached();

    await confirm.click();
    await expect(main.getByText(title)).toHaveCount(0, { timeout: 15_000 });
    await expect(sidebarRow).toHaveCount(0);
  } finally {
    // Idempotent: the happy path above already retired it. A failure partway
    // through must not leave this run's fixture for the next run to collide
    // with, the way the fixed-title version of this test used to.
    await request.delete(`${API}/ledgers/${slug}`);
  }
});
