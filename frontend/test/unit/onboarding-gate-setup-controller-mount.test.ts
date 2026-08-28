// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { CompanyStatus, TeamMemberDto } from "@/api/types";
import { AppShell } from "@/components/app-shell";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import type { ConnectionId, LocalScope } from "@/connections/types";

/**
 * PR #1875 review finding, round 13.
 *
 * `shouldHoldShellPending` (`@/onboarding/gate-logic`) holds `AppShell` in a
 * neutral pending state — `<RouteLoading>` — for as long as `setupChecked` is
 * `false`, and `SetupController`'s own `onOpenChange` is the *only* thing
 * that ever sets it (see that state's own doc in `app-shell.tsx`). Before
 * this fix, the JSX that mounted `<SetupController>` lived below both of
 * `AppShell`'s early returns — reachable only once the ordinary shell itself
 * was chosen. Every fresh mount starts `setupChecked === false`, so the very
 * predicate this component exists to satisfy made `SetupController`
 * unreachable: the hold fired, the function returned before that JSX,
 * `SetupController` never mounted, `onOpenChange` never fired, and
 * `setupChecked` stayed `false` forever — a permanent loader for every
 * signed-in operator who was not a confirmed non-admin or an already-skipped
 * session.
 *
 * This is not testable as a pure `shouldHoldShellPending`/`shouldShowOnboarding
 * Gate` unit case (`onboarding-gate-logic.test.ts`) — the predicates
 * themselves are correct; the defect is in which JSX branch `AppShell`
 * mounted `<SetupController>` under. Proving the fix requires actually
 * rendering `AppShell` and observing whether `SetupController`'s own roster
 * read (`client.listTeam`) runs while the shell is held pending.
 */

const SCOPE: LocalScope = { connection: "test-connection" as ConnectionId, company: null };

const STATUS: CompanyStatus = {
  id: "co",
  name: "Acme",
  lifecycle: "running",
  pending_approvals: 0,
};

/** A staffed roster — baseline plus one real teammate (`teamIsUnstaffed`'s own contract). */
const STAFFED: TeamMemberDto[] = [
  { id: "operations", role: "Analyst", inboxEnabled: false, global: true } as TeamMemberDto,
  { id: "ada", role: "Operations", inboxEnabled: false } as TeamMemberDto,
];

/** A promise that never settles — every mount-time read this test is not exercising. */
function hang(): Promise<never> {
  return new Promise<never>(() => {});
}

/**
 * A minimal `OpenCompanyClient` double.
 *
 * `listDesks` is deliberately left hanging: `AppShell`'s own thread-hydration
 * effect only calls `client.listTeam` inside `listDesks(...).then(...)`, so
 * hanging `listDesks` suppresses that unrelated call and leaves
 * `SetupController`'s own `listTeam` read as the sole caller — the signal
 * this test actually needs. `/auth/me` and `/activation` (both routed through
 * `get`) are hung too, so `shouldHoldShellPending` stays true (the pending
 * branch) for the whole test — the same state a fresh mount starts every
 * session in. Anything this large a component reaches for that is not named
 * here becomes a permanently-pending no-op via the `Proxy` below rather than
 * a hard crash; this test only cares about `SetupController`'s own read.
 */
function buildClient(listTeam: ReturnType<typeof vi.fn>): OpenCompanyClient {
  const known = {
    baseUrl: "",
    scopeFor: (company: string | null) => `/api/v1/companies/${company ?? ""}`,
    listTeam,
    subscribeToEvents: () => () => {},
    get: hang,
    status: hang,
    approvals: hang,
    listDesks: hang,
  };
  return new Proxy(known, {
    get(target, prop, receiver) {
      if (prop in target) return Reflect.get(target, prop, receiver);
      return hang;
    },
  }) as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.location.hash = "#/overview";
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("AppShell holds pending without stranding SetupController", () => {
  it("runs SetupController's own roster read while the shell is held pending", async () => {
    const listTeam = vi.fn(async () => STAFFED);
    const client = buildClient(listTeam);

    await act(async () => {
      root.render(
        createElement(ConnectionScopeProvider, {
          scope: SCOPE,
          children: createElement(AppShell, {
            client,
            company: null,
            initialStatus: STATUS,
            companies: [STATUS],
            onSwitchCompany: () => {},
          }),
        }),
      );
    });

    // `/auth/me` and `/activation` are left hanging above, so this render
    // must still be in the neutral pending state — confirm that directly so
    // this test fails loudly (not silently) if some unrelated change moved
    // the render to a different branch before the real assertion below.
    expect(container.textContent).toContain("Loading");

    // The bug this guards: `SetupController` mounting (and completing its own
    // roster read) has nothing to do with the still-unresolved activation and
    // admin reads. Before the fix, `SetupController` was nested in JSX only
    // the fully-resolved shell reached, so `listTeam` here was never called.
    expect(listTeam).toHaveBeenCalled();
  });
});
