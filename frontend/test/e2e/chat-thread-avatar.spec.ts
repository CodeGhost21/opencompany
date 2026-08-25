import { expect, test, type Locator, type Page } from "@playwright/test";

import { bubbles, openChannel } from "./chat-helpers";

/**
 * End-to-end proof for issue #1729 — the thread panel drew the agent's face on
 * the current user's own line.
 *
 * This needs a browser, and specifically needs the *signed-in* console: the
 * face on a "you" line comes from `auth/me`, and the bug was `ThreadPanel`
 * resolving its senders without it, so every reader's own line fell back to the
 * mascot hashed from the literal string "You". A unit test pins the resolution
 * (`test/unit/chat-thread-avatar.test.ts`); this pins that the value the panel
 * needs actually reaches it through the real shell, against a real host, for a
 * real viewer.
 *
 * The assertion is deliberately "the thread shows the same face the timeline
 * shows", not "the face is such-and-such flavour" — that is the property the
 * issue is about (a reader must be able to tell their own lines from the
 * agent's, in both places), and it cannot be passed by coincidence the way an
 * assertion on one hashed flavour could.
 */

const ENGINEERING = "engineering";

test.beforeEach(async ({ page }) => {
  // The first-run tour opens a modal over the console and swallows every click
  // beneath it (the pattern every chat spec here uses).
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

/** The `src` of the avatar image inside one bubble or thread line. */
async function faceOf(row: Locator): Promise<string> {
  return (await row.locator("img").first().getAttribute("src")) ?? "";
}

/** The thread panel, once it is open. */
function panel(page: Page): Locator {
  return page.locator("aside").filter({ has: page.getByRole("heading", { name: "Thread" }) });
}

test("your own line wears your own face in the thread panel too", async ({ page }, testInfo) => {
  await openChannel(page, ENGINEERING);

  const marker = `thread-face-${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(marker);
  await page.getByPlaceholder(/^Message /).press("Enter");

  // Your own line in the main timeline, and the reply beneath it. The offline
  // echo brain answers every message, so there is always a second voice.
  const mine = bubbles(page).filter({ hasText: marker }).first();
  await expect(mine).toBeVisible({ timeout: 30_000 });
  const theirs = bubbles(page).filter({ hasText: `You said: ${marker}` }).first();
  await expect(theirs).toBeVisible({ timeout: 30_000 });

  const timelineFace = await faceOf(mine);
  const agentFace = await faceOf(theirs);
  expect(timelineFace, "the main timeline already drew your own face").not.toBe("");

  // Open a thread off your own message and reply in it, so the panel holds
  // both voices: your line and the agent's answer to it.
  await mine.hover();
  await mine.getByRole("button", { name: "Reply in thread" }).click();
  const thread = panel(page);
  await expect(thread).toBeVisible();

  await thread.getByPlaceholder("Reply…").fill("and here is a reply");
  await thread.getByPlaceholder("Reply…").press("Enter");
  await expect(thread.getByText("and here is a reply")).toBeVisible({ timeout: 30_000 });

  // The bug: this line's face was the mascot hashed from "You", which is the
  // agent's green one — both participants wore one face and the thread could
  // not be read.
  const inThread = thread.locator("div").filter({ hasText: "and here is a reply" }).last();
  const threadFace = await faceOf(inThread);
  expect(threadFace, "the thread must show the same face the timeline does").toBe(timelineFace);
  expect(threadFace, "and it must not be the agent's").not.toBe(agentFace);

  // Evidence for the issue: the panel with a human author beside an agent one,
  // in both themes.
  for (const theme of ["light", "dark"] as const) {
    // `next-themes` is configured `attribute="class"`, so the live switch is
    // the class on `<html>`; `localStorage` is only what it reads at boot.
    await page.evaluate((value) => {
      window.localStorage.setItem("theme", value);
      document.documentElement.classList.remove("light", "dark");
      document.documentElement.classList.add(value);
      document.documentElement.style.colorScheme = value;
    }, theme);
    await testInfo.attach(`thread-panel-${theme}`, {
      body: await thread.screenshot(),
      contentType: "image/png",
    });
  }
});
