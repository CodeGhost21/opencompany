import { expect, test } from "@playwright/test";

/**
 * The nav sidebar must start where the connection rail ends.
 *
 * The shell's sidebar container is `position: fixed`, so it positions against
 * the viewport rather than the flex column it is written inside. Pinned to
 * `left: 0` it slides underneath the connection rail — which appears as soon
 * as a second host is added — and the rail's 56px covers every nav icon and
 * the first characters of every label. "Company" reads as "mpany".
 *
 * It survived review once already: the rail carries `z-50` from an earlier
 * pass that fixed clicks landing on the wrong element, which made the rail
 * paint *over* the sidebar rather than under it, and left the visual clipping
 * in place while looking like a fix.
 *
 * It also survived a full design-system migration and two contrast audits,
 * because every one of those ran against the default single-host console where
 * the rail does not render at all. That is the point of this spec: the broken
 * state needs two connections, and nothing else in the suite creates them.
 */

// The first-run tour opens a dialog over the console and `aria-hidden`s
// everything beneath it. Same shim the rest of the suite uses.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

/** Must match `companies/e2e_harness/company.toml`'s `[users] admins`. */
const ADMIN_EMAIL = "harness-e2e@tinyhumans.ai";

test("the sidebar clears the connection rail when a second host is added", async ({
  page,
  baseURL,
}) => {
  const base = (baseURL ?? "http://127.0.0.1:8080").replace(/\/$/, "");

  // Signs in through the page's OWN context, so the session cookie lands in
  // the jar the page uses. Deliberately not relying on the shared
  // `storageState`: this spec seeds `oc.connections.v1` before boot, and a
  // spec that rewrites the connection list should not also depend on a
  // session minted for a different connection list.
  const api = page.context().request;
  const requested = await api.post(`${base}/api/v1/company/auth/request`, {
    data: { email: ADMIN_EMAIL },
  });
  const { dev_code: devCode } = (await requested.json()) as { dev_code?: string };
  expect(devCode, "the host should echo a dev_code when no mail is configured").toBeTruthy();
  const verified = await api.post(`${base}/api/v1/company/auth/verify`, {
    data: { email: ADMIN_EMAIL, code: devCode },
  });
  expect(verified.ok(), "sign-in should mint a session").toBe(true);
  // Two hosts, so the rail renders. The second address is never contacted —
  // the rail draws it as unreachable, which is all this spec needs.
  await page.addInitScript((primary: string) => {
    localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "rail-spec-primary",
          baseUrl: primary,
          label: "Primary",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
        {
          id: "rail-spec-second",
          baseUrl: "http://127.0.0.1:9",
          label: "Second host",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
  }, base);

  await page.goto("/#/company");

  const rail = page.getByTestId("connection-rail");
  await expect(rail).toBeVisible();

  const sidebar = page.locator("[data-slot=sidebar-container]");
  await expect(sidebar).toBeVisible();

  const railBox = await rail.boundingBox();
  const sidebarBox = await sidebar.boundingBox();
  expect(railBox, "connection rail should have a box").not.toBeNull();
  expect(sidebarBox, "sidebar should have a box").not.toBeNull();

  // The whole assertion: the sidebar starts at or after the rail's right edge.
  // Pinned to the viewport it starts at 0, and this reads 0 >= 56.
  expect(sidebarBox!.x).toBeGreaterThanOrEqual(railBox!.x + railBox!.width);

  // And the labels are actually on screen rather than clipped behind it — the
  // symptom a reader would report, asserted separately from the cause so a
  // future change that moves the sidebar for some other reason still fails
  // here rather than passing on a technicality.
  // A nav row is a button, not a link — the shell routes on the hash rather
  // than navigating. `exact` keeps this off the page's own "Company" heading.
  const label = page.getByRole("button", { name: "Company", exact: true });
  const labelBox = await label.boundingBox();
  expect(labelBox, "the Company nav row should have a box").not.toBeNull();
  expect(labelBox!.x).toBeGreaterThanOrEqual(railBox!.x + railBox!.width);
});
