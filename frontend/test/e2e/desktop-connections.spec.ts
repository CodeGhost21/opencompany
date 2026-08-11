import { expect, test, type Page } from "@playwright/test";

/**
 * What the desktop opens on, driven against a real host.
 *
 * Issue #613: the packaged app added a connection to its own origin — an empty
 * base url, which means "same origin" and is a real host only in a browser —
 * made it the bootstrap, and therefore opened on "Couldn't reach a company host
 * at this origin" every launch while its embedded host sat healthy and
 * unselected in the rail.
 *
 * The desktop cannot be packaged inside this suite, but the thing that *makes*
 * it a desktop can be: `isDesktopRuntime()` is `"__TAURI__" in window` and
 * nothing else, and every host request then goes through `ProxyTransport` to
 * the bridge stubbed below. So this exercises the real console bundle, the real
 * boot sequence and the real Rust-backed host — with the one seam that defines
 * the desktop standing in for the shell.
 *
 * The stub is deliberately faithful on the point the bug turned on: it refuses
 * a base url that is not absolute, exactly as `ProxyRegistry::upsert` now does,
 * because a stub that quietly resolved `"" + "/api/v1"` would be a stub in
 * which the bug cannot happen.
 */

interface BridgeConfig {
  /** What `oc_embedded` answers. `null` is a desktop whose host did not start. */
  embedded: string | null;
}

/** One `oc_connect` the console made, as the test reads them back. */
interface RegisteredConnection {
  connectionId: string;
  baseUrl: string;
}

declare global {
  interface Window {
    __ocRegistered?: RegisteredConnection[];
  }
}

/**
 * Installs a Tauri bridge before the app boots.
 *
 * `oc_request` forwards to `fetch` against the connection's *registered* base
 * url, which is what the Rust proxy does — and, like the proxy, it resolves for
 * every HTTP status rather than throwing, so the console's own error handling
 * is the thing under test.
 */
async function asDesktop(page: Page, config: BridgeConfig) {
  await page.addInitScript((cfg: BridgeConfig) => {
    // The tour modal covers the board and swallows clicks. These are the legacy
    // keys, which every connection adopts on first read — the scoped ones carry
    // a connection id this test cannot know, because the embedded host's is
    // minted at runtime.
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, JSON.stringify({ skipped: true, seenAt: Date.now() }));
    }

    const registered: RegisteredConnection[] = [];
    const hosts = new Map<string, string>();
    window.__ocRegistered = registered;

    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      Channel: class {
        onmessage: ((message: string) => void) | null = null;
      },
      async invoke(command: string, args: Record<string, unknown>): Promise<unknown> {
        switch (command) {
          case "oc_connect": {
            const id = args.connectionId as string;
            const baseUrl = args.baseUrl as string;
            registered.push({ connectionId: id, baseUrl });
            hosts.set(id, baseUrl);
            return undefined;
          }
          case "oc_disconnect": {
            hosts.delete(args.connectionId as string);
            return undefined;
          }
          case "oc_embedded":
            return cfg.embedded === null
              ? null
              : { baseUrl: cfg.embedded, dataDir: "/tmp/e2e-desktop" };
          case "oc_request": {
            const id = args.connectionId as string;
            const base = hosts.get(id);
            if (base === undefined) throw new Error(`no such connection: ${id}`);
            // `ProxyRegistry::upsert` refuses these at registration; refusing
            // here too keeps the stub from being kinder than the core.
            if (!/^https?:\/\//.test(base)) throw new Error(`not an absolute host url: "${base}"`);
            const req = args.request as {
              method: string;
              path: string;
              headers: Record<string, string>;
              body?: string;
            };
            const response = await fetch(base + req.path, {
              method: req.method,
              headers: req.headers,
              body: req.body ?? undefined,
              credentials: "include",
            });
            const text = await response.text();
            const headers: Record<string, string> = {};
            response.headers.forEach((value, name) => {
              headers[name.toLowerCase()] = value;
            });
            return {
              status: response.status,
              statusText: response.statusText,
              url: response.url,
              text,
              headers,
            };
          }
          default:
            return null;
        }
      },
    };
  }, config);
}

