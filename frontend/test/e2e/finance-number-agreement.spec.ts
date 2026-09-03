import { expect, test } from "@playwright/test";

import { clearLedger, ledgerPath, seedLedger, subDollarMonth } from "./ledger";

/**
 * The Finance overview must tell one story about money.
 *
 * `spentUsd` is the sum of the magnitudes of the same negative ledger entries
 * the transaction list renders, so the tile and the rows are two renderings of
 * one number. They disagreed: the tile formatted to whole dollars while the rows
 * formatted to cents, so this month's $0.163 of real spend read as "Spend $0"
 * directly above a list of three outflows.
 *
 * Driven against a real host with a real seeded ledger rather than a stubbed
 * response, because the unit test already proves the component and what was
 * missing was any proof the *host's own projection* and the console agree. The
 * suite could not seed a spend at all before this — see `./ledger.ts`.
 *
 * Like the rest of `test/e2e`, this needs a running host and is not a CI merge
 * gate; `npm run typecheck:e2e` compiles it, `npm run e2e` runs it.
 */

const SEEDED = ledgerPath() !== null;

test.describe("Finance number agreement", () => {
  test.skip(
    !SEEDED,
    "needs a host this suite brought up, so the ledger can be seeded on its disk (unset PW_BASE_URL)",
  );

  test.beforeEach(async () => {
    clearLedger();
    seedLedger(subDollarMonth());
  });

  test.afterEach(async () => {
    clearLedger();
  });

  test("states this month's sub-dollar spend as the amount the rows total", async ({
    page,
  }) => {
    await page.goto("/#/finances/overview");
    await expect(page.getByText("Recent transactions")).toBeVisible();

    // The three seeded outflows, at the precision they were recorded in.
    await expect(page.getByText("Model turn — drafting reply")).toBeVisible();
    await expect(page.getByText("−$0.12")).toBeVisible();
    await expect(page.getByText("−$0.04")).toBeVisible();

    // $0.12 + $0.04 + $0.003 = $0.163, which is sixteen cents — not "$0".
    const spend = page.locator("div").filter({ hasText: /^Spend$/ }).first();
    const tile = page.locator("div.space-y-2").filter({ has: spend }).first();
    await expect(tile).toContainText("$0.16");
    await expect(tile).not.toContainText(/\$0(?!\.)/);
  });

  test("never reports a spend smaller than a cent as nothing", async ({ page }) => {
    await page.goto("/#/finances/overview");
    await expect(page.getByText("Spend by category")).toBeVisible();

    // The metered search is $0.003 — real money, below cent precision. It is
    // its own category, so the chart states it as a bound, not as zero.
    await expect(page.getByText("<$0.01").first()).toBeVisible();
    await expect(page.getByText("Metered web search")).toBeVisible();
  });

  test("says where the monthly budget comes from rather than only that it is unset", async ({
    page,
  }) => {
    await page.goto("/#/finances/overview");

    const budget = page.getByTestId("monthly-budget-origin");
    await expect(budget).toBeVisible();
    await expect(budget).toContainText("company manifest");
    await expect(budget).toContainText("cannot be changed here");
  });
});

/**
 * The spend-approval threshold is inert on this build, and the console says so.
 *
 * Pinned rather than fixed: `ManifestApprovalGate` is constructed
 * `with_policy_hitl_disabled`, and `evaluate` returns `Allow` above the tier
 * match and above the cap comparison, so `autoApproveUnderUsd` governs nothing
 * at runtime on either the native or the harness path. Enabling this control
 * would persist a number that changes no behaviour, which is worse than a
 * control that admits it is off. If policy HITL is ever turned on, this test
 * fails and is the reminder to wire the control in the same change.
 */
test("the spend-approval threshold stays disabled while policy HITL is off", async ({
  page,
}) => {
  await page.goto("/#/settings/approvals");

  const cap = page.locator("#spend-cap");
  await expect(cap).toBeVisible();
  await expect(cap).toBeDisabled();
  await expect(page.getByText("Spend approval threshold (inactive)")).toBeVisible();

  // The deadline control right beside it is live, so this is a deliberate
  // refusal rather than a page that failed to load.
  await expect(page.locator("#approval-deadline")).toBeEnabled();
});
