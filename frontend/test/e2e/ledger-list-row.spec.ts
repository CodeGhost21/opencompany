import { expect, test } from "@playwright/test";

const API = "/api/v1/company";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

test("a list row leads with its title and shows one readable status", async ({
  page,
  request,
}) => {
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
        fields: { title, column: "todo", note: "First line\n\nSecond line" },
      },
    });
    expect(recorded.ok()).toBeTruthy();

    await page.goto(`/#/ledgers/${slug}`);
    await page.getByRole("button", { name: "List" }).click();

    const row = page.getByTestId(`ledger-entry-${id}`);
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row.getByTestId("ledger-entry-title")).toHaveText(title);
    await expect(row.getByTestId("ledger-entry-status")).toHaveText("To-do");
    await expect(row.getByTestId("ledger-entry-id")).toHaveText(id);
    await expect(row.getByText("column", { exact: true })).toHaveCount(0);
    await expect(row.locator("dd")).toHaveText("First line\nSecond line");

    const order = await row.locator(":scope > div").first().evaluate((header) =>
      Array.from(header.children).map((child) => child.getAttribute("data-testid")),
    );
    expect(order).toEqual(["ledger-entry-title", "ledger-entry-status", "ledger-entry-id"]);
  } finally {
    await request.delete(`${API}/ledgers/${slug}?purge=true`);
  }
});
