import { expect, type Locator, type Page } from "@playwright/test";

/**
 * Shared locators for the chat e2e specs (issue #1000). These used to be
 * redefined in each of the three specs that need them; a change to
 * `article[data-message-id]` or to the `working-indicator` test id therefore
 * required the same edit in every file. Kept here next to `./capabilities` so
 * the specs import one definition instead of drifting apart.
 */

/**
 * Opens one channel by id and waits for its transcript to be on screen.
 */
export async function openChannel(page: Page, channelId: string) {
  await page.goto(`/#/chat/${channelId}`);
  await expect(page.getByPlaceholder(/^Message /)).toBeVisible({ timeout: 30_000 });
}

/** Every non-system bubble currently rendered in the open channel. */
export function bubbles(page: Page): Locator {
  return page.locator("article[data-message-id]");
}

/**
 * The bubble carrying the reply to `marker`, in the open transcript.
 *
 * Scoped to a bubble, not matched across the page: the rail renders a one-line
 * preview of each channel's last message, so a bare
 * `getByText("You said: <marker>")` resolves to two elements the moment the
 * preview catches up. Found while standing up the live-brain lane (#467).
 */
export function reply(page: Page, marker: string): Locator {
  return bubbles(page).filter({ hasText: `You said: ${marker}` });
}

/** The working/queued row, however it is currently worded. */
export function workingRow(page: Page): Locator {
  return page.getByTestId("working-indicator");
}
