import { expect, test, type Page } from "@playwright/test";

/**
 * The headline requirement, and the regression that comes with it.
 *
 * The console holds several OpenCompany hosts at once. The thing that makes
 * that worth having — and the thing that is easy to lose — is that the
 * connections are *independent*: one host being unreachable reddens one row in
 * the rail and leaves every other host's console working.
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

/**
 * Seeds a second host into the connection store before the app boots.
 *
 * Through `localStorage`, the same way the app itself persists hosts, because
 * the rail's own "Add a host" control only appears once there are two — and the
 * first extra host on a *browser* has no other entry point yet (the desktop
 * shell is where adding hosts becomes a first-class flow).
 */
async function seedSecondHost(page: Page) {
  await page.addInitScript((dead) => {
    // The tour modal covers the board and swallows clicks.
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, JSON.stringify({ skipped: true, seenAt: Date.now() }));
    }
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

test("a host that is down reddens its own row and leaves the others working", async ({
  page,
}) => {
  await seedSecondHost(page);
  await page.goto("/#/tasks");

  // Both hosts are present, so the rail is showing.
  const rail = page.getByTestId("connection-rail");
  await expect(rail).toBeVisible({ timeout: 30_000 });

  // The live one reaches `live`; the dead one reaches `down`. Neither waits on
  // the other — that independence is the property, and a global phase would
  // have blanked the app before either resolved.
  await expect(page.getByTestId("connection-row-conn-primary")).toHaveAttribute(
    "data-status",
    "live",
    { timeout: 30_000 },
  );
  await expect(page.getByTestId("connection-row-conn-dead")).toHaveAttribute(
    "data-status",
    "down",
    { timeout: 30_000 },
  );

  // THE assertion. The working host's console is rendered, not a full-screen
  // error — the dead host is contained to its row.
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1);
  await expect(page.getByTestId("connection-error")).toHaveCount(0);
});

test("selecting the dead host shows its failure without disturbing the live one", async ({
  page,
}) => {
  await seedSecondHost(page);
  await page.goto("/#/tasks");
  await expect(page.getByTestId("connection-rail")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1);

  await page.getByTestId("connection-row-conn-dead").click();

  // The dead host says so, in the console area rather than over the whole app.
  await expect(page.getByTestId("connection-error")).toBeVisible({ timeout: 30_000 });
  // And the rail is still there, still showing the live host as live: selecting
  // a broken connection must not tear down a working one. This is exactly what
  // a stateful "apply" switch would break.
  await expect(page.getByTestId("connection-row-conn-primary")).toHaveAttribute(
    "data-status",
    "live",
  );

  // Switching back finds the working console still working, with no reload.
  await page.getByTestId("connection-row-conn-primary").click();
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1);
  await expect(page.getByTestId("connection-error")).toHaveCount(0);
});
