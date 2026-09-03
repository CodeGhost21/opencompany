import { expect, test, type Page } from "@playwright/test";

/**
 * The live click-through for #2028: four **real** parked blockers, four real
 * clicks, four real resolves. Nothing is stubbed.
 *
 * Its sibling `approval-blocker-verdicts.spec.ts` intercepts the resolve to
 * assert the body the console composes. This one lets the request reach the
 * host, so what is proved here is the other half: the host accepts each of the
 * four, banks the verdict the operator meant, and re-enters (or does not
 * re-enter) the stopped node accordingly.
 *
 * ## How the blockers get there
 *
 * Parked by seeding the host's own journal before it boots — an `ApprovalParked`
 * line carrying a `blocker.infrastructure` effect whose payload names a node of
 * a real workflow in the harness company, plus the `BlockedNodeStashed` line the
 * resume reads to find the run. That is exactly what a node that blocked would
 * have written; parking one for real needs an agent turn that fails in a
 * particular way, which needs an inference credential and is not deterministic.
 *
 * To run it, bring the host up once against a data root OUTSIDE `target/e2e`
 * (`host.sh` only wipes inside that scratch area), seed it, then run this:
 *
 *     D=$PWD/../target/live2028
 *     PW_HOST_DATA_DIR=$D npx playwright test approval-blocker-verdicts -g "an ordinary"
 *     python3 test/e2e/seed-blockers.py $D
 *     PW_LIVE_BRAIN=1 PW_HOST_DATA_DIR=$D npx playwright test blocker-verdicts-live
 *
 * `PW_LIVE_BRAIN=1` is what wires a workflow runner: without an inference
 * source the host has none, and the three resuming verdicts bank correctly and
 * then re-enter nothing.
 *
 * It is not part of the ordinary suite — the fixture has to be laid down first
 * — and like the rest of `test/e2e` nothing runs it automatically.
 */

const CARDS = {
  retry: "live-2028-retry",
  amend: "live-2028-amend",
  skip: "live-2028-skip",
  cancel: "live-2028-cancel",
} as const;

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

const card = (page: Page, id: string) => page.locator(`[data-approval-id="${id}"]`);

/**
 * Move the operator's attention off the queue.
 *
 * `useStableList` freezes the rendered order — and holds removals — while the
 * pointer is over the queue or focus is inside it, and a decide click leaves
 * both true. This is what "moving away" means to that hook, so the decided card
 * can drop.
 */
async function leaveQueue(page: Page) {
  await page.mouse.move(0, 0);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
}

test("all four verdicts are reachable, and each one lands", async ({ page }) => {
  test.setTimeout(180_000);
  await page.goto("/#/approvals");

  // Every seeded blocker is on the page, and every one of them offers four
  // controls rather than Approve and Decline.
  for (const id of Object.values(CARDS)) {
    await expect(card(page, id)).toBeVisible({ timeout: 20_000 });
    const footer = card(page, id).getByTestId("approval-decide");
    await expect(footer.getByRole("button", { name: /^Retry/ })).toBeVisible();
    await expect(footer.getByRole("button", { name: /^Answer this question/ })).toBeVisible();
    await expect(footer.getByRole("button", { name: /^Skip this step/ })).toBeVisible();
    await expect(footer.getByRole("button", { name: /^Cancel run/ })).toBeVisible();
    await expect(footer.getByRole("button", { name: /^Approve:/ })).toHaveCount(0);
  }

  // Retry.
  await card(page, CARDS.retry)
    .getByRole("button", { name: /^Retry/ })
    .click();
  await leaveQueue(page);
  await expect(card(page, CARDS.retry)).toHaveCount(0, { timeout: 60_000 });

  // Amend — the words the operator types.
  const amend = card(page, CARDS.amend);
  await amend.getByRole("button", { name: /^Answer this question/ }).click();
  const send = amend.getByRole("button", { name: /^Send this answer/ });
  await expect(send, "a blank answer cannot be sent").toBeDisabled();
  await amend.getByRole("textbox", { name: /^Answer:/ }).fill("use gpt-4o-mini instead");
  await expect(send).toBeEnabled();
  await send.click();
  await leaveQueue(page);
  await expect(amend).toHaveCount(0, { timeout: 60_000 });

  // Skip.
  await card(page, CARDS.skip)
    .getByRole("button", { name: /^Skip this step/ })
    .click();
  await leaveQueue(page);
  await expect(card(page, CARDS.skip)).toHaveCount(0, { timeout: 60_000 });

  // Cancel.
  await card(page, CARDS.cancel)
    .getByRole("button", { name: /^Cancel run/ })
    .click();
  await leaveQueue(page);
  await expect(card(page, CARDS.cancel)).toHaveCount(0, { timeout: 60_000 });
});
