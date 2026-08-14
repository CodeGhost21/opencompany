import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

const COMPANY_SCOPE = "/api/v1/company";

/** Dismisses the first-run tour if it is still visible. */
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

/** A minimal valid graph body: one trigger, one output, one edge. */
function graphBody(id: string, name: string) {
  return {
    id,
    name,
    description: "Created by the #864 e2e selection persistence spec.",
    nodes: [
      { id: "start", kind: "trigger", name: "Start" },
      { id: "done", kind: "output", name: "Done" },
    ],
    edges: [{ from: "start", to: "done" }],
  };
}

async function createWorkflow(request: APIRequestContext, id: string, name: string) {
  const res = await request.post(`${COMPANY_SCOPE}/workflows`, { data: graphBody(id, name) });
  expect(res.ok(), `create ${id}: ${res.status()} ${await res.text()}`).toBeTruthy();
}

async function deleteWorkflow(request: APIRequestContext, id: string) {
  await request.delete(`${COMPANY_SCOPE}/workflows/${id}`).catch(() => undefined);
}

/** The workflow picker's trigger. */
function picker(page: Page) {
  return page.getByRole("combobox").first();
}

async function selectWorkflow(page: Page, name: string) {
  await picker(page).click();
  await page.getByRole("option", { name, exact: true }).click();
  await expect(picker(page)).toContainText(name);
}

async function openWorkflows(page: Page) {
  await page.goto("/#/workflows");
  await dismissTour(page);
  await expect(picker(page)).toBeEnabled({ timeout: 30_000 });
}

test("workflows tab selection is preserved across tab switches (#864)", async ({ page, request }) => {
  const stamp = Date.now();
  const firstId = `e2e-864-first-${stamp}`;
  const secondId = `e2e-864-second-${stamp}`;
  const firstName = "Workflow selector probe A";
  const secondName = "Workflow selector probe B";

  try {
    await createWorkflow(request, firstId, firstName);
    await createWorkflow(request, secondId, secondName);

    await openWorkflows(page);
    await selectWorkflow(page, secondName);
    await expect(picker(page)).toContainText(secondName);

    await page.getByRole("button", { name: "Workspace" }).click();
    await page.getByRole("button", { name: "Workflows" }).click();
    await expect(picker(page)).toContainText(secondName);

    await page.goto(`/#/workflows/${firstId}`);
    await expect(picker(page)).toContainText(firstName);

    await page.getByRole("button", { name: "Workspace" }).click();
    await page.getByRole("button", { name: "Workflows" }).click();
    await expect(picker(page)).toContainText(firstName);
  } finally {
    await deleteWorkflow(request, firstId);
    await deleteWorkflow(request, secondId);
  }
});
