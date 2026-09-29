import { randomUUID } from "node:crypto";

import { expect, test } from "@playwright/test";

/**
 * Adding a server asks what it is for.
 *
 * Default-feature host: the add and remove routes are served here, which is
 * what this spec drives. Probing is `not_wired` on this build, so neither the
 * probe line nor the probed-description offer can appear — both are asserted in
 * the unit suite instead, and what is asserted here is the outcome step itself.
 */

type Page = import("@playwright/test").Page;

async function openMcp(page: Page) {
  await page.goto("/#/connections/mcp");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already seen in this context */
    });
  await expect(page.getByTestId("mcp-search")).toBeVisible({ timeout: 30_000 });
}

test("the form is a dialog, not four fields on every visit", async ({ page }) => {
  await openMcp(page);

  await expect(page.getByTestId("mcp-add-dialog")).toHaveCount(0);
  await page.getByTestId("mcp-add-open").click();
  await expect(page.getByTestId("mcp-add-dialog")).toBeVisible();
  await expect(page.getByTestId("mcp-add-description")).toBeVisible();
});

test("a server added by URL can say what it is for", async ({ page }) => {
  // Unique per run: the host keeps runtime servers in its secret store, so a
  // fixed name plus a tolerated "already exists" would let this spec adopt a
  // leftover registration pointing anywhere at all.
  const name = `pw-add-${randomUUID().slice(0, 8)}`;

  await openMcp(page);
  await page.getByTestId("mcp-add-open").click();
  await page.getByTestId("mcp-add-name").fill(name);
  await page.getByTestId("mcp-add-endpoint").fill("https://mcp.example.test/mcp");
  await page
    .getByTestId("mcp-add-description")
    .fill("Reads the quarterly reporting replica.");
  await page.getByTestId("mcp-add-submit").click();

  // The outcome step, not the probe line on it: probing is `not_wired` on this
  // build, so the host sends no `test` and that line is correctly absent. The
  // button back to the server is what says the add landed.
  await expect(page.getByTestId("mcp-add-open-server")).toBeVisible({
    timeout: 30_000,
  });
  await page.getByRole("button", { name: "Done" }).click();

  const added = page.getByTestId("mcp-server-row").filter({ hasText: name });
  await expect(added).toBeVisible({ timeout: 30_000 });
  await expect(added).toContainText("Reads the quarterly reporting replica.");

  // Leave the company as it was found.
  await added.getByTestId("mcp-row-overflow").click();
  await page.getByRole("menu").getByTestId("mcp-remove").click();
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(added).toHaveCount(0, { timeout: 30_000 });
});

test("a name already in use is refused in place", async ({ page }) => {
  // A name the host already holds as a *runtime* entry. Not the manifest's
  // `deepwiki`: the host refuses that with a different sentence ("declared in
  // this company's bundle — update it to override"), which the dialog reads as a
  // form-level refusal rather than a Name-field one. Driving the manifest name
  // here would have asserted the field message against a response that never
  // carries it.
  const taken = `pw-dup-${randomUUID().slice(0, 8)}`;
  const registered = await page.request.post("/api/v1/company/mcp/servers", {
    data: { name: taken, endpoint: "https://mcp.example.test/taken" },
  });
  expect(
    registered.ok(),
    `registering ${taken} failed: ${registered.status()}`,
  ).toBeTruthy();

  try {
    await openMcp(page);
    await page.getByTestId("mcp-add-open").click();
    await page.getByTestId("mcp-add-name").fill(taken);
    await page
      .getByTestId("mcp-add-endpoint")
      .fill("https://mcp.example.test/mcp");
    await page.getByTestId("mcp-add-submit").click();

    // Named on the field that is wrong, not as a toast that has already gone by
    // the time the operator looks for it — and the dialog stays open, with what
    // was typed still in it.
    await expect(page.getByTestId("mcp-add-name-error")).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByTestId("mcp-add-dialog")).toBeVisible();
    await expect(page.getByTestId("mcp-add-name")).toHaveValue(taken);
  } finally {
    await page.request
      .delete(`/api/v1/company/mcp/servers/${encodeURIComponent(taken)}`)
      .catch(() => undefined);
  }
});
