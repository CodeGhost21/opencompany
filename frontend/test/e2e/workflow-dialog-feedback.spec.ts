import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";

import {
  draftedGraph,
  openEditForm,
  openRefusedCreateForm,
  removeWorkflow,
} from "./workflow-dialog";

/**
 * Issues #260 / #261 / #262: how the workflow graph form validates input and
 * gives feedback.
 *
 * These three are one behaviour seen from three sides — a rule stated once
 * (#260), stated *early* (#261), and stated back in plain English (#262) — so
 * they are pinned together, in the browser, against the live host.
 *
 * The browser is the only place two of these are real. Blur validation is a
 * DOM event and a render, not a function call: the unit-testable part
 * (`destinationTargetProblem`) was already correct before #261 and would stay
 * green while the error rendered nowhere, or rendered under a control the
 * author can no longer see. And the cron preview's whole job is to show a
 * *round trip* — debounce, request, response, two clock renderings — which
 * cannot be observed anywhere else.
 *
 * ## Where the form went
 *
 * Every test here used to open **New workflow** and start filling fields in.
 * Creating a workflow is now one description box, so that route reaches a dialog
 * with none of these controls on it — and this file was the only coverage the
 * graph form's feedback had.
 *
 * It is re-routed rather than deleted, along the two routes the form actually
 * has (`test/e2e/workflow-dialog.ts`), and which route each test takes is a
 * decision about what the test is *about*:
 *
 * - **an edit**, for everything that is a fact about the form's controls. There
 *   is one implementation of them — `createSurface` chooses when the form
 *   renders, never what it is — so a blur error, a kind swap, a cron preview and
 *   the host pre-flight are the same events either side of it. This is also the
 *   honest route: editing is how an operator meets these controls now.
 * - **a create the host refused**, for the three that are facts about *create
 *   mode* — an id that can still be typed, the created-paused disclosure, and
 *   `validate()`'s own complaints. That route is not a workaround for reaching
 *   them: it is the one route the create form has left, and a browser is the
 *   only place its existence can be proved.
 *
 * What is NOT here any more is "a valid workflow still saves" as a *create*. The
 * create write is pinned in `workflow-one-box-create.spec.ts`, through the box
 * that now performs it; the save write is pinned below by the pre-flight test,
 * which ends by saving the graph the host approved and reading it back.
 *
 * Runs against the live host the harness brings up (see `playwright.config.ts`).
 */

/**
 * Every cron-preview assertion waits on a round trip the other assertions in
 * this file do not: a 350 ms debounce, then a request to the host, then a
 * render. Playwright's default `expect` timeout is 5 s and the config's
 * `timeout` governs the whole test, not each assertion — so on a slow host
 * these are the assertions that flake first. Stated explicitly rather than
 * inherited.
 */
const PREVIEW_TIMEOUT = 15_000;

const SUBMIT = "workflow-dialog-submit";

/** A fresh id/name pair, so a re-run against the same company cannot collide. */
function stamped(slug: string): { id: string; name: string } {
  const stamp = Date.now();
  return { id: `e2e-${slug}-${stamp}`, name: `${slug} probe ${stamp}` };
}

/**
 * Opens the graph form on a freshly created one-trigger workflow.
 *
 * The lone trigger matters: the create form opened on exactly that, so every
 * `.nth(1)` below still means "the row this test added".
 */
async function formOnStarter(
  page: Page,
  request: APIRequestContext,
  slug: string,
): Promise<{ dialog: Locator; id: string }> {
  const { id, name } = stamped(slug);
  const dialog = await openEditForm(page, request, id, name);
  return { dialog, id };
}

/**
 * Turns the form's second node row into an `output` node routing to `email`,
 * and returns its target Input.
 */
