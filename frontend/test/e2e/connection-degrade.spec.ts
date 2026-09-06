import { expect, test, type Page } from "@playwright/test";



/**
 * The headline requirement, and the regression that comes with it.
 *
 * The console holds several OpenCompany hosts at once. The thing that makes
 * that worth having — and the thing that is easy to lose — is that the
 * connections are *independent*: one host being unreachable reddens one row in
 * the host switcher and leaves every other host's console working.
 *
 * Before this, failure was global. `App` held one phase, so a host that could
 * not be reached rendered a full-screen "Can't connect" over the whole app, and
 * a 401 anywhere dropped the entire console to a sign-in screen. With one host
 * that is indistinguishable from correct. With two it is the bug.
 *
 * block/buzz is the cautionary example and it is worth naming, because its
 * desktop *looks* like this: a rail of workspaces down the left edge. Only one
 * is live. Its `AppState` holds a single `relay_url_override`, its retention
 * database is scoped per community, and switching rows is a stateful apply that
 * re-resolves identity and restarts agents. The rail is the easy half; staying
 * genuinely N-at-once is the half this spec guards.
 *
 * The second host here is deliberately one that does **not** exist. A test that
 * needs two live servers is a test nobody runs; an unreachable address exercises
 * exactly the property under test — that a dead connection is contained.
 */

/** A port nothing is listening on, so the second connection is always down. */
const DEAD_HOST = "http://127.0.0.1:9";

/** The tour modal covers the board and swallows clicks. */
async function silenceTour(page: Page) {
  await page.addInitScript(() => {
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, JSON.stringify({ skipped: true, seenAt: Date.now() }));
    }
  });
}

/**
 * Seeds a second host into the connection store before the app boots.
 *
 * Through `localStorage`, the same way the app itself persists hosts, because
 * the switcher's own "Add a host" item only appears once there are two — and
 * the first extra host on a *browser* has no other entry point yet (the desktop
 * shell is where adding hosts becomes a first-class flow).
 */
async function seedSecondHost(page: Page) {
  await silenceTour(page);
  await page.addInitScript((dead) => {
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        // The bootstrap host, same-origin. Named with the id the app would have
        // minted so it adopts this row rather than adding a third.
        {
          id: "conn-primary",
          baseUrl: "",
          label: "Primary",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
        {
          id: "conn-dead",
          baseUrl: dead,
          label: "Offline host",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
  }, DEAD_HOST);
}

/**
 * Three cases retired with the roster they read from.
 *
 * They covered issue #1167: a host that is down reddening its own row while the
 * others go on working, selecting the dead host to see its failure without
 * disturbing the live one, and — the naming half — a same-origin host the
 * console was told nothing about being named by the address it answers on
 * rather than by a constant, so two unnamed rows could not read identically
 * with only a dot colour between them.
 *
 * Every one of them reads a `host-row-…`, and the roster is hidden while the
 * product is scoped to one company per install (`src/product-scope.ts`,
 * `HOSTS_HIDDEN`). The degrade behaviour itself is untouched — the status probe,
 * the per-host error console and the naming rule all still run — so turning the
 * flag off restores the rows and these cases with them.
 *
 * Written down rather than deleted: the coverage that went is worth a reader
 * knowing about. What remains below is the case that still has a subject.
 */

test("the number row goes with the roster it selected from", async ({ page }) => {
  // It used to switch hosts: the listener is installed on `window` by the hosts
  // provider, not by the menu, so hiding the roster does not remove it. Left
  // live it would swallow the browser's own Cmd-2 and put an unreachable host
  // on screen with nothing naming it — the roster row that used to explain the
  // switch is gone.
  await seedSecondHost(page);
  await page.goto("/#/tasks");
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1, {
    timeout: 30_000,
  });

  const mod = process.platform === "darwin" ? "Meta" : "Control";

  // The dead host is seeded and reachable by the old shortcut. Nothing happens.
  await page.keyboard.press(`${mod}+2`);
  await expect(page.getByTestId("connection-error")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1);

  await page.keyboard.press(`${mod}+1`);
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1);
  await expect(page.getByTestId("connection-error")).toHaveCount(0);
});
