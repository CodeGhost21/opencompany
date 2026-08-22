import { expect, type Locator, type Page } from "@playwright/test";

/**
 * The console gestures an orchestration run is made of, shared by the two specs
 * that drive one: `orchestration-simulation.spec.ts` (scripted answers) and
 * `orchestration-live.spec.ts` (a real model).
 *
 * The two specs assert very different things — one that the chain *runs* in a
 * fixed shape, the other that a model *chooses* to use it — but they drive the
 * same console the same way, and a chat composer or a board column that moved
 * should break one edit here rather than two specs quietly.
 *
 * Everything below is a gesture an operator can actually perform. Reads of the
 * host's own record belong in the specs (through `request`), where they can be
 * read beside the claim they settle.
 */

/** The single-company alias the host answers on. */
export const SCOPE = "/api/v1/company";

/** The main line — the General desk, which is the orchestrator's thread. */
export const MAIN_LINE = "main";

/** The board, now that it is the `tasks` ledger rather than a screen of its own. */
export const BOARD = "/#/ledgers/tasks";

/** The board's phases, in board order (issue #1512). */
export const PENDING = 0;
export const WORKING = 1;
export const DONE = 2;

/**
 * Answers "already skipped" for whatever tour key the console asks about.
 *
 * The first-run tour renders a modal over the whole console and swallows the
 * first click on every screen. Keyed on the company id, which this suite does
 * not want to hard-code, so the prefix is matched instead — the same shape
 * `chat-dispatch-marker.spec.ts` uses.
 */
export async function silenceTour(page: Page) {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
}

/** Opens one channel by id and waits for its composer, i.e. for the view. */
export async function openChannel(page: Page, channelId: string = MAIN_LINE) {
  await page.goto(`/#/chat/${channelId}`);
  await expect(page.getByPlaceholder(/^Message /)).toBeVisible({ timeout: 30_000 });
}

/** Says one thing in the open channel. */
export async function say(page: Page, text: string) {
  await page.getByPlaceholder(/^Message /).fill(text);
  await page.getByRole("button", { name: "Send", exact: true }).click();
}

/**
 * Every dispatch marker in the open transcript, as links.
 *
 * A link rather than any element carrying the text: the channel rail renders a
 * one-line preview of the last message as a *button*, so an unscoped match
 * counts one extra intermittently, depending on which rendered first — the trap
 * `chat-live-events.spec.ts` documents and `chat-dispatch-marker.spec.ts`
 * inherits.
 */
export function markers(page: Page): Locator {
  return page.getByRole("link", { name: /^finished → / });
}

/** The board, with every phase pinned open. */
export async function openBoard(page: Page) {
  await page.goto(BOARD);
  // The columns are a read off the `tasks` ledger, not a literal: the board is
  // not itself until it has them, and a card located before that resolves to
  // nothing for reasons that look like the card never being opened.
  await expect(column(page, DONE)).toHaveCount(1, { timeout: 30_000 });
  await expandAll(page);
}

export const board = (page: Page) => page.getByTestId("ledger-board");

export const column = (page: Page, index: number) =>
  board(page).getByTestId("board-column").nth(index);

/**
 * Opens every phase that has collapsed itself to a rail (issue #1101).
 *
 * An empty phase folds to a ~40px strip, and dropping a card onto a rail is a
 * different gesture from dropping it onto a column — the rail opens under the
 * pointer and the board reflows mid-drop. Every dispatch below therefore starts
 * from a board whose phases are all open, which is also the state an operator
 * reaches by clicking a rail.
 */
export async function expandAll(page: Page) {
  const rails = board(page).locator("button[aria-label^='Expand ']");
  for (let attempt = 0; attempt < 8; attempt += 1) {
    if ((await rails.count()) === 0) return;
    await rails.first().click();
  }
  await expect(rails).toHaveCount(0);
}

/** One card on the board, by the title it carries. */
export function card(page: Page, title: string): Locator {
  return board(page).locator("[draggable=true]").filter({ hasText: title }).first();
}

/**
 * A real pointer drag from `card` to the middle of `target`.
 *
 * Walked in steps rather than jumped: Chromium promotes a press-and-move to a
 * drag session only after the pointer has actually moved, and a single
 * `mouse.move` to the destination is not a drag as far as the board's
 * `dragover` handlers are concerned.
 */
export async function dragCard(page: Page, from: Locator, target: Locator) {
  const start = await from.boundingBox();
  const end = await target.boundingBox();
  if (!start) throw new Error("the card has no box to grab");
  if (!end) throw new Error("the target column has no box");
  const fromX = start.x + start.width / 2;
  const fromY = start.y + start.height / 2;
  const toX = end.x + end.width / 2;
  const toY = end.y + Math.min(end.height / 2, 160);

  await page.mouse.move(fromX, fromY);
  await page.mouse.down();
  for (let step = 1; step <= 12; step += 1) {
    await page.mouse.move(fromX + ((toX - fromX) * step) / 12, fromY + ((toY - fromY) * step) / 12);
    await page.waitForTimeout(40);
  }
  await page.waitForTimeout(150);
  await page.mouse.up();
}

/**
 * Dispatches one card the way an operator does: dragging it into Working.
 *
 * This is the spend gate, and it is deliberately the operator's move rather
 * than the orchestrator's (issue #1512). `spawn_task` opens a card in Pending;
 * entering Working is what fires the assignee's turn, so a company cannot spend
 * money on work nobody asked it to start.
 */
export async function dispatch(page: Page, title: string) {
  await openBoard(page);
  const dragging = card(page, title);
  await expect(dragging, `the card "${title}" is not on the board`).toBeVisible({
    timeout: 30_000,
  });
  await dragCard(page, dragging, column(page, WORKING));
  await expect(column(page, WORKING)).toContainText(title, { timeout: 30_000 });
}

/**
 * Opens one card's detail screen and returns its id, read off the address.
 *
 * The id is needed to say anything precise afterwards — which card settled,
 * which one the orchestrator was asked to approve — and clicking the card is
 * how an operator gets it. Nothing here reads the host's task list to find it,
 * because a spec that asked the API which cards exist would pass against a
 * board that rendered none of them.
 */
export async function openCard(page: Page, title: string): Promise<string> {
  await openBoard(page);
  await card(page, title).click();
  await expect(page).toHaveURL(/#\/tasks\/[^/]+$/, { timeout: 30_000 });
  const id = page.url().split("#/tasks/")[1];
  await expect(page.getByText(title).first()).toBeVisible({ timeout: 30_000 });
  return id;
}