async function emailOutputRow(page: Page, dialog: Locator) {
  await dialog.getByRole("button", { name: "Add node" }).click();

  const kind = dialog.getByLabel("Node kind").nth(1);
  await kind.click();
  await page.getByRole("option", { name: /^Output(?! parser)/ }).click();

  const destination = dialog.getByLabel("Send report to");
  await destination.click();
  await page.getByRole("option", { name: /^Email/ }).click();

  return dialog.getByLabel("Recipient address");
}

test("a bad email target is reported on blur, and typing again clears it", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "blur");
  try {
    const target = await emailOutputRow(page, dialog);

    // Typing alone must NOT complain: `n` is not a valid address, and neither is
    // `no`, so an author gets an error on the way to typing a correct one. #261
    // rejects keystroke validation explicitly.
    await target.fill("nope");
    await expect(dialog.getByText(/is not an email address/)).toHaveCount(0);

    // Blur is the moment the author has finished with the field.
    await target.blur();
    const error = dialog.getByText(/is not an email address/);
    await expect(error).toBeVisible();
    // #260: the message ends with the SAME fix instruction the host's 400 ends
    // with, and echoes the offending target.
    await expect(error).toContainText("`nope`");
    await expect(error).toContainText("give the recipient's full address.");
    await expect(target).toHaveAttribute("aria-invalid", "true");

    // Typing again clears it — the author is already fixing it.
    await target.fill("ada@example.com");
    await expect(error).toHaveCount(0);
    await expect(target).not.toHaveAttribute("aria-invalid", "true");

    // A now-valid address stays quiet on blur.
    await target.blur();
    await expect(dialog.getByText(/is not an email address/)).toHaveCount(0);
  } finally {
    await removeWorkflow(request, id);
  }
});

test("an empty field is never nagged on blur — that stays submit's business", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "emptyblur");
  try {
    const target = await emailOutputRow(page, dialog);

    // Tabbing straight through a fresh field must say nothing. "You haven't
    // filled this in yet" is true of every field an author passes on the way
    // somewhere else.
    await target.click();
    await target.blur();
    await expect(dialog.getByText(/is not an email address/)).toHaveCount(0);
  } finally {
    await removeWorkflow(request, id);
  }
});

test("changing a node's kind clears the field AND its error, leaving no orphan", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "kindswap");
  try {
    const target = await emailOutputRow(page, dialog);

    await target.fill("nope");
    await target.blur();
    await expect(dialog.getByText(/is not an email address/)).toBeVisible();

    // Switching the kind un-renders the destination controls (PR #226's
    // `changeKind` reset). The error must go with them: an error about a field
    // with no control left on screen is one the author cannot clear.
    const kind = dialog.getByLabel("Node kind").nth(1);
    await kind.click();
    await page.getByRole("option", { name: /^Agent/ }).click();

    await expect(dialog.getByLabel("Recipient address")).toHaveCount(0);
    await expect(dialog.getByText(/is not an email address/)).toHaveCount(0);

    // Switching back gives a clean field, not a resurrected value or error.
    await kind.click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();
    await expect(dialog.getByLabel("Send report to")).toBeVisible();
    await expect(dialog.getByText(/is not an email address/)).toHaveCount(0);
  } finally {
    await removeWorkflow(request, id);
  }
});

/**
 * The cron preview line's parser complaint, and only it.
 *
 * `CronPreviewLine` renders into a `<p>`; the host pre-flight renders into an
 * alert. Both quote the parser, so the element is part of the assertion.
 */
function cronPreview(dialog: Locator): Locator {
  return dialog.locator("p", { hasText: /value out of range/ });
}

/** Opens the trigger row's custom-cron input. */
async function customCronInput(page: Page, dialog: Locator) {
  await dialog.getByLabel("Schedule").click();
  await page.getByRole("option", { name: "Custom cron…" }).click();
  return dialog.getByLabel("Custom cron schedule");
}

