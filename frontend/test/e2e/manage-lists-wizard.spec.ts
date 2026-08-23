import { expect, test, type Page } from "@playwright/test";

/**
 * Issue #1284 — Manage Lists and the declare wizard, end to end.
 *
 * Declaring a list used to mean editing a JSON `fields[]`/`statuses[]`
 * template in a dialog reached from the Ledgers screen's own toolbar. This
 * drives the real replacement against a live host: Work's title switcher →
 * Manage lists → New list → the four plain-language steps and a review →
 * Create, then confirms the result is a real, retireable list — reachable
 * both from Manage Lists and as an entry in the title switcher's own menu
 * (`list-switcher.spec.ts` covers the switcher and its in-place wizard) —
 * and that retiring it asks first, the same confirm-before-destroy
 * assertion `ledger-retire-confirm.test.ts` makes at unit level.
 *
 * Manage Lists lives in Work now, not Company (`#/ledgers/manage`) — an
 * earlier cut put it under Company, "parallel to Manage Desks", and that
 * placement meant every visit crossed a section boundary (Work → Company →
 * Work) since the switcher is the only real entry point. See
 * `docs/spec/runtime/ledgers-console-ia.md`'s Rule 2.
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
  await page.goto("/#/ledgers");
  await dismissTour(page);
  await page.getByTestId("list-switcher-trigger").click();
  await page.getByTestId("list-switcher-manage").click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers/manage");
  await expect(page.getByRole("heading", { name: "Manage lists" })).toBeVisible({
    timeout: 15_000,
  });
}

test("the wizard declares a real list from Manage Lists, which appears in the switcher", async ({
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
  // neither the Manage Lists row nor the switcher's menu item exist until
  // Create actually lands.
  await expect(page.getByText(title)).toBeVisible();
  await expect(page.getByText(/open → closed/)).toBeVisible();
  await page.getByRole("button", { name: "Create list" }).click();

  const main = page.locator("main");

  try {
    // Lands back on Manage Lists with the new row live — no reload needed.
    await expect(page.getByRole("heading", { name: "Manage lists" })).toBeVisible({
      timeout: 15_000,
    });
    await expect(main.getByText(title)).toBeVisible({ timeout: 15_000 });

    // And it is a live entry in the switcher's menu the moment it exists
    // (issue #1284's core claim: no list is more than one click away,
    // direct, no picker).
    await page.getByTestId("lists-back").click();
    await page.getByTestId("list-switcher-trigger").click();
    await expect(page.getByTestId(`list-switcher-${slug}`)).toBeVisible({ timeout: 15_000 });
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

    // Confirm it is a live switcher entry first — Manage Lists and the
    // switcher read the same shared list, so declaring it (via the API seed
    // above) already made it one before this test ever opened the retire
    // confirm.
    await page.getByTestId("lists-back").click();
    await page.getByTestId("list-switcher-trigger").click();
    await expect(page.getByTestId(`list-switcher-${slug}`)).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press("Escape");
    await page.getByTestId("list-switcher-trigger").click();
    await page.getByTestId("list-switcher-manage").click();

    // Scoped to the one Card carrying this list's title, not any `div` that
    // happens to contain the text — several ancestor divs do, and only the
    // card itself has the Retire button as a sibling rather than a stranger.
    const row = page.locator('[data-slot="card"]', { hasText: title });
    await row.getByRole("button", { name: "Retire" }).click();

    const confirm = page.getByTestId("ledger-retire-confirm");
    await expect(confirm).toBeVisible();
    // Not gone yet — the confirm has not been pressed. `toBeAttached`, not
    // `toBeVisible`: the open AlertDialog dims and inerts the rest of the
    // page (confirmed live), so a visibility check here would be asserting
    // the modal's own backdrop rather than whether the retire actually
    // happened.
    await expect(main.getByText(title)).toBeAttached();

    await confirm.click();
    await expect(main.getByText(title)).toHaveCount(0, { timeout: 15_000 });

    // And it is gone from the switcher's menu too, with no reload.
    await page.getByTestId("lists-back").click();
    await page.getByTestId("list-switcher-trigger").click();
    await expect(page.getByTestId(`list-switcher-${slug}`)).toHaveCount(0, { timeout: 15_000 });
  } finally {
    // Idempotent: the happy path above already retired it. A failure partway
    // through must not leave this run's fixture for the next run to collide
    // with, the way the fixed-title version of this test used to.
    await request.delete(`${API}/ledgers/${slug}`);
  }
});

test("a managed list row opens that list", async ({ page }) => {
  await openManageLists(page);

  await page.getByTestId("managed-ledger-goals").click();

  await expect.poll(() => new URL(page.url()).hash).toBe("#/ledgers/goals");
  await expect(page.getByRole("heading", { name: "Goals" })).toBeVisible({
    timeout: 15_000,
  });
});
