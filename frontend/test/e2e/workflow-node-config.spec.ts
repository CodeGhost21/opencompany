import { expect, test, type APIRequestContext } from "@playwright/test";

import { workflowDetailName } from "./workflows";
import { COMPANY_SCOPE, dismissTour, openEditForm, removeWorkflow } from "./workflow-dialog";

/**
 * Issue #541: the five withheld node kinds (`tool_call`, `http_request`,
 * `switch`, `output_parser`, `sub_workflow`) grew config forms in the workflow
 * creator. The engine has always run them; only the console lacked controls, so
 * an operator could not author (say) a tool call from the UI.
 *
 * This spec covers the half only a browser proves: authoring a `tool_call`
 * node through the dialog's config form, saving it, and confirming the config
 * the form emitted actually round-tripped through the host — reopened from the
 * saved graph and shown in the node inspector. A unit test pins the string
 * transforms (`workflow-node-config.test.ts`); this pins that the form is wired
 * to them and that the host stored what it produced.
 *
 * It used to author the node on the create form, which no longer exists as a
 * route: creating a workflow is one description box. The claim under test is
 * about the config form and the host, not about which write carries it, so it
 * is authored on the same form reached the way an operator reaches it now — by
 * editing (`test/e2e/workflow-dialog.ts`). What is lost by moving is nothing
 * this spec was ever asserting; what would have been lost by deleting it is the
 * only browser coverage `tool_call` config has.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

const SUBMIT = "workflow-dialog-submit";

/** Reads a workflow's stored graph back from the host. */
async function readGraph(request: APIRequestContext, id: string) {
  const res = await request.get(`${COMPANY_SCOPE}/workflows/${id}`);
  if (!res.ok()) return null;
  return res.json();
}

test("authoring a tool_call node's config through the form round-trips to the host", async ({
  page,
  request,
}) => {
  const stamp = Date.now();
  const id = `e2e-cfg-${stamp}`;
  const name = `Config probe ${stamp}`;

  try {
    // The starter is a lone trigger, exactly what the create form opened on, so
    // the row added below is still index 1.
    const dialog = await openEditForm(page, request, id, name);

    // Add a second node and make it a tool call — its config form appears only
    // once the kind is chosen.
    await dialog.getByRole("button", { name: "Add node" }).click();
    const nodeId = dialog.getByRole("textbox", { name: "Node id" }).nth(1);
    await nodeId.fill("act");

    // Change the second row's kind to Tool call.
    await dialog.getByRole("combobox", { name: "Node kind" }).nth(1).click();
    await page.getByRole("option", { name: /Tool call/ }).click();

    await dialog.getByRole("textbox", { name: "Node name" }).nth(1).fill("Do it");

    // The config form is now mounted. Fill the engine-exact keys.
    await dialog.getByRole("textbox", { name: "Tool slug", exact: true }).fill("web_fetch");
    await dialog
      .getByRole("textbox", { name: "Arguments", exact: true })
      .fill('{ "url": "https://example.com" }');

    // Connect trigger → tool call so the graph is valid.
    await dialog.getByRole("button", { name: "Add edge" }).click();
    await dialog.getByRole("combobox", { name: "Edge from" }).click();
    await page.getByRole("option", { name: "start", exact: true }).click();
    await dialog.getByRole("combobox", { name: "Edge to" }).click();
    await page.getByRole("option", { name: "act", exact: true }).click();

    // Edit mode writes with no id confirm (issue #1808 gates create only): the
    // id keys the saved graph and cannot change, so there is nothing to confirm.
    await dialog.getByTestId(SUBMIT).click();
    await expect(dialog).toBeHidden({ timeout: 15_000 });

    // The host stored exactly the keys the form emitted — not the node-id
    // fallback, and with the args object intact.
    await expect
      .poll(
        async () => {
          const graph = await readGraph(request, id);
          if (!graph) return null;
          return graph.nodes.find((n: { id: string }) => n.id === "act")?.config ?? null;
        },
        { timeout: 15_000 },
      )
      .toEqual({ slug: "web_fetch", args: { url: "https://example.com" } });

    // Reopen from the saved graph (a fresh load, not local state) and click the
    // node: the inspector's Config block shows the slug the host round-tripped.
    // Issue #1110: the detail view lives at the workflow's own URL, so the
    // reload comes back on it with no picking to do — which is also this spec's
    // incidental proof that a `#/workflows/<id>` survives a reload.
    await expect(page).toHaveURL(new RegExp(`#/workflows/${id}$`));
    await page.reload();
    await dismissTour(page);
    await expect(workflowDetailName(page)).toHaveText(name, { timeout: 30_000 });

    const node = page.locator('.react-flow__node[data-id="act"]');
    await expect(node).toBeVisible({ timeout: 15_000 });
    await node.click();

    await expect(page.getByText("Config", { exact: true })).toBeVisible();
    await expect(page.getByText(/"slug": "web_fetch"/)).toBeVisible();
  } finally {
    await removeWorkflow(request, id);
  }
});