test("a cron expression is read back in plain English, in UTC and local time", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "cron");
  try {
    const cron = await customCronInput(page, dialog);

    // **The issue #262 case.** `0 9 * * *` and `9 0 * * *` are two characters
    // apart, both valid, and nine hours apart in meaning. Only the preview tells
    // them apart before the report lands at the wrong time.
    await cron.fill("0 9 * * *");
    await expect(dialog.getByText("Every day at 09:00 UTC")).toBeVisible({
      timeout: PREVIEW_TIMEOUT,
    });
    // The local gloss is the part that earns its keep: it is the only place the
    // UTC contract becomes concrete rather than a hint the author skimmed.
    await expect(dialog.getByText(/your time\)/)).toBeVisible({ timeout: PREVIEW_TIMEOUT });

    await cron.fill("9 0 * * *");
    await expect(dialog.getByText("Every day at 00:09 UTC")).toBeVisible({
      timeout: PREVIEW_TIMEOUT,
    });
    await expect(dialog.getByText("Every day at 09:00 UTC")).toHaveCount(0, {
      timeout: PREVIEW_TIMEOUT,
    });

    // The UTC hint stays — the preview says what THIS expression means, it does
    // not replace the statement of the contract.
    await expect(dialog.getByText("5-field cron. Times are UTC.")).toBeVisible();
  } finally {
    await removeWorkflow(request, id);
  }
});

test("a schedule the humaniser won't paraphrase still previews its next runs", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "cronterse");
  try {
    const cron = await customCronInput(page, dialog);

    // A restricted day-of-month is left undescribed on purpose — a wrong
    // paraphrase would be worse than none — but the fire times still state it.
    await cron.fill("0 0 1 * *");
    await expect(dialog.getByText(/^Next runs:/)).toBeVisible({ timeout: PREVIEW_TIMEOUT });
  } finally {
    await removeWorkflow(request, id);
  }
});

test("garbage in the cron field previews the parser's message without blocking", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "crongarbage");
  try {
    const cron = await customCronInput(page, dialog);

    // Five fields, so the shape check passes and the request goes out; the hour
    // is out of range, so the host's parser rejects it. That arrives as a 200
    // with a message, not a thrown error, and it does not disable anything.
    //
    // Scoped to the preview line's own paragraph (`CronPreviewLine`), because
    // on a graph that is otherwise complete the host's #1074 pre-flight quotes
    // the same parser message back in its alert — two true statements about one
    // mistake, and an unscoped match resolves to both.
    await cron.fill("0 99 * * *");
    await expect(cronPreview(dialog)).toBeVisible({ timeout: PREVIEW_TIMEOUT });
    await expect(dialog.getByTestId(SUBMIT)).toBeEnabled();

    // Fewer than five fields never reaches the wire — the pre-flight shape check
    // gates it — so the author sees the blur message, not a parser message.
    await cron.fill("hourly");
    await cron.blur();
    await expect(dialog.getByText(/5-field cron, e\.g\./)).toBeVisible();
    // And one mistake produces ONE complaint, not a blur error stacked on a
    // preview error.
    await expect(cronPreview(dialog)).toHaveCount(0, { timeout: PREVIEW_TIMEOUT });
  } finally {
    await removeWorkflow(request, id);
  }
});

/**
 * The condition branch control, and the host pre-flight behind it (issue #1074).
 *
 * Both are only real in a browser. `conditionBranchChoice` is unit-tested, but a
 * correct helper stays green while `EdgeRow` renders the old free-text box, or
 * passes it node ids and so never sees the source node's `kind`. And the
 * pre-flight is a debounce, a round trip and a render — the same shape as the
 * cron preview above, and observable nowhere else.
 */
