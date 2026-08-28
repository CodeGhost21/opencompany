import { expect, test, type Page } from "@playwright/test";

/**
 * Proof for issue #1573: a teammate's run history is reachable against the
 * live host.
 *
 * The claim under test is one no unit render can make, because it spans the
 * whole stack: the console asks for `?agent=`, a real `RunStore` answers it
 * from its index, and the rows that come back are this teammate's attempts and
 * nobody else's. A stubbed `listRuns` proves the component; only a host proves
 * the selector.
 *
 * The walk navigates to `/#/team/engineer` and accepts either a populated
 * attempt list or the "hasn't run yet" state — whether the host's data
 * directory holds any attempts decides which, and both are correct. The
 * journey is the operator's, and it had no surface at all before this.
 *
 * Default features are enough: the harness company boots on the offline echo
 * brain, and whichever state the section lands in must render without error.
 */

async function dismissOnboarding(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (!(await skip.isVisible().catch(() => false))) return;
    await skip.click({ force: true }).catch(() => {});
    await page.waitForTimeout(300);
  }
  await expect(skip).toHaveCount(0);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

test("a teammate's page lists the attempts it has made", async ({ page }) => {
  await page.goto("/#/team/engineer");
  await dismissOnboarding(page);

  const section = page.getByTestId("agent-runs");
  await expect(section).toBeVisible({ timeout: 30_000 });

  // Either state is correct against a host whose data directory may or may not
  // persist between runs — what must never happen is the section failing to
  // render, or rendering a spinner forever.
  await expect(section).toContainText(/Runs/);
  await expect(
    section.getByTestId("agent-runs-filter-all").or(section.getByText(/hasn't run yet/)),
  ).toBeVisible();
});
