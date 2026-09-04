import { expect, test, type Page } from "@playwright/test";



/**
 * Choosing where a runtime runs, driven through the whole app.
 *
 * The unit tests drive the connector model directly — what a profile means,
 * what the store refuses, when a failed probe is worth another attempt. This
 * drives the console the way a person does, because everything under test here
 * lives in the wiring rather than in an export: which tabs a runtime offers,
 * what the shell is asked for when one is chosen, and what the roster says
 * afterwards.
 *
 * See `docs/spec/runtime/connectors.md`.
 *
 * ## What is shimmed, and what is not
 *
 * `window.__TAURI__` exists only inside the packaged shell, so the bridge is
 * shimmed and nothing else — the same shape `desktop-local-instances.spec.ts`
 * uses, plus the three tunnel commands. It answers `oc_open_ssh_tunnel` from a
 * script the spec holds, which is what `SshTunnels` does in Rust minus the
 * child process. **No `ssh` runs here**: a spec that needs a reachable machine
 * with a key on it is a spec CI skips, and a skipped spec proves nothing.
 * `ssh.rs`'s own tests cover the argv and the roster; every decision asserted
 * on here belongs to the console.
 *
 * The browser cases shim nothing at all.
 */



/**
 * Opens the chooser, by the one route that still reaches it.
 *
 * It used to be the switcher's "Add a host". That item is hidden while the
 * product is scoped to one company per install (`src/product-scope.ts`), and a
 * hub is now the only build that opens this screen at all — it runs no host of
 * its own, so "where does this company run" is a question it genuinely has to
 * ask. The desktop's zero-host recovery is a single "start the host on this
 * computer" action instead, covered in `desktop-connections.spec.ts`.
 */
async function openTheChooser(page: Page): Promise<void> {
  await expect(page.getByTestId("no-connection")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("no-connection-add").click();
  await expect(page.getByTestId("add-host-remote")).toBeVisible({ timeout: 5_000 });
}

/** Every connector this console is offering right now. */
async function offeredConnectors(page: Page): Promise<string[]> {
  const kinds = ["local", "cloud", "remote", "ssh"];
  const present = await Promise.all(
    kinds.map(async (kind) => ((await page.getByTestId(`add-host-${kind}`).count()) > 0 ? kind : null)),
  );
  return present.filter((kind): kind is string => kind !== null);
}

/**
 * Four cases retired with the desktop's chooser.
 *
 * They covered the screen an operator reaches from "Add a host": that a desktop
 * offers all four places a runtime can run (this computer, the hosted platform,
 * a gateway, over ssh); that the chooser is a screen whose tabs fit the card
 * they are drawn in rather than overflowing it; that a host reached over ssh is
 * added at the address the shell actually opened rather than the one typed; and
 * that ssh's refusal stays on screen instead of quietly becoming a host.
 *
 * All four need the desktop shell, and the desktop has no way onto that screen
 * while the product is scoped to one company per install
 * (`src/product-scope.ts`, `HOSTS_HIDDEN`): the switcher opens nothing, and the
 * zero-host recovery is a single "start the host on this computer" action,
 * covered in `desktop-connections.spec.ts`. The local and ssh connectors are
 * desktop-only, so a hub cannot stand in for them — retargeting these would
 * change what they test rather than where they run.
 *
 * The connector registry, the ssh address rule and the tab set are untouched in
 * the source; turning the flag off restores the screen and all four cases. The
 * two below still run, because a hub genuinely has to ask where a company runs
 * and so keeps the chooser.
 */

test("a browser is offered only the two connectors it can honour", async ({ page }) => {
  // A hub, because it is the browser shape that genuinely holds N hosts, and
  // the connector list is about adding the *next* one. A single-host console
  // opens the same menu (`hostSwitcherMenu`), but only because it has a host
  // to manage — this spec is about the choice, so it drives the shape the
  // choice belongs to.
  await page.goto("/?hub");
  await openTheChooser(page);

  // `local` and `ssh` both need a process started on this machine, and a
  // browser has no core to start one in. A tab whose button cannot be honoured
  // is worse than a tab that is not there.
  expect(await offeredConnectors(page)).toEqual(["cloud", "remote"]);
});

test("a browser is told a gateway has to allow this console's origin", async ({ page }) => {
  // The most likely support question this connector generates, answered where
  // it is cheapest to answer. There is no wildcard for it either — the session
  // is a credential — so the operator has to go and set this.
  await page.goto("/?hub");
  await openTheChooser(page);
  await page.getByTestId("add-host-remote").click();

  await expect(page.getByText("OPENCOMPANY_CORS_ORIGINS")).toBeVisible();
});

/**
 * Two cases retired with the roster they read from.
 *
 * They covered the reason a connector is written down at all: the same dead
 * address means different things depending on what is behind it. A cloud tenant
 * that is asleep reads "Waking…" and ranks as connecting, because something is
 * going to wake it; a gateway somebody runs themselves reads "Unreachable",
 * because nothing is. Both are read off a `host-row-…`, and the roster is hidden
 * while the product is scoped to one company per install
 * (`src/product-scope.ts`, `HOSTS_HIDDEN`).
 *
 * The distinction itself is untouched — `keepWaking` and the connector field it
 * reads are still unit-tested in `connectors.test.ts` — so what went is the
 * rendering of it. Turning the flag off restores both rows.
 */

test("a hub with no hosts offers the choice rather than describing it", async ({ page }) => {
  // The onboarding dead end. A hub's own origin serves assets and nothing
  // else, so a new one holds zero connections — nothing went wrong, and the
  // desktop's "the host on this computer didn't start" describes a computer
  // that was never going to run one.
  await page.goto("/?hub");

  const empty = page.getByTestId("no-connection");
  await expect(empty).toBeVisible({ timeout: 30_000 });
  await expect(empty).toContainText("No company connected yet");
  await expect(empty).not.toContainText("didn't start");

  // And it is a control, not a sentence naming one somewhere else.
  await page.getByTestId("no-connection-add").click();
  expect(await offeredConnectors(page)).toEqual(["cloud", "remote"]);
});
