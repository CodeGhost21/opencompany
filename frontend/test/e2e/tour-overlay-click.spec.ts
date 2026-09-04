import { expect, test, type Page } from "@playwright/test";

/**
 * A click on the dimmed page during the tour must not navigate.
 *
 * # The failure this reproduces
 *
 * react-joyride draws a full-page overlay with a cutout around the current
 * stop's target, and its `overlayClickAction` defaults to `close` — which under
 * `continuous` advances the run. So a click on anything outside the cutout hit
 * the overlay rather than the element under it: the element's own handler never
 * ran, the tour stepped forward, and `TourController`'s `before` hook drove the
 * console to the next stop's view. The first stop is the Room, so an operator
 * clicking a teammate on Company landed in `#/chat/main` with no teammate open.
 *
 * Captured from the running console on the parent commit — the click's own
 * `composedPath` was:
 *
 *     path → svg[spotlight] → div.react-joyride__overlay → …
 *
 * It was reported twice, as a teammate card and as a workflow card, which is
 * what one cause looks like from outside; the cards were never involved. Every
 * clickable surface reached during a tour behaved this way, and a first-time
 * operator meets the tour on their first visit to a company.
 *
 * # Why this is only a browser test
 *
 * The fix is one option on a prop, and a unit test asserting that option is set
 * would be the same literal written twice — it would pass on a build where the
 * prop never reached joyride. What must be true is that joyride *receives* the
 * policy and stops treating its backdrop as a control, which only a real
 * overlay in a real browser can answer. Same split `tour-completion-persists`
 * describes for issue #1408.
 *
 * # Why the precondition assertion is not optional
 *
 * The first version of this spec passed against the broken build. The overlay's
 * cutout moves with the stop's target and the roster reflows with the viewport,
 * so at a smaller viewport the click landed on the page rather than on the
 * overlay and the spec asserted nothing at all — green, for the wrong reason.
 * `expectOverlayCovers` pins the condition the test needs to be exercising, so
 * a layout change turns this into a failure rather than a silent pass. The
 * viewport is fixed here for the same reason.
 */

test.use({ viewport: { width: 1400, height: 900 } });

const TOUR_STOPS = 7;
const WELCOME = "Welcome to your company";
const TEAMMATE = "Chief Executive";

function card(page: Page) {
  return page.getByTestId("team-card").filter({ hasText: TEAMMATE }).first();
}

function opener(page: Page) {
  return card(page).getByTestId("team-card-open");
}

/** The Company roster, rendered and ready to be clicked. */
async function goToRoster(page: Page): Promise<void> {
  // By address rather than by clicking the nav: during a tour the nav is under
  // the overlay, which is the very thing under test.
  await page.goto("/#/company");
  await expect(opener(page)).toBeVisible({ timeout: 30_000 });
}

/**
 * Assert joyride's overlay really is the topmost thing over the teammate card.
 *
 * Without this the click below can land on the page and prove nothing.
 */
async function expectOverlayCovers(page: Page): Promise<void> {
  const box = await opener(page).boundingBox();
  expect(box, "the card should have a box to click").not.toBeNull();
  const covered = await page.evaluate(
    ({ x, y }) => {
      const at = document.elementFromPoint(x, y);
      return {
        overlay: Boolean(at?.closest(".react-joyride__overlay")),
        actual: at ? at.tagName : "nothing",
      };
    },
    { x: box!.x + box!.width / 2, y: box!.y + box!.height / 2 },
  );
  expect(
    covered.overlay,
    `the tour overlay must be over the card for this spec to exercise anything (found ${covered.actual})`,
  ).toBe(true);
}

test("a click on the dimmed page neither moves the tour nor changes the address", async ({
  page,
}) => {
  await page.goto("/#/company");
  await page.getByRole("button", { name: "Take the tour" }).click();

  const tooltip = page.getByRole("alertdialog");
  await expect(tooltip.getByText(`Step 1 of ${TOUR_STOPS}`)).toBeVisible();

  await goToRoster(page);
  await expectOverlayCovers(page);

  // `force` because the overlay is genuinely covering the card — that is the
  // condition under test. The event still lands on whatever is topmost at those
  // coordinates, which the assertion above has just proven is the overlay.
  await opener(page).click({ force: true });

  // Give a navigation the chance to happen before concluding none did.
  await page.waitForTimeout(750);

  await expect(page, "the overlay must not carry the click into the Room").not.toHaveURL(
    /#\/chat\/main$/,
  );
  await expect(page, "the address must not move at all").toHaveURL(/#\/company$/);
  await expect(
    tooltip.getByText(`Step 1 of ${TOUR_STOPS}`),
    "and the tour must not have stepped",
  ).toBeVisible();
});

test("the tooltip's own Next still advances the tour", async ({ page }) => {
  await page.goto("/#/company");
  await page.getByRole("button", { name: "Take the tour" }).click();

  const tooltip = page.getByRole("alertdialog");
  await expect(tooltip.getByText(`Step 1 of ${TOUR_STOPS}`)).toBeVisible();
  await page.waitForTimeout(250);
  await tooltip.locator("button", { hasText: "Next" }).click();

  await expect(
    tooltip.getByText(`Step 2 of ${TOUR_STOPS}`),
    "refusing the overlay's click must not disarm the tour's own controls",
  ).toBeVisible();
});

test("with the tour dismissed the same card still opens the teammate", async ({ page }) => {
  await page.goto("/#/company");
  await page.getByText(WELCOME).waitFor();
  await page.getByRole("button", { name: "Skip for now" }).click();

  await goToRoster(page);
  await opener(page).click();

  await expect(page).toHaveURL(/#\/team\/ceo$/);
  await expect(page.getByTestId("agent-name")).toHaveText(TEAMMATE);
});
