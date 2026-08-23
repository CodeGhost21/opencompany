import { expect, test } from "@playwright/test";

const API = "/api/v1/company";

test("repro the List click hang", async ({ page, request }) => {
  const marker = Date.now();
  const slug = `zz-repro-${marker}`;
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
  const declared = await request.post(`${API}/ledgers`, {
    data: {
      slug, title: `ZZ list ${marker}`, purpose: "repro",
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
    await request.post(`${API}/ledgers/${slug}/entries`, {
      data: { id: `entry-${marker}`, status: "todo", fields: { title: `ZZ row ${marker}`, column: "todo", note: "First line\n\n    indented line\nSecond line" } },
    });
    await page.goto(`/#/ledgers/${slug}`);
    console.log("GOTO DONE");
    await page.getByRole("button", { name: "List", exact: true }).click({ timeout: 10_000 });
    console.log("CLICK DONE");
    const row = page.getByTestId(`ledger-entry-entry-${marker}`);
    await expect(row).toBeVisible({ timeout: 10_000 });
    console.log("ROW VISIBLE");
  } finally {
    await request.delete(`${API}/ledgers/${slug}?purge=true`).catch((e) => console.log("DELETE FAILED:", String(e).slice(0, 200)));
    console.log("CLEANUP DONE");
  }
});
