import { expect, test } from "@playwright/test";

import { LIVE_BRAIN, LIVE_BRAIN_REASON } from "./capabilities";
import { expectWorkflowIndex } from "./workflows";

/**
 * Issue #1697: a company-wide table of workflow runs — ran at, duration,
 * trigger, status — with a transcript side sheet, so "what ran and how did it
 * go" no longer means opening each workflow's own history rail in turn.
 *
 * These specs pin the two things that make the feature what it is rather than
 * a re-skin of the existing rail:
 *
 *  - the list is reachable from the index as its own tab, unscoped by
 *    workflow (the same company-wide read `WorkflowIndex`'s health strips
 *    already make);
 *  - a row opens a SHEET, not a navigation — the URL and the graph editor
 *    stay exactly where they were, unlike the `?run=` deep link, which still
 *    lands on the canvas.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

async function dismissTour(page: import("@playwright/test").Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  try {
    await skip.waitFor({ state: "visible", timeout: 10_000 });
  } catch {
    return;
  }
  await skip.click();
  await expect(skip).toBeHidden();
}

/** Switches the index from Workflows to Runs and waits for the table. */
async function openRunsTab(page: import("@playwright/test").Page) {
  await expectWorkflowIndex(page);
  await page.getByTestId("workflow-index-tab-runs").click();
  const list = page.getByTestId("workflow-run-traces");
  await expect(list).toBeVisible();
  return list;
}

test("the Runs tab lists the company-wide run page, or says there is none yet", async ({
  page,
}) => {
  await page.goto("/#/workflows");
  await dismissTour(page);
  const list = await openRunsTab(page);

  // The Workflows/Cards/List toggle is a Workflows-tab-only control — the
  // Runs tab is always a table, so it should not be offered a choice it does
  // not act on.
  await expect(page.getByTestId("workflow-index-cards")).toBeHidden();
  await expect(page.getByTestId("workflow-index-list")).toBeHidden();

  const rows = list.getByTestId("workflow-run-trace-row");
  const count = await rows.count();
  if (count === 0) {
    await expect(list.getByText(/No workflow runs yet/)).toBeVisible();
  } else {
    // Every row says which workflow, when it fired, and how — the four facts
    // the issue asks for, minus duration (absent on a run with no recorded
    // start, which the row already renders around rather than blocking on).
    await expect(rows.first().getByText(/^(Scheduled|Manual)$/)).toBeVisible();
  }
});

test("opening a run from the traces list shows its transcript without navigating", async ({
  page,
}) => {
  await page.goto("/#/workflows");
  await dismissTour(page);
  const list = await openRunsTab(page);

  const rows = list.getByTestId("workflow-run-trace-row");
  const count = await rows.count();
  test.skip(count === 0, "this company has no runs yet to open a transcript for");

  const urlBefore = page.url();
  await rows.first().click();

  const sheet = page.getByTestId("run-trace-sheet");
  await expect(sheet).toBeVisible();
  // The whole point: reading a run's transcript is not a page navigation.
  // The old `?run=` deep link lands on the graph editor; this must not.
  expect(page.url()).toBe(urlBefore);

  // The sheet always has a place to look for what each node produced — even
  // when the run predates output capture, it says so rather than being blank.
  await expect(sheet.getByText("NODE OUTPUT")).toBeVisible();
});

test("running a workflow surfaces it in the traces list, and its sheet's canvas link navigates there", async ({
  page,
}) => {
  // The two specs above read whatever history the host already holds and
  // pass on a default build. This one has to actually RUN a workflow, and the
  // runner lives behind `openhuman` — with the feature off the Run button
  // journals nothing and there would be no fresh row to find.
  test.skip(!LIVE_BRAIN, LIVE_BRAIN_REASON);

  await page.goto("/#/workflows");
  await dismissTour(page);

  await expectWorkflowIndex(page);
  const firstCard = page.getByTestId("workflow-card").first();
  const name = (await firstCard.locator("span.font-semibold").first().textContent())?.trim();
  await firstCard.click();
  await expect(page.getByTestId("workflow-detail-name")).toBeVisible();

  // The run may fail (an agent node with no inference source) — that is fine
  // and in fact the more interesting case: a failed run belongs in the traces
  // list exactly as much as a clean one.
  await page.getByRole("button", { name: "Run", exact: true }).click();
  await page.waitForURL(/#\/workflows\/[^/]+$/);

  await page.getByTestId("workflow-back-to-index").click();
  const list = await openRunsTab(page);

  const row = list.getByTestId("workflow-run-trace-row").filter({ hasText: name ?? "" }).first();
  await expect(row).toBeVisible({ timeout: 60_000 });
  await row.click();

  const sheet = page.getByTestId("run-trace-sheet");
  await expect(sheet).toBeVisible();

  await sheet.getByRole("button", { name: /Show on canvas/ }).click();
  // The sheet's canvas link is the one deliberate exception to "opening a run
  // never navigates" — it is an opt-in, not the row click.
  await expect(page).toHaveURL(/#\/workflows\/[^/]+\?run=/);
});