test("a condition's branches are picked, not typed, and the host checks the graph (#1074)", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "branch");
  try {
    // Trigger → condition → output, the smallest graph with a branch in it. The
    // trigger row is the fixture's `start`; the other two are added here.
    await dialog.getByRole("button", { name: "Add node" }).click();
    await dialog.getByLabel("Node id").nth(1).fill("gate");
    await dialog.getByLabel("Node name").nth(1).fill("Gate");
    await dialog.getByLabel("Node kind").nth(1).click();
    await page.getByRole("option", { name: /^Condition/ }).click();
    // A condition with no `config.field` always routes true, so the host requires
    // one — filled here so the branch rule is the only thing under test.
    // By test id, not by label: the label renders as "Field *" for a required
    // field, so a label match is a substring match on rendered decoration.
    await dialog.getByTestId("config-field-field").fill("=item.approved");

    await dialog.getByRole("button", { name: "Add node" }).click();
    await dialog.getByLabel("Node id").nth(2).fill("done");
    await dialog.getByLabel("Node name").nth(2).fill("Ship it");
    await dialog.getByLabel("Node kind").nth(2).click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();

    await dialog.getByRole("button", { name: "Add edge" }).click();
    await dialog.getByLabel("Edge from").first().click();
    await page.getByRole("option", { name: "start", exact: true }).click();
    await dialog.getByLabel("Edge to").first().click();
    await page.getByRole("option", { name: "gate", exact: true }).click();

    // An edge OUT of the condition: this is the row whose control must change.
    await dialog.getByRole("button", { name: "Add edge" }).click();
    await dialog.getByLabel("Edge from").nth(1).click();
    await page.getByRole("option", { name: "gate", exact: true }).click();
    await dialog.getByLabel("Edge to").nth(1).click();
    await page.getByRole("option", { name: "done", exact: true }).click();

    // The wiring: a branch row is a combobox, not the free-text box every other
    // edge keeps. The first edge leaves a `trigger`, so it must still be a
    // textbox — the control swaps on the SOURCE node's kind, and asserting both
    // is what proves it swapped rather than being replaced everywhere.
    const branchLabel = dialog.getByLabel("Edge label").nth(1);
    await expect(branchLabel).toHaveRole("combobox");
    await expect(dialog.getByLabel("Edge label").first()).toHaveRole("textbox");

    // And it offers exactly the host's branches. `error` is absent because this
    // condition is not `on_error = "route"` — the narrow exception a hand-written
    // client check gets wrong.
    await branchLabel.click();
    await expect(page.getByRole("option", { name: "yes", exact: true })).toBeVisible();
    await expect(page.getByRole("option", { name: "no", exact: true })).toBeVisible();
    await expect(page.getByRole("option", { name: "error", exact: true })).toHaveCount(0);
    await page.getByRole("option", { name: "yes", exact: true }).click();

    // The pre-flight: the host is asked about this graph and answers before
    // anyone presses the write. This is what #1074 asked for — the dialog no
    // longer submits blind.
    await expect(dialog.getByTestId("preflight-ok")).toBeVisible({
      timeout: PREVIEW_TIMEOUT,
    });

    // And the graph the pre-flight approved is the graph the host accepts. Edit
    // mode writes with no id confirm — the id keys the saved graph and cannot
    // change, so there is nothing to confirm.
    await dialog.getByTestId(SUBMIT).click();
    await expect(dialog).toBeHidden({ timeout: 30_000 });

    const saved = await request.get(`/api/v1/company/workflows/${id}`);
    expect(saved.ok(), "the host stored the graph it pre-approved").toBeTruthy();
    const graph = await saved.json();
    expect(graph.nodes.map((n: { id: string }) => n.id)).toEqual(["start", "gate", "done"]);
  } finally {
    await removeWorkflow(request, id);
  }
});

/**
 * The other direction: a rule the client cannot pre-empt without mirroring it.
 *
 * An unreachable node is a graph-wide reachability question — the client-side
 * BFS #1074 argues against — so the dialog's own checks pass and only the host
 * knows. Before this, the operator learned it from the write's error; now the
 * pre-flight says so first, in the host's own words.
 */
