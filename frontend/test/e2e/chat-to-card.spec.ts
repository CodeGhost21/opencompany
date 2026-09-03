import { expect, test, type Page } from "@playwright/test";

import { LIVE_BRAIN, LIVE_BRAIN_REASON } from "./capabilities";

/**
 * End-to-end proof for the chat↔card edge (issue #246).
 *
 * Runs against a live host the harness brings up separately, with an inference
 * backend whose *choices* are scripted: a prompt carrying `SPAWNONE` makes the
 * orchestrator call `spawn_task` once. Everything else — the harness, the tool
 * plumbing, the cycle, the journal, the HTTP surface — is real.
 *
 * Three things are asserted that a curl cannot reach:
 *
 * 1. the "Add to board" action exists on a **desk** thread, where no responder
 *    carries the delegation tools and a card was previously unreachable;
 * 2. the chip a spawned card produces survives a **reload**, not merely the
 *    live POST response;
 * 3. the card links back to the conversation it came from.
 */

/**
 * A console opened against a fresh company home shows the welcome tour, which
 * renders as a modal over the whole console and swallows the first click.
 * Dismiss it when it is up. Whether it appears depends on console-local state,
 * so its absence is not a failure.
 */
async function dismissWelcome(page: Page) {
  const skip = page.getByRole("button", { name: /Skip for now/ });
  try {
    await skip.waitFor({ state: "visible", timeout: 5_000 });
  } catch {
    return;
  }
  await skip.click();
  await skip.waitFor({ state: "hidden", timeout: 10_000 });
}

/**
 * Opens one Room channel by id.
 *
 * The channel comes from the address rather than a rail click, so the spec
 * cannot proceed against an unselected transcript that still accepts a `fill`.
 */
async function openThread(page: Page, channelId: string) {
  await page.goto(`/#/chat/${channelId}`);
  await dismissWelcome(page);
  await expect(page.getByPlaceholder(/^Message /)).toBeVisible({ timeout: 30_000 });
}