/**
 * A port nothing listens on, so a remembered remote host is reliably down.
 *
 * Being unreachable is what gives the ordering test its teeth: if the console
 * opened on this row rather than on the embedded host, the assertion that the
 * board is on screen could not pass.
 */
const DEAD_REMOTE = "http://127.0.0.1:9";

/** Seeds the dead row a desktop built before this fix wrote on its first run. */
async function seedSameOriginProfile(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "conn-stale-origin",
          baseUrl: "",
          label: "This host",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
  });
}

test("a desktop opens on its embedded host, not on its own origin", async ({
  page,
  baseURL,
}) => {
  // The embedded host is the host serving this suite: a real OpenCompany at a
  // real absolute address, which is exactly what `oc_embedded` reports on a
  // packaged run.
  await asDesktop(page, { embedded: new URL(baseURL ?? "http://127.0.0.1:8080").origin });
  await seedSameOriginProfile(page);
  await page.goto("/#/tasks");

  // THE assertion, and the whole issue: no error panel. Before the fix this
  // read "Couldn't reach a company host at this origin" on every launch.
  await expect(page.getByTestId("connection-error")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1, {
    timeout: 30_000,
  });

  // The same-origin connection was never registered with the core. A row that
  // reached `oc_connect` is a row the console believed it could address.
  const registered = await page.evaluate(() => window.__ocRegistered ?? []);
  expect(registered.length).toBeGreaterThan(0);
  expect(registered.map((r) => r.baseUrl)).not.toContain("");

  // And the row a previous build wrote is gone from storage rather than merely
  // skipped — otherwise it returns on the next launch, forever.
  const stored = await page.evaluate(() =>
    JSON.parse(window.localStorage.getItem("oc.connections.v1") ?? "[]"),
  );
  expect(stored.map((p: { baseUrl: string }) => p.baseUrl)).not.toContain("");
});

test("a remembered host does not take the launch just by being older", async ({
  page,
  baseURL,
}) => {
  // The case the fix's *selection* half exists for, and the one a single-host
  // test cannot reach. A host added in some previous session is restored at
  // first paint; the embedded host is appended later, because its port only
  // arrives over IPC. So list order records when each was learned about — and
  // taking the first entry would open the desktop on last Tuesday's remote host
  // instead of on the machine in front of the person. That is #613's shape
  // again, with the dead bootstrap swapped for a stale favourite.
  await asDesktop(page, { embedded: new URL(baseURL ?? "http://127.0.0.1:8123").origin });
  await page.addInitScript((remote: string) => {
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "conn-remembered-remote",
          baseUrl: remote,
          label: "Remembered remote",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
  }, DEAD_REMOTE);
  await page.goto("/#/tasks");

  // Both hosts are registered, so the rail is drawn and there is a choice.
  await expect(page.locator('[data-testid^="connection-row-"]')).toHaveCount(2, {
    timeout: 30_000,
  });
  // The console on screen is a working one — which it could not be if the
  // unreachable remote had been selected for sorting first.
  await expect(page.getByRole("button", { name: "Add task" })).toHaveCount(1, {
    timeout: 30_000,
  });
  await expect(page.getByTestId("connection-error")).toHaveCount(0);

  // Said directly, rather than inferred from what rendered: the remembered host
  // is present and not current, and whatever is current is live.
  await expect(page.getByTestId("connection-row-conn-remembered-remote")).toHaveAttribute(
    "aria-current",
    "false",
  );
  await expect(
    page.locator('[data-testid^="connection-row-"][aria-current="true"]'),
  ).toHaveAttribute("data-status", "live");
});

test("a desktop whose host did not start says so, and can still add one", async ({ page }) => {
  // No embedded host, no remembered hosts: the state that used to render an
  // empty pane once the same-origin connection stopped filling it.
  await asDesktop(page, { embedded: null });
  await page.goto("/");

  await expect(page.getByTestId("no-connection")).toBeVisible({ timeout: 30_000 });
  // The rail holds the only "add a host" control, so it has to survive a count
  // of zero or the operator is looking at a dead end.
  await expect(page.getByTestId("connection-rail")).toBeVisible();
  await expect(page.getByTestId("connection-add")).toBeVisible();
});