test("the host's refusal of an unreachable node arrives before the write (#1074)", async ({
  page,
  request,
}) => {
  const { dialog, id } = await formOnStarter(page, request, "unreach");
  try {
    await dialog.getByRole("button", { name: "Add node" }).click();
    await dialog.getByLabel("Node id").nth(1).fill("done");
    await dialog.getByLabel("Node name").nth(1).fill("Ship it");
    await dialog.getByLabel("Node kind").nth(1).click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();

    // A third node nothing points at. Every client-side check passes: the ids are
    // unique, the kinds are fine, there is exactly one trigger, and no edge names
    // a node that is not there.
    await dialog.getByRole("button", { name: "Add node" }).click();
    await dialog.getByLabel("Node id").nth(2).fill("orphan");
    await dialog.getByLabel("Node name").nth(2).fill("Orphan");
    await dialog.getByLabel("Node kind").nth(2).click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();

    await dialog.getByRole("button", { name: "Add edge" }).click();
    await dialog.getByLabel("Edge from").click();
    await page.getByRole("option", { name: "start", exact: true }).click();
    await dialog.getByLabel("Edge to").click();
    await page.getByRole("option", { name: "done", exact: true }).click();

    const refused = dialog.getByTestId("preflight-refused");
    await expect(refused).toBeVisible({ timeout: PREVIEW_TIMEOUT });
    await expect(refused).toContainText("orphan");
    await expect(refused).toContainText("cannot be reached");
  } finally {
    await removeWorkflow(request, id);
  }
});

test("the collapsed destination shows a label, never the raw __none__ sentinel (#813)", async ({
  page,
  request,
}) => {
  // #813 defect 8: base-ui renders the stored value in the collapsed control,
  // which surfaced the bare "__none__" sentinel. The friendly label only
  // exists once rendered, so this is a browser fact.
  const { dialog, id } = await formOnStarter(page, request, "sentinel");
  try {
    await dialog.getByRole("button", { name: "Add node" }).click();
    const kind = dialog.getByLabel("Node kind").nth(1);
    await kind.click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();

    // No destination chosen yet: the control stores the "__none__" sentinel, and
    // must still read as a human label collapsed.
    const destination = dialog.getByLabel("Send report to");
    await expect(destination).toContainText("Nowhere (run result only)");
    await expect(destination).not.toContainText("__none__");
  } finally {
    await removeWorkflow(request, id);
  }
});

test("the channel destination is a picker of wired channels, not free text (#813)", async ({
  page,
  request,
}) => {
  // #813 defect 4: typing a channel id that isn't wired only failed at delivery.
  // The target is now a picker whose options are the company's wired channels.
  // `engineering` is the e2e harness company's desk channel
  // (`companies/e2e_harness/company.toml`), so it is always in the wired set.
  // `operator` is too, as of #1757: it moved from an in-memory response
  // surface delivery refused by name (#981) to a durable, journal-backed
  // channel every company wires, so the picker now lists it as a real target
  // alongside the desk channels rather than hiding it.
  const { dialog, id } = await formOnStarter(page, request, "channels");
  try {
    await dialog.getByRole("button", { name: "Add node" }).click();
    const kind = dialog.getByLabel("Node kind").nth(1);
    await kind.click();
    await page.getByRole("option", { name: /^Output(?! parser)/ }).click();
    await dialog.getByLabel("Send report to").click();
    await page.getByRole("option", { name: /^Channel/ }).click();

    // The channel target is a combobox offering both the wired desk channel and
    // the durable operator channel.
    await dialog.getByLabel("Channel id").click();
    await expect(page.getByRole("option", { name: "engineering" })).toBeVisible();
    await expect(page.getByRole("option", { name: "operator" })).toBeVisible();
  } finally {
    await removeWorkflow(request, id);
  }
});

