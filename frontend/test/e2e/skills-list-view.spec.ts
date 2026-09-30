import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  amendSkills,
  hostSkills,
  installedCard,
  openSkills,
  suppressTour,
} from "./skills";

/**
 * The two states of the installed list a live host cannot be put into.
 *
 * Everything else in the Skills suite asserts against what the host serves.
 * These two cannot: a row carrying an available update needs the shared
 * library to move under a running process (see the note above
 * `skills-console.spec.ts`'s drift test), and the reach cell's overflow
 * control needs more teammates than the harness company has. Both are driven
 * here over the host's own answer with one field rewritten — `amendSkills`
 * says what that does and does not prove.
 *
 * What is being proved is client-side and worth proving: a predicate over the
 * rows, a count, a badge, and a control that only exists once the faces stop
 * fitting. The cards-or-rows choice itself needs no fixture, so it stays in
 * `skills-console.spec.ts` beside the rest of the list. The host's half — that `updateAvailable` is computed by comparing a
 * pinned digest against the library's current one — belongs to
 * `skill_provenance_tests.rs` and `graphql/skills_drift_tests.rs`.
 *
 * Default features are enough; every route here ships in the default build.
 */

/** A skill the harness company bundles, used as the row that has drifted. */
const NAME = "Meeting Brief";
const SLUG = "meeting-brief";

/** Picks `option` from the Base UI select at `testId`. */
async function choose(page: Page, testId: string, option: string) {
  await page.getByTestId(testId).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test.beforeEach(async ({ page }) => {
  await suppressTour(page);
});

// ---------------------------------------------------------------------------
// Filtering to the rows a library has moved under
// ---------------------------------------------------------------------------

test("the Updates filter keeps the row with an update and drops the rest", async ({
  page,
  request,
}) => {
  const served = await hostSkills(request);
  await amendSkills(page, (skills) =>
    skills.map((skill) =>
      skill.id === SLUG
        ? { ...skill, updateAvailable: { from: "1.0.0", to: "1.1.0" } }
        : skill,
    ),
  );
  await openSkills(page);

  const cards = page.getByTestId("installed-card");
  await expect(cards).toHaveCount(served.length, { timeout: 30_000 });
  // The badge is on the row before any filtering, which is what makes the
  // filter findable in the first place.
  await expect(
    installedCard(page, NAME).getByTestId("skill-update-available"),
  ).toBeVisible();

  await choose(page, "skills-filter-drift", "Has update");

  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText(NAME);
  await expect(page.getByTestId("skills-count")).toContainText(
    `1 of ${served.length} installed`,
  );

  await choose(page, "skills-filter-drift", "Any version");
  await expect(cards).toHaveCount(served.length);
});

test("a row the library moved under is still offered when it has also been edited", async ({
  page,
  request,
}) => {
  // The decision this pins: `"Has update"` collects every row the library has
  // moved under, including one whose own document was edited since. That row's
  // Update is refused and its badge says so — but it is precisely the row an
  // operator filtering for drift needs to make a decision about, so filtering
  // on "can this be applied" would hide it. A second row, edited with no
  // update behind it, must not be collected.
  const served = await hostSkills(request);
  const other = served.find((skill) => skill.id !== SLUG);
  expect(
    other,
    "the harness company should install more than one skill",
  ).toBeTruthy();

  await amendSkills(page, (skills) =>
    skills.map((skill) => {
      if (skill.id === SLUG) {
        return {
          ...skill,
          modified: true,
          updateAvailable: { from: "1.0.0", to: "1.1.0" },
        };
      }
      return skill.id === other!.id ? { ...skill, modified: true } : skill;
    }),
  );
  await openSkills(page);

  const cards = page.getByTestId("installed-card");
  await expect(cards).toHaveCount(served.length, { timeout: 30_000 });
  // Both rows read as modified, which is the badge that outranks the update.
  await expect(
    installedCard(page, NAME).getByTestId("skill-modified"),
  ).toBeVisible();
  await expect(
    installedCard(page, other!.name).getByTestId("skill-modified"),
  ).toBeVisible();

  await choose(page, "skills-filter-drift", "Has update");

  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText(NAME);
  // And the row menu still refuses the update it collected, naming the edit.
  await installedCard(page, NAME).getByTestId("skill-row-menu").click();
  await expect(page.getByTestId("skill-menu-update")).toBeDisabled();
});

// ---------------------------------------------------------------------------
// The reach cell when the faces stop fitting
// ---------------------------------------------------------------------------

/** Every holder the reach cell drew, counted, and the overflow's own number. */
async function reachCounts(cell: Locator) {
  return cell.evaluate((node) => {
    const text = (selector: string) =>
      node.querySelector(selector)?.textContent ?? "";
    return {
      held: Number(text('[data-testid="skill-reach-count"]').split(" of ")[0]),
      faces: node.querySelectorAll("img").length,
      hidden: Number(
        text('[data-testid="skill-reach-overflow"]').replace("+", ""),
      ),
    };
  });
}

test("a scope too wide for the cell counts the faces it hid and opens the page", async ({
  page,
  request,
}) => {
  // Enough holders that no column width fits them, so this does not depend on
  // how a card happens to squeeze at a given viewport.
  const EXTRA = 14;
  const served = await hostSkills(request);
  const bundled = served.find((skill) => skill.id === SLUG);
  const holders = (bundled?.agents ?? []).filter((agent) => agent.holds).length;
  expect(holders, `${SLUG} should reach somebody to widen`).toBeGreaterThan(0);

  const extra = Array.from({ length: EXTRA }, (_, i) => ({
    id: `overflow-probe-${i}`,
    state: "included",
    holds: true,
  }));

  await amendSkills(page, (skills) =>
    skills.map((skill) =>
      skill.id === SLUG
        ? { ...skill, agents: [...(skill.agents ?? []), ...extra] }
        : skill,
    ),
  );
  await openSkills(page);

  const card = installedCard(page, NAME);
  await expect(card).toBeVisible({ timeout: 30_000 });
  const reach = card.getByTestId("skill-reach");
  await expect(card.getByTestId("skill-reach-overflow")).toBeVisible();

  // Read in one pass and polled to settle: the cell measures itself with a
  // `ResizeObserver`, so separate reads can straddle a re-measure and disagree
  // with each other rather than with the component. The invariant is that the
  // faces drawn plus the ones counted are every holder — a `+N` that does not
  // add up is the bug this control can actually have.
  await expect
    .poll(async () => {
      const { held, faces, hidden } = await reachCounts(reach);
      return { held, accounted: faces + hidden, drewSome: faces > 0 };
    })
    .toEqual({
      held: holders + EXTRA,
      accounted: holders + EXTRA,
      drewSome: true,
    });

  // Some were hidden, or there was nothing for this control to say.
  const { faces, hidden } = await reachCounts(reach);
  expect(hidden, `${faces} faces drawn of ${holders + EXTRA}`).toBeGreaterThan(
    0,
  );

  // It is a way into the page, where every teammate is named rather than
  // counted.
  await card.getByTestId("skill-reach-overflow").click();
  await expect(page.getByTestId("skill-page")).toBeVisible();
  await expect(page.getByTestId("skill-detail-name")).toHaveText(NAME);
});
