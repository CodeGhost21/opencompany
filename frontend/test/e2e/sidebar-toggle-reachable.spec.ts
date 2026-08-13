import { expect, test } from "@playwright/test";

/** The tour can cover the fixed trigger while it is showing. */
async function dismissTour(page: import("@playwright/test").Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  try {
    await skip.waitFor({ state: "visible", timeout: 10_000 });
    await skip.click();
  } catch {
    // The signed-in browser profile may already have completed the tour.
  }
}

test.describe("sidebar toggle reachability", () => {
  test("the mobile sheet has an in-viewport way back", async ({ page }) => {
    await page.setViewportSize({ width: 700, height: 800 });
    await page.goto("/#/overview");
    await dismissTour(page);

    const trigger = page.getByRole("button", { name: "Toggle sidebar" });
    await expect(trigger).toBeInViewport();
    await trigger.click();
    await expect(page.getByText("Workflows", { exact: true })).toBeVisible();
  });

  test("the inline sidebar keeps its labelled collapse control", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 800 });
    await page.goto("/#/overview");
    await dismissTour(page);

    await expect(page.getByRole("button", { name: "Collapse" })).toBeInViewport();
  });
});
