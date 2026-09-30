import { expect, test } from "@playwright/test";

import { LIVE_BRAIN } from "./capabilities";

/**
 * The MCP module's front door is one searchable list.
 *
 * Runs on the default-feature host the rest of this directory drives. Every
 * assertion is about rendering and navigation, which that host can answer. Tool
 * inventory reports `not_wired` without the `openhuman` feature, so nothing
 * about tool permissions belongs here: a spec asserting that a forbidden row is
 * absent would pass against a page that renders no rows at all.
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

/** The row for `name` in the one list. */
function row(page: Page, name: string) {
  return page.getByTestId("mcp-server-row").filter({ hasText: name });
}

test("the list carries the manifest server and where it came from", async ({
  page,
}) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));

  await openMcp(page);

  const deepwiki = row(page, "deepwiki");
  await expect(deepwiki).toBeVisible();
  await expect(deepwiki.getByTestId("mcp-source-badge")).toHaveText("manifest");

  expect(pageErrors).toEqual([]);
});

test("search narrows this company's own servers", async ({ page }) => {
  await openMcp(page);
  await expect(row(page, "deepwiki")).toBeVisible();

  await page.getByTestId("mcp-search").fill("no-such-server-anywhere");
  await expect(row(page, "deepwiki")).toHaveCount(0);

  await page.getByTestId("mcp-search").fill("deep");
  await expect(row(page, "deepwiki")).toBeVisible();
});

test("a row opens its detail from the arrow, and never from a hover", async ({
  page,
}) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");
  const expander = deepwiki.getByTestId("mcp-row-expander");

  await expect(expander).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByTestId("mcp-row-detail")).toHaveCount(0);

  // The earlier draft revealed on hover too, which made the table move under
  // the pointer on the way to anything else and was unreachable by touch.
  await deepwiki.hover();
  await expect(expander).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByTestId("mcp-row-detail")).toHaveCount(0);

  await expander.click();
  await expect(expander).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByTestId("mcp-row-detail")).toContainText(
    "https://mcp.deepwiki.com/mcp",
  );
});

test("clicking the row opens its detail, anywhere but a control", async ({
  page,
}) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");
  const expander = deepwiki.getByTestId("mcp-row-expander");

  await deepwiki.getByTestId("mcp-source-badge").click();
  await expect(expander).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByTestId("mcp-row-detail")).toBeVisible();

  await deepwiki.getByTestId("mcp-source-badge").click();
  await expect(expander).toHaveAttribute("aria-expanded", "false");
});

test("the name is the link, so no row carries a View button", async ({ page }) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");

  await expect(deepwiki.getByRole("button", { name: "View" })).toHaveCount(0);

  // The name navigates and does NOT also toggle the row underneath it: the row
  // is a click target now, so every control on it has to stop being one.
  await deepwiki.getByTestId("mcp-server-open").click();
  await expect(page.getByTestId("mcp-server-page")).toBeVisible();
  await expect(page.getByTestId("mcp-row-detail")).toHaveCount(0);
});

test("a control on the row does its own job and nothing else", async ({
  page,
}) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");

  // The overflow opens its menu without also disclosing the detail — the
  // failure being prevented is a menu that appears over a row that just grew.
  await deepwiki.getByTestId("mcp-row-overflow").click();
  await expect(page.getByTestId("mcp-toggle")).toBeVisible();
  await expect(page.getByTestId("mcp-row-detail")).toHaveCount(0);
});

test("a row keeps its secondary controls behind the overflow", async ({ page }) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");

  // The failure being prevented: the old row carried up to seven icon-only
  // controls, remove among them, distinguished only by aria-label — so a
  // destructive action sat one mis-click from a scan.
  //
  // Counted at page level, not inside the row: the menu content is portaled out
  // of it, so a row-scoped absence check would also pass with the menu open and
  // would therefore be a check that cannot fail.
  for (const hidden of ["mcp-toggle", "mcp-test", "mcp-tools", "mcp-permissions"]) {
    await expect(page.getByTestId(hidden)).toHaveCount(0);
  }

  await deepwiki.getByTestId("mcp-row-overflow").click();
  const menu = page.getByRole("menu");
  await expect(menu.getByTestId("mcp-toggle")).toBeVisible();
  await expect(menu.getByTestId("mcp-permissions")).toBeVisible();

  // A manifest declaration cannot be removed from the console at all, so the
  // destructive item is absent rather than present and refused.
  await expect(menu.getByTestId("mcp-remove")).toHaveCount(0);
});

test("the page states what this build can do with these servers", async ({
  page,
}) => {
  await openMcp(page);

  // Asserted in both directions: the notice is a function of the host, so a
  // hard-coded one fails on whichever lane it is wrong for.
  const notice = page.getByTestId("mcp-bridge-absent");
  if (LIVE_BRAIN) await expect(notice).toHaveCount(0);
  else await expect(notice).toBeVisible();
});

test("a double-click settles the row open and highlights nothing", async ({
  page,
}) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");
  const expander = deepwiki.getByTestId("mcp-row-expander");

  await deepwiki.getByTestId("mcp-source-badge").dblclick();

  await expect(expander).toHaveAttribute("aria-expanded", "true");
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toEqual(
    "",
  );
});

test("dragging across a row opens it and selects nothing", async ({ page }) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");
  const badge = deepwiki.getByTestId("mcp-source-badge");

  // `hover` first: the box must be measured with the pointer already on the row.
  await badge.hover();
  const box = await badge.boundingBox();
  if (box === null) throw new Error("the source badge has no box to drag across");
  await page.mouse.down();
  for (const dx of [-18, -6, 6, 18]) {
    await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2);
  }
  await page.mouse.up();

  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toEqual(
    "",
  );
  await expect(deepwiki.getByTestId("mcp-row-expander")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
});

test("the endpoint in the open detail can still be selected and copied", async ({
  page,
}) => {
  await openMcp(page);
  const deepwiki = row(page, "deepwiki");
  await deepwiki.getByTestId("mcp-row-expander").click();

  const endpoint = page
    .getByTestId("mcp-row-detail")
    .getByText("https://mcp.deepwiki.com/mcp");
  await expect(endpoint).toBeVisible();

  await endpoint.dblclick();
  expect(
    (await page.evaluate(() => window.getSelection()?.toString() ?? "")).length,
  ).toBeGreaterThan(0);
});