test("any message on a desk thread can be added to the board", async ({ page }) => {
  await openThread(page, "engineering");

  const prompt = `ship the launch checklist ${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(prompt);
  await page.getByRole("button", { name: "Send", exact: true }).click();

  // The operator's own bubble is the one being turned into a card.
  const bubble = page.getByText(prompt, { exact: true }).first();
  await expect(bubble).toBeVisible({ timeout: 60_000 });

  // The action is hover-revealed but always focusable; hover for realism.
  await bubble.hover();
  const row = page.locator("article[data-message-id]", { hasText: prompt }).first();
  await row.getByRole("button", { name: "Add to board" }).click();

  // The confirmation chip appears on that message and links to the card.
  const chip = row.getByRole("link", { name: /Added to the board/ });
  await expect(chip).toBeVisible({ timeout: 30_000 });
  const href = await chip.getAttribute("href");
  expect(href).toMatch(/^#\/tasks\/.+/);

  // The card is real, titled from the message, and — the spend gate — did NOT
  // land in the Working phase, which is what dispatch means now (issue #1512).
  await page.goto(href!);
  await dismissWelcome(page);
  await expect(page.getByText(prompt).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText("Working", { exact: true })).toHaveCount(0);

  // …and it knows which conversation opened it.
  const origin = page.getByRole("button", { name: /Opened from chat/ });
  await expect(origin).toBeVisible();

  // The other half of the round trip: the jump lands in Room, on the channel
  // carrying that conversation. It used to land on `#/conversation` — a
  // separate surface with its own thread rail down the left — which is the
  // wireframe issue #2020 was filed against.
  await origin.click();
  // The Engineering desk's own channel, not merely some channel: a regression
  // that landed the jump on the wrong one would still match a bare `/.+/`.
  await expect(page).toHaveURL(/#\/chat\/engineering(?:[/?]|$)/);
  // `data-active` is a boolean attribute the sidebar row renders empty when
  // set and omits when not, so the assertion is on its presence.
  await expect(
    page
      .locator("[data-slot=sidebar-content]")
      .getByRole("button", { name: "Room", exact: true }),
  ).toHaveAttribute("data-active", "");

  // And Back returns to the card, because the jump went through the address
  // rather than through shell state the history knows nothing about.
  await page.goBack();
  await expect(page).toHaveURL(/#\/tasks\/.+/);
  await expect(origin).toBeVisible();
});

/**
 * Issue #2020: a card raised from **inside a thread** opens that thread on the
 * jump back, not merely the channel it lives in.
 *
 * The test above proves the channel-level half; it cannot prove this half,
 * because a first message in an empty conversation and a reply nested under it
 * both resolve to the same channel — the console would land in the right place
 * either way even with the thread root dropped on the floor. So this seeds a
 * root message and a threaded reply to it directly (the same REST surface
 * `sayFromElsewhere`-style specs already use for setup elsewhere in this
 * suite), lets the deterministic "Track" triage open the card from the reply,
 * and asserts the jump renders the thread panel holding the *root's* text —
 * not the reply's, and not merely the channel.
 */
test("a card raised inside a thread opens that thread on the jump back, not just the channel", async ({
  page,
  request,
}) => {
  const API = "/api/v1/company";
  const marker = Date.now();
  const rootText = `quick sync on Q3 priorities ${marker}`;
  const rootResponse = await request.post(`${API}/chat`, {
    data: { text: rootText, chat: "engineering" },
  });
  expect(rootResponse.ok(), await rootResponse.text()).toBeTruthy();
  const rootId = (await rootResponse.json()).messageId as string;
  expect(rootId).toBeTruthy();

  // An imperative lead ("build …") is what the deterministic triage cards —
  // independent of whatever the echo brain answers with — and `parent` is
  // what makes this a threaded reply rather than a second channel-level line.
  const replyText = `build the onboarding checklist ${marker}`;
  const replyResponse = await request.post(`${API}/chat`, {
    data: { text: replyText, chat: "engineering", parent: rootId },
  });
  expect(replyResponse.ok(), await replyResponse.text()).toBeTruthy();

  const tasksResponse = await request.get(`${API}/tasks`);
  expect(tasksResponse.ok()).toBeTruthy();
  const tasks = (await tasksResponse.json()) as Array<{
    id: string;
    title: string;
    note?: string;
  }>;
  // Keyed on the NOTE, never the title: the card's headline is named by a
  // titling pass, so the message that opened it is not in there — the note is
  // where the operator's own words are kept.
  const card = tasks.find((t) => (t.note ?? "").includes(String(marker)));
  expect(card, `no card opened from "${replyText}": ${JSON.stringify(tasks)}`).toBeTruthy();

  await page.goto(`/#/tasks/${card!.id}`);
  await dismissWelcome(page);
  const origin = page.getByRole("button", { name: /Opened from chat/ });
  await expect(origin).toBeVisible({ timeout: 15_000 });
  await origin.click();

  await expect(page).toHaveURL(/#\/chat\/engineering(?:[/?]|$)/);

  const thread = page
    .locator("aside")
    .filter({ has: page.getByRole("heading", { name: "Thread" }) });
  await expect(thread).toBeVisible({ timeout: 15_000 });
  // The root's own text is in the panel — proof the jump opened *that*
  // thread, since a channel-level landing (or the reply's own self-thread)
  // would show none of this or the wrong message.
  await expect(thread.getByText(rootText, { exact: true })).toBeVisible({ timeout: 15_000 });
});

test("a card the orchestrator opens is chipped in chat, and survives a reload", async ({
  page,
}) => {
  // Only THIS test needs the scripted backend — the one above it drives the
  // console's own "Add to board" action and passes against a default host, so
  // the skip is per-test rather than per-file.
  test.skip(!LIVE_BRAIN, LIVE_BRAIN_REASON);

  await openThread(page, "");

  // `SPAWNONE` is the scripted backend's cue to call `spawn_task` once.
  const prompt = `please track this SPAWNONE ${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(prompt);
  await page.getByRole("button", { name: "Send", exact: true }).click();

  // Live: the reply bubble says a card was opened.
  const chip = page.getByRole("link", { name: /Card opened/ }).last();
  await expect(chip).toBeVisible({ timeout: 60_000 });
  const href = await chip.getAttribute("href");
  expect(href).toMatch(/^#\/tasks\/.+/);

  // After a reload the transcript is rehydrated from `chat/history`, so a chip
  // that only existed on the live POST response would vanish here.
  await page.reload();
  await openThread(page, "");
  const rehydrated = page.getByRole("link", { name: /Card opened/ }).last();
  await expect(rehydrated).toBeVisible({ timeout: 30_000 });
  expect(await rehydrated.getAttribute("href")).toBe(href);
});

/**
 * **The dismissal, end to end, including the reload (issue #984).**
 *
 * The affordance had no coverage at all, and the half that had none was the
 * half that was broken: `clearTaskCard` only touches React state, so a
 * dismissal that looked right in the session came back on the next reload — the
 * console rehydrates from `chat/history`, and the host still had `task_id` on
 * the journaled row. The chip returned pointing at a card that no longer
 * existed, which reads as the delete having failed.
 *
 * So the reload is the assertion that matters here, and it is deliberately the
 * mirror image of the reload assertion in the test above: that one proves a
 * live card's chip *survives*, this one proves a dismissed card's chip *does
 * not come back*. Neither is safe without the other — a host that dropped every
 * `task_id` would pass this and fail that.
 *
 * Runs on the "Add to board" path rather than the scripted-backend one, so it
 * needs no `LIVE_BRAIN` and runs on every CI.
 */
test("a dismissed card's chip goes away and does not come back on reload", async ({ page }) => {
  await openThread(page, "engineering");

  const prompt = `dismiss this one ${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(prompt);
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const bubble = page.getByText(prompt, { exact: true }).first();
  await expect(bubble).toBeVisible({ timeout: 60_000 });
  await bubble.hover();
  const row = page.locator("article[data-message-id]", { hasText: prompt }).first();
  await row.getByRole("button", { name: "Add to board" }).click();

  const chip = row.getByRole("link", { name: /Added to the board/ });
  await expect(chip).toBeVisible({ timeout: 30_000 });
  const href = await chip.getAttribute("href");

  // The control is a confirm, not a bare delete — a card is not something to
  // lose to a stray click.
  await row.getByRole("button", { name: "Dismiss this card" }).click();
  await expect(page.getByText("Dismiss this card?")).toBeVisible();
  await page.getByRole("button", { name: "Dismiss card", exact: true }).click();

  // Gone from the transcript in-session…
  await expect(chip).toBeHidden({ timeout: 30_000 });

  // …and gone from the board, which is what makes it a dismissal rather than a
  // hidden chip over a card that is still filling the board.
  await page.goto(href!);
  await dismissWelcome(page);
  await expect(page.getByText(prompt).first()).toHaveCount(0, { timeout: 30_000 });

  // …and still gone after a reload. This is the regression: the transcript is
  // rehydrated from the host here, not from the React state the click cleared.
  await openThread(page, "engineering");
  await page.reload();
  await openThread(page, "engineering");
  await expect(page.getByText(prompt, { exact: true }).first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("link", { name: /Added to the board/ })).toHaveCount(0);
});
