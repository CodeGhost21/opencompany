import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

/**
 * Issue #824: the workflows toolbar laid out wider than its container and
 * overflowed into an `overflow-hidden` ancestor, so its last control —
 * **New workflow** — was clipped off the right edge. Nothing in that chain
 * scrolls, so the button was not merely off-screen: it could not be clicked.
 *
 * This is a **layout** defect, so it is only observable in a browser. jsdom
 * computes no geometry, and a unit test asserting the class string would pass
 * against any width — including a row that still overflows. The measurement has
 * to be a real one.
 *
 * The row grows every time a control is added. `Pause` (#814) took the overhang
 * from 22px to 113px, which is what made it visible, but the row was already
 * overflowing before it. So the spec asserts the **property** rather than
 * today's button count: every control in the toolbar is inside the viewport and
 * clickable. A tenth control that reintroduces the overflow fails here.
 *
 * Runs at two widths. 1280 is the common laptop width the defect was reported
 * at; 1024 is the narrow end, where a fix that merely bought a few pixels would
 * still fail.
 */

const COMPANY_SCOPE = "/api/v1/company";
const WORKFLOW_ID = "e2e-824-toolbar";

/** The tour's overlay swallows pointer events; tolerate its absence. */
async function dismissTour(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  try {
    await skip.waitFor({ state: "visible", timeout: 10_000 });
  } catch {
    return;
  }
  await skip.click();
  await expect(skip).toBeHidden();
}

async function createWorkflow(request: APIRequestContext) {
  const res = await request.post(`${COMPANY_SCOPE}/workflows`, {
    data: {
      id: WORKFLOW_ID,
      name: "Toolbar reachability",
      description: "Created by the #824 e2e spec.",
      nodes: [
        { id: "start", kind: "trigger", name: "Start", schedule: "0 9 * * *" },
        { id: "done", kind: "output", name: "Report" },
      ],
      edges: [{ from: "start", to: "done" }],
    },
  });
  expect(res.ok(), `create: ${res.status()} ${await res.text()}`).toBeTruthy();
}

async function removeWorkflow(request: APIRequestContext) {
  await request.delete(`${COMPANY_SCOPE}/workflows/${WORKFLOW_ID}`).catch(() => undefined);
}

/**
 * The toolbar's controls. A workflow must be **selected** for the full set to
 * mount — Pause/Edit/Delete are per-workflow — which is the state the defect
 * appears in and the reason this spec creates one.
 */
const CONTROLS = [
  "Run",
  "Test run",
  "Browse",
  "Copilot",
  "History",
  "Edit",
  "Delete",
  "New workflow",
];

test.describe("workflows toolbar reachability (#824)", () => {
  test.beforeEach(async ({ request }) => {
    await removeWorkflow(request);
    await createWorkflow(request);
  });

  test.afterEach(async ({ request }) => {
    await removeWorkflow(request);
  });

  for (const width of [1280, 1024]) {
    test(`every toolbar control is inside the viewport at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.goto("/#/workflows");
      await dismissTour(page);

      // Wait for the toolbar to mount before measuring anything.
      const newWorkflow = page.getByRole("button", { name: "New workflow" });
      await expect(newWorkflow).toBeVisible();

      for (const name of CONTROLS) {
        const control = page.getByRole("button", { name, exact: false }).first();
        await expect(control, `${name} should be mounted`).toBeVisible();
        // The assertion that matters. `toBeVisible` is true for a button that
        // has been pushed past the right edge — it is painted, just not
        // anywhere reachable. `toBeInViewport` is what distinguishes the two.
        await expect(control, `${name} should be reachable at ${width}px`).toBeInViewport();
      }
    });
  }

  test("New workflow can actually be clicked, not merely rendered", async ({ page }) => {
    // The defect's real cost. Every control above could be in the viewport and
    // this could still fail if something overlapped it, so the last word is an
    // actual click with its actual consequence.
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/#/workflows");
    await dismissTour(page);

    await page.getByRole("button", { name: "New workflow" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page.getByText("Describe the workflow")).toBeVisible();
  });
});
