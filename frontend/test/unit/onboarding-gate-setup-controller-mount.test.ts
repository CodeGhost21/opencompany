// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { CompanyStatus, TeamMemberDto } from "@/api/types";
import { AppShell } from "@/components/app-shell";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { HostsProvider, type HostsValue } from "@/connections/HostsContext";
import type { ConnectionId, LocalScope } from "@/connections/types";

/**
 * The ordinary shell branch mounts `HostSwitcher`, which reads `useHosts()` —
 * the round-13 test above never reaches that branch (it stays in the pending
 * one), so it never needed this. A minimal value: no real hosts, every
 * mutator a no-op.
 */
const HOSTS: HostsValue = {
  connections: [],
  selected: null,
  onSelect: () => {},
  onAdd: () => {},
  localInstances: [],
  onEditHost: () => {},
  onRemoveHost: () => {},
  hub: false,
};

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

/**
 * jsdom ships no `matchMedia`, and `useIsMobile` — which `SidebarProvider`
 * calls, reached only by the ordinary shell branch below — reaches for it
 * unguarded. Same stub `sidebar-collapse-button.test.ts` installs, always
 * reporting "not matching", the desktop case at jsdom's default 1024px
 * window.
 */
function stubMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
      onchange: null,
    }),
  });
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.location.hash = "#/overview";
  stubMatchMedia();
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

  /**
   * PR #1875 review finding on `app-shell.tsx:2865` (round 13's own fix
   * hoisted `setupController` so every branch below the two early returns
   * could render it, but did not make those branches share a JSX root).
   *
   * An unstaffed roster's own read flips `setupChecked` and `setupOpen` both
   * `true` in the same `onOpenChange` call (`SetupController`'s
   * `onOpenChange?.(open || unstaffed)`). That satisfies
   * `shouldHoldShellPending`'s `if (input.setupOpen) return false` and
   * `shouldShowOnboardingGate`'s identical guard, so `AppShell` falls
   * through both early returns into the ordinary shell branch — the one
   * branch whose JSX root is `<ConsoleProvider>` rather than the `<>` the
   * pending and gate branches share. React diffs `setupController` against
   * whatever sits at that same position in the previous commit; a root-type
   * change there is indistinguishable to React from an unrelated subtree
   * replacing another, so it tears down the already-resolved
   * `SetupController` instance and mounts a fresh one — discarding the
   * proven "unstaffed" result and re-running the roster read.
   */
  it("keeps SetupController mounted when an unstaffed roster moves AppShell out of the pending branch", async () => {
    const listTeam = vi.fn(async () => [] as TeamMemberDto[]);
    const client = buildClient(listTeam);

    await act(async () => {
      root.render(
        createElement(HostsProvider, {
          value: HOSTS,
          children: createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(AppShell, {
              client,
              company: null,
              initialStatus: STATUS,
              companies: [STATUS],
              onSwitchCompany: () => {},
            }),
          }),
        }),
      );
    });

    // Let the roster read this test drives resolve, and every state update
    // and re-render it cascades into — `SetupController`'s own
    // `checked`/`unstaffed`, then `onOpenChange` flipping AppShell's
    // `setupChecked`/`setupOpen`, then AppShell's own re-render choosing a
    // different return branch — settle before inspecting how many times
    // `listTeam` actually ran. `activation` and `auth/me` are still hung
    // (`buildClient`'s own doc), so nothing here depends on either resolving.
    for (let flush = 0; flush < 5; flush += 1) {
      // eslint-disable-next-line no-await-in-loop
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
    }

    // Confirms the render actually reached the ordinary shell branch — the
    // "Skip to content" link is unique to it, so this fails loudly (not
    // silently) if some unrelated change stopped the unstaffed roster from
    // clearing the pending/gate branches the way this test assumes.
    expect(container.textContent).toContain("Skip to content");

    // The bug: reaching the ordinary shell branch above should not have cost
    // `SetupController` its already-resolved state. A second `listTeam` call
    // means it did.
    expect(listTeam).toHaveBeenCalledTimes(1);
  });
});
