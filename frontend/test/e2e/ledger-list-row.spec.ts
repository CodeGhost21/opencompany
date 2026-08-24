import { expect, test } from "@playwright/test";

const API = "/api/v1/company";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of [
      "oc-tour:single",
      "oc-tour:e2e-harness-co",
      "oc-tour:null",
    ]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

test("a list row leads with its title and shows one readable status", async ({
  page,
  request,
}) => {
  // The row renders correctly but the whole job slows to a crawl when the CI
  // runner pool is saturated — page load alone can eat the suite's 60s default
  // before the assertions run. The assertions below are exact; the budget is
  // the only thing under-tuned, so it is stated rather than inherited.
  test.setTimeout(120_000);

  const marker = Date.now();
  const slug = `e2e-list-row-${marker}`;
  const title = `E2E list row ${marker}`;
  const id = `entry-${marker}`;
  const declared = await request.post(`${API}/ledgers`, {
    data: {
      slug,
      title: `E2E list ${marker}`,
      purpose: "A list used to check its row presentation.",
      fields: [
        { name: "id", role: "id" },
        { name: "title", role: "title", required: true },
        { name: "column", role: "status", required: true },
        { name: "note", role: "prose" },
      ],
      statuses: [{ name: "todo", label: "To-do" }],
      checks: ["required-field", "known-status"],
    },
  });
  expect(declared.ok()).toBeTruthy();

  try {
    const recorded = await request.post(`${API}/ledgers/${slug}/entries`, {
      data: {
        id,
        status: "todo",
        fields: {
          title,
          column: "todo",
          note: "First line\n\n    indented line\nSecond line",
        },
      },
    });
    expect(recorded.ok()).toBeTruthy();

    await page.goto(`/#/ledgers/${slug}`);
    // Declared ledgers open in their readable list form (`defaultLedgerMode`,
    // issue #1351), so the row below is the list row already — the "List"
    // toggle only exists when the board is the active view, and it is not for
    // a declared list. No toggle click needed to get there.

    const row = page.getByTestId(`ledger-entry-${id}`);
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row.getByTestId("ledger-entry-title")).toHaveText(title);
    await expect(row.getByTestId("ledger-entry-status")).toHaveText("To-do");
    await expect(row.getByTestId("ledger-entry-id")).toHaveText(id);
    await expect(row.getByText("column", { exact: true })).toHaveCount(0);
    // The blank paragraph goes — that is `compactFieldValue`, and textContent
    // shows it. Indentation inside a line survives only because the row renders
    // with `pre-wrap`; CSS collapsing under `pre-line` leaves textContent
    // untouched, so the computed style is what actually guards it.
    const note = row.locator("dd");
    expect(await note.evaluate((el) => el.textContent)).toBe(
      "First line\n    indented line\nSecond line",
    );
    expect(await note.evaluate((el) => getComputedStyle(el).whiteSpace)).toBe(
      "pre-wrap",
    );

    const order = await row.locator(":scope > div").first().evaluate((header) =>
      Array.from(header.children).map((child) => child.getAttribute("data-testid")),
    );
    expect(order).toEqual([
      "ledger-entry-title",
      "ledger-entry-status",
      "ledger-entry-id",
    ]);
  } finally {
    await request.delete(`${API}/ledgers/${slug}?purge=true`);
  }
});