/**
 * The create form, and the one route that still reaches it.
 *
 * A one-box create the host refuses hands the operator the fields, carrying the
 * graph that was refused. That is not a convenience for these three tests — it
 * is the claim the redesign rests on. The one box shows no id, no name and no
 * nodes, so **every** refusal it can earn names something there is no control
 * for, and without the hand-over the dialog would dead-end on the most ordinary
 * failure it has: two similar descriptions minting the same id.
 */
test("a refused create hands over the fields, carrying the graph the host refused", async ({
  page,
}) => {
  const graph = draftedGraph("e2e-handover", "Handover probe");
  const dialog = await openRefusedCreateForm(page, graph);

  // The fields are back, hydrated with what was refused — so the instruction in
  // the message ("pick a different id") is one the operator can actually follow.
  await expect(dialog.getByLabel("Workflow ID", { exact: true })).toHaveValue(graph.id);
  await expect(dialog.getByLabel("Name", { exact: true })).toHaveValue(graph.name);
  await expect(dialog.getByLabel("Node id").first()).toHaveValue("start");
  await expect(dialog.getByLabel("Node id").nth(1)).toHaveValue("report");

  // …and the id is writable, because this is still a create.
  await expect(dialog.getByLabel("Workflow ID", { exact: true })).toHaveJSProperty(
    "readOnly",
    false,
  );

  // The host's own words are on screen, not swallowed by the hand-over.
  await expect(dialog.getByTestId("create-error")).toContainText("Pick a different id");

  // The description box is gone: the dialog is the form now, not both at once.
  await expect(dialog.getByTestId("workflow-describe-box")).toHaveCount(0);
});

test("a new scheduled workflow discloses that it starts paused (#813)", async ({
  page,
}) => {
  // #813 defect 7: the #276 disarm rule creates a scheduled workflow paused,
  // but the dialog showed a live "next run" preview and said nothing about it,
  // so an author reasonably believed the cron was armed. This is rendered
  // output gated on create-mode-and-a-real-schedule, which is why it is pinned
  // on the refused-create form rather than on an edit.
  const graph = draftedGraph("e2e-paused", "Paused probe");
  const dialog = await openRefusedCreateForm(page, {
    ...graph,
    // No schedule on the trigger, so the notice has somewhere to appear from.
    nodes: [
      { id: "start", kind: "trigger", name: "Start" },
      { id: "report", kind: "output", name: "Report" },
    ],
  });
  const paused = dialog.getByText(/scheduled workflow is created paused/i);

  // No schedule set yet — there is nothing to disclose.
  await expect(paused).toHaveCount(0);

  // Pick a preset schedule on the trigger row.
  await dialog.getByLabel("Schedule").click();
  await page.getByRole("option", { name: /^Daily/ }).click();

  // Now the notice appears at author time, next to the schedule they just set.
  await expect(paused).toBeVisible();
});

test("submitting with an empty id surfaces the validation message on-screen (#813)", async ({
  page,
}) => {
  // #813 defect 6: the banner sat below the fold, so the write button looked
  // dead. On a failed submit it must be brought into view (and focused).
  //
  // Create-mode validation, and only reachable as one: `validate()` complains
  // about the id first, and edit mode never can, because there the id is
  // read-only.
  const dialog = await openRefusedCreateForm(
    page,
    draftedGraph("e2e-emptyid", "Empty id probe"),
  );

  await dialog.getByLabel("Workflow ID", { exact: true }).fill("");
  await dialog.getByTestId(SUBMIT).click();

  const banner = dialog.getByText("Give the workflow an id.");
  await expect(banner).toBeVisible();
  await expect(banner).toBeInViewport();

  // ...and the banner takes focus, so a keyboard/screen-reader user is landed on
  // the reason the write did nothing rather than left at the pressed button.
  const bannerBox = dialog.getByTestId("create-error");
  await expect(bannerBox).toBeFocused();
  await expect(bannerBox).toContainText("Give the workflow an id.");
});
