import { expect, test, type Locator, type Page } from "@playwright/test";

import { openWorkflow, workflowDetailName } from "./workflows";

/** The source-defined chain whose last node opens under the inspector. */
const FIXTURE = "Committed flow";

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

async function box(locator: Locator) {
  const result = await locator.boundingBox();
  expect(result, "element has no box").not.toBeNull();
  return result!;
}

async function openCanvas(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/#/workflows");
  await dismissTour(page);
  await openWorkflow(page, FIXTURE);
  await expect(page.locator(".react-flow__node").first()).toBeVisible({
    timeout: 30_000,
  });
}

async function rightmostNode(page: Page) {
  const nodes = page.locator(".react-flow__node");
  const count = await nodes.count();
  let rightmost: Locator | null = null;
  let rightEdge = -Infinity;
  for (let index = 0; index < count; index++) {
    const node = nodes.nth(index);
    const nodeBox = await box(node);
    if (nodeBox.x + nodeBox.width > rightEdge) {
      rightmost = node;
      rightEdge = nodeBox.x + nodeBox.width;
    }
  }
  expect(rightmost, "the canvas rendered no nodes").not.toBeNull();
  return rightmost!;
}

test("Escape closes the node inspector and restores the canvas", async ({ page }) => {
  await openCanvas(page);
  const node = await rightmostNode(page);
  const before = await box(node);

  await node.click();
  const inspector = page.getByTestId("workflow-node-detail");
  await expect(inspector).toBeVisible();
  await expect(async () => {
    expect((await box(node)).x).toBeLessThan(before.x - 1);
  }).toPass({ timeout: 5_000 });

  await page.keyboard.press("Escape");
  await expect(inspector).toBeHidden();
  await expect(async () => {
    const restored = await box(node);
    expect(Math.abs(restored.x - before.x)).toBeLessThan(2);
    expect(Math.abs(restored.y - before.y)).toBeLessThan(2);
  }).toPass({ timeout: 5_000 });
});

test("Escape closes the copilot and does nothing once canvas overlays are gone", async ({
  page,
}) => {
  await openCanvas(page);

  await page.getByTestId("workflow-copilot-toggle").click();
  const copilot = page.getByTestId("workflow-copilot");
  await expect(copilot).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(copilot).toBeHidden();
  await expect(workflowDetailName(page)).toHaveText(FIXTURE);

  await page.keyboard.press("Escape");
  await expect(workflowDetailName(page)).toHaveText(FIXTURE);
  await expect(page.getByTestId("workflow-node-detail")).toBeHidden();
});
