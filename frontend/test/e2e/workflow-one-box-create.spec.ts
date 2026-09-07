import { expect, test, type Page } from "@playwright/test";

import { workflowDetailName } from "./workflows";
import {
  COMPANY_SCOPE,
  draftedGraph,
  openOneBox,
  removeWorkflow,
  stubCognition,
  stubCreateFailure,
  stubDraft,
} from "./workflow-dialog";

/**
 * Creating a workflow: one box, then the canvas.
 *
 * The New-workflow dialog is a sentence and a Create button — no Name, no
 * Workflow ID, no Nodes, no Connections. `workflow-one-box-dialog.test.ts`
 * proves the branch reaches the DOM; this file proves the two things a rendered
 * unit test cannot:
 *
 * - the graph the console posts is one the **real host stores**, on both routes
 *   the box has (a copilot draft, and the fallback that names the workflow from
 *   the operator's own sentence); and
 * - a create that fails for a reason nobody can act on **leaves the box up**.
 *   That one is the redesign's own failure mode: the hand-over to the graph form
 *   is a one-way door, so a 500 that triggered it would silently undo the whole
 *   change for the rest of the open — and it did, until this was pinned.
 *
 * ## What is stubbed, and what is not
 *
 * The cognition read (`GET …/inference`) is stubbed, because the two lanes this
 * suite runs in (`playwright.config.ts`) answer it differently and BOTH answers
 * are behaviours worth pinning — a spec that inherited the lane's would test one
 * of them, twice, and say nothing about which. The copilot's own answer is
 * stubbed for the same reason a model's output always is.
 *
 * The **write is never stubbed**. Every creation below is stored by the host and
 * read back over the API, because "the console posted a graph the host accepts"
 * is exactly the half a stub cannot tell you — and it is the half that was
 * broken when a drafted graph was round-tripped through a form that had no
 * control for half its fields.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

const SUBMIT = "workflow-dialog-submit";

/**
 * Presses the permanent-id confirm (issue #1808).
 *
 * The confirm is portalled onto `document.body`, so it is reached from the page
 * rather than scoped to the dialog — a `dialog`-scoped locator misses it, which
 * reads as "the confirm never opened".
 */
async function confirmId(page: Page, expected: string) {
  await expect(
    page.getByTestId("workflow-id-confirm-value"),
    "a console-derived permanent id must be said out loud before it is permanent",
  ).toHaveText(expected);
  await page.getByTestId("workflow-id-confirm-create").click();
}

test("the box drafts, saves, and lands on the canvas — one gesture", async ({
  page,
  request,
}) => {
  const id = `e2e-onebox-${Date.now()}`;
  const graph = draftedGraph(id, `One box probe ${Date.now()}`);
  const drafts = { count: 0 };

  try {
    await stubCognition(page, "hosted");
    await stubDraft(page, { automatable: true, summary: "a digest", workflow: graph }, drafts);

    const dialog = await openOneBox(page);
    // The whole dialog: no Name, no Workflow ID, no node rows to fill in.
    await expect(dialog.getByLabel("Workflow ID", { exact: true })).toHaveCount(0);
    await expect(dialog.getByLabel("Name", { exact: true })).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Add node" })).toHaveCount(0);

    await dialog
      .getByTestId("workflow-describe-box")
      .fill("Every Monday, have the writer draft the weekly digest.");
    await dialog.getByTestId(SUBMIT).click();

    // No id confirm on this route: the id is the host's, chosen with a graph the
    // operator asked for and is about to look at. Putting it in front of them
    // here would reinstate the field this redesign removed.
    await expect(dialog).toBeHidden({ timeout: 30_000 });
    expect(drafts.count, "Create drafts once, not per keystroke").toBe(1);

    // The canvas the create lands on IS this workflow's.
    await expect(page).toHaveURL(new RegExp(`#/workflows/${id}$`), { timeout: 30_000 });
    await expect(workflowDetailName(page)).toHaveText(graph.name, { timeout: 30_000 });

    // And the host stored the drafted graph verbatim — nodes and edges, not the
    // blank starter, and not a graph round-tripped through a form that never
    // rendered.
    const saved = await request.get(`${COMPANY_SCOPE}/workflows/${id}`);
    expect(saved.ok(), `the host stored the drafted graph: ${saved.status()}`).toBeTruthy();
    const stored = await saved.json();
    expect(stored.nodes.map((n: { id: string }) => n.id)).toEqual(["start", "report"]);
    expect(stored.edges).toHaveLength(1);
  } finally {
    await removeWorkflow(request, id);
  }
});

test("with no copilot, the box still creates — naming the workflow from the sentence", async ({
  page,
  request,
}) => {
  // The reversal this PR is about. A company with no model configured used to
  // get the full graph form: the dialog the redesign exists to retire, handed to
  // the operator least likely to want to hand-author a graph. It now gets the
  // same one box, and Create builds the workflow from what they wrote.
  const stamp = Date.now();
  const id = `chase-overdue-invoices-${stamp}`;
  const drafts = { count: 0 };

  try {
    await stubCognition(page, "echo");
    // Registered but never expected to fire: a company with no model is asked
    // nothing, and "it did not ask a host that cannot answer" is a claim only a
    // counter can make.
    await stubDraft(page, { automatable: false, reason: "unreachable" }, drafts);

    const dialog = await openOneBox(page);

    // What Create will do is said before it is pressed — no promise of a draft
    // that cannot happen.
    const notice = dialog.getByTestId("workflow-draft-unavailable");
    await expect(notice).toBeVisible();
    await expect(notice).toContainText("no model configured");
    await expect(notice).toContainText("empty canvas");

    await dialog
      .getByTestId("workflow-describe-box")
      .fill(`Chase overdue invoices ${stamp}, every Friday, and nudge the client.`);
    await dialog.getByTestId(SUBMIT).click();

    // Issue #1808: this id is slugged from a clause the operator wrote as prose,
    // and the box has no field that would ever show it to them. So it is said
    // once, before it becomes the permanent join key for the schedule and every
    // run this workflow ever has.
    await confirmId(page, id);

    await expect(dialog).toBeHidden({ timeout: 30_000 });
    expect(drafts.count, "the copilot must not be asked on the offline brain").toBe(0);

    await expect(page).toHaveURL(new RegExp(`#/workflows/${id}$`), { timeout: 30_000 });

    // The host stored it: named from the sentence, described by the whole
    // sentence, over the single trigger the blank form has always started from.
    const saved = await request.get(`${COMPANY_SCOPE}/workflows/${id}`);
    expect(saved.ok(), `the host stored the derived graph: ${saved.status()}`).toBeTruthy();
    const stored = await saved.json();
    expect(stored.name).toBe(`Chase overdue invoices ${stamp}`);
    expect(stored.description).toBe(
      `Chase overdue invoices ${stamp}, every Friday, and nudge the client.`,
    );
    expect(stored.nodes.map((n: { kind: string }) => n.kind)).toEqual(["trigger"]);
  } finally {
    await removeWorkflow(request, id);
  }
});

test("a write that fails for a reason nobody can act on leaves the box up", async ({
  page,
}) => {
  // The hand-over to the graph form is a **one-way door** — it retires the box
  // for the rest of the open. It used to fire from the first line of the write
  // path's error handling, so a 500 or a dropped connection collapsed the
  // redesign into the very dialog it replaced, permanently, over something that
  // would have worked on the next press.
  const graph = draftedGraph("e2e-blip", "Blip probe");
  const drafts = { count: 0 };

  await stubCognition(page, "hosted");
  await stubDraft(page, { automatable: true, summary: "a digest", workflow: graph }, drafts);
  await stubCreateFailure(page, 500, {
    error: "the host fell over on the way to saving it",
    code: "internal",
  });

  const dialog = await openOneBox(page);
  const sentence = "Every Monday, have the writer draft the weekly digest.";
  await dialog.getByTestId("workflow-describe-box").fill(sentence);
  await dialog.getByTestId(SUBMIT).click();

  // The failure is reported — Create never reads as a button that did nothing.
  await expect(dialog.getByTestId("create-error")).toContainText("fell over", {
    timeout: 30_000,
  });

  // …and the box is still the whole dialog, with the sentence still in it.
  await expect(dialog.getByTestId("workflow-describe-box")).toHaveValue(sentence);
  await expect(
    dialog.getByLabel("Workflow ID", { exact: true }),
    "a 500 must not hand over the graph form",
  ).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Add node" })).toHaveCount(0);
});

test("a copilot that could not draft says so, rather than advising it be done by hand", async ({
  page,
}) => {
  // `automatable: false` is one flag over two very different events. A draft the
  // copilot could not get past the host's own gates used to arrive as advice,
  // verbatim — which meant an operator was shown "a workflow needs exactly one
  // `trigger` node (found 0)" by the one dialog built to stop mentioning nodes,
  // under a button offering to overrule an opinion nobody had expressed.
  const drafts = { count: 0 };
  await stubCognition(page, "hosted");
  await stubDraft(
    page,
    {
      automatable: false,
      reason:
        "the described workflow could not be drafted into one that would be accepted: " +
        "invalid request: a workflow needs exactly one `trigger` node to say what " +
        "starts it (found 0).",
    },
    drafts,
  );

  const dialog = await openOneBox(page);
  await dialog
    .getByTestId("workflow-describe-box")
    .fill("Every Friday, email the sales digest and file it in Dropbox.");
  await dialog.getByTestId(SUBMIT).click();

  const declined = dialog.getByTestId("workflow-draft-declined");
  await expect(declined).toBeVisible({ timeout: 30_000 });
  await expect(declined).toHaveAttribute("data-decline-kind", "failure");
  // The vocabulary the one box exists to retire never reaches the operator.
  await expect(declined).not.toContainText("trigger");
  await expect(declined).not.toContainText("invalid request");
  await expect(declined).toContainText("could not turn that into a workflow");
  // And the offered action is the canvas, not the overruling of a judgement.
  await expect(dialog.getByTestId("workflow-create-anyway")).toContainText(
    "Start it on the canvas",
  );
});
