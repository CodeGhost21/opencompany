// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { CompanyStatus } from "@/api/types";
import { AppShell } from "@/components/app-shell";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { HostsProvider, type HostsValue } from "@/connections/HostsContext";
import type { Connection, ConnectionId, LocalScope } from "@/connections/types";
import { clearGateStepWaivers, markGateStepWaived, waivedGateSteps } from "@/onboarding/state";

/**
 * Codex review, PR #2046.
 *
 * `AppShell`'s `gateWaived` state only re-read `localStorage` when `scope`
 * changed — never on an update written by a DIFFERENT tab already open on
 * the same company. A waiver is durably scoped and meant to survive a fresh
 * tab (`markGateStepWaived`'s own doc), but a tab that was already open when
 * another tab waived the last outstanding step kept the gate up regardless,
 * until reloaded or the founder repeated the waive in that tab too.
 *
 * The fix listens for the native `storage` event, which the browser already
 * fires in every OTHER same-origin tab (never the one that wrote) — this
 * test dispatches that event by hand to stand in for the other tab.
 */

const CONNECTION_ID = "conn-1" as ConnectionId;
const SCOPE: LocalScope = { connection: CONNECTION_ID, company: "acme" };

const STATUS: CompanyStatus = {
  id: "acme",
  name: "Acme",
  lifecycle: "running",
  pending_approvals: 0,
};

function hang(): Promise<never> {
  return new Promise<never>(() => {});
}

/**
 * name and workflow are already done; only `integration` is outstanding —
 * the one step this test waives from "another tab".
 */
function buildClient(): OpenCompanyClient {
  const known = {
    baseUrl: "",
    scopeFor: (company: string | null) => `/api/v1/companies/${company ?? ""}`,
    subscribeToEvents: () => () => {},
    get: (path: string) => {
      if (path.endsWith("/auth/me")) return Promise.resolve({ role: "admin" });
      if (path.endsWith("/activation")) {
        return Promise.resolve({
          nameConfirmed: true,
          integrationConnected: false,
          workflowRunSucceeded: true,
          isActivated: false,
        });
      }
      return hang();
    },
    status: hang,
    approvals: hang,
    listDesks: hang,
    // Must resolve, not hang: `shouldHoldShellPending` also gates on
    // `SetupController`'s own `setupChecked`, which only settles once this
    // read lands (see the round-13/14 findings in
    // `onboarding-gate-setup-controller-mount.test.ts`).
    listTeam: async () => [
      { id: "operations", role: "Analyst", inboxEnabled: false, global: true },
      { id: "ada", role: "Operations", inboxEnabled: false },
    ],
  };
  return new Proxy(known, {
    get(target, prop, receiver) {
      if (prop in target) return Reflect.get(target, prop, receiver);
      return hang;
    },
  }) as unknown as OpenCompanyClient;
}

const CONNECTION: Connection = {
  id: CONNECTION_ID,
  defaultCompany: null,
  label: "test",
  baseUrl: "",
  credential: { kind: "cookie" },
  status: "live",
  identity: null,
  companies: [],
  connector: { kind: "remote" },
};

const HOSTS: HostsValue = {
  connections: [CONNECTION],
  selected: CONNECTION_ID,
  onSelect: () => {},
  onAdd: () => {},
  localInstances: [],
  onEditHost: () => {},
  onRemoveHost: () => {},
  hub: false,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  window.location.hash = "#/overview";
  localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

/**
 * Codex review, PR #2046, round 3: a waiver must be dropped when ITS OWN step
 * completes, not only when the whole funnel activates. Otherwise a founder who
 * waives `integration`, then genuinely connects one while another step is
 * still outstanding (so `isActivated` never latches), keeps a stale waiver that
 * silently comes back into force if the connection is later revoked — masking
 * a step the host still considers owed and that a credential now makes
 * ordinarily completable.
 */
describe("AppShell drops a waiver once the host reports that step complete", () => {
  it("clears the integration waiver when integration connects, without waiting for activation", async () => {
    const client = new Proxy(
      {
        baseUrl: "",
        scopeFor: (company: string | null) => `/api/v1/companies/${company ?? ""}`,
        subscribeToEvents: () => () => {},
        get: (path: string) => {
          if (path.endsWith("/auth/me")) return Promise.resolve({ role: "admin" });
          if (path.endsWith("/activation")) {
            // The waived step is now genuinely done — but `workflow` is not,
            // so the funnel as a whole has NOT activated and the
            // clear-everything branch never runs.
            return Promise.resolve({
              nameConfirmed: true,
              integrationConnected: true,
              workflowRunSucceeded: false,
              isActivated: false,
            });
          }
          return hang();
        },
        status: hang,
        approvals: hang,
        listDesks: hang,
        listTeam: async () => [
          { id: "operations", role: "Analyst", inboxEnabled: false, global: true },
          { id: "ada", role: "Operations", inboxEnabled: false },
        ],
      },
      {
        get(target, prop, receiver) {
          if (prop in target) return Reflect.get(target, prop, receiver);
          return hang;
        },
      },
    ) as unknown as OpenCompanyClient;

    markGateStepWaived(SCOPE, "integration");
    expect(waivedGateSteps(SCOPE)).toContain("integration");

    await act(async () => {
      root.render(
        createElement(HostsProvider, {
          value: HOSTS,
          children: createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(AppShell, {
              client,
              company: STATUS.id,
              initialStatus: STATUS,
              companies: [STATUS],
              onSwitchCompany: () => {},
            }),
          }),
        }),
      );
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });

    expect(
      waivedGateSteps(SCOPE),
      "a waiver for a step the host now reports complete must not survive to speak for a later incomplete one",
    ).not.toContain("integration");
    // And the gate is still up, because `workflow` is genuinely outstanding —
    // clearing the waiver must not be mistaken for finishing the funnel.
    expect(container.querySelector('[data-testid="gate-step-workflow"]')).toBeTruthy();
  });
});

describe("AppShell notices a waiver written by another tab", () => {
  it("closes the gate on a storage event, without a reload or a local waive click", async () => {
    const client = buildClient();

    await act(async () => {
      root.render(
        createElement(HostsProvider, {
          value: HOSTS,
          children: createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(AppShell, {
              client,
              company: STATUS.id,
              initialStatus: STATUS,
              companies: [STATUS],
              onSwitchCompany: () => {},
            }),
          }),
        }),
      );
      // The activation and admin reads route through `withReadTimeout`'s own
      // `Promise.race` on top of the client's plain `Promise.resolve`, so a
      // couple of bare microtask ticks is not always enough to settle both
      // and let the resulting re-render commit — flush generously.
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });

    // Precondition: the gate is up (integration outstanding), not the shell.
    expect(
      container.querySelector("#main-content"),
      "the gate must be showing before the cross-tab waiver lands",
    ).toBeNull();
    expect(container.querySelector('[data-testid="gate-integration-step"]')).toBeTruthy();

    // Another tab durably waives the last outstanding step — written directly
    // to storage, exactly as `markGateStepWaived` does, WITHOUT going through
    // this tab's own `waiveGateStep`.
    await act(async () => {
      markGateStepWaived(SCOPE, "integration");
      window.dispatchEvent(new StorageEvent("storage", { key: "irrelevant-to-the-browser" }));
      await Promise.resolve();
    });

    expect(
      container.querySelector("#main-content"),
      "the gate must close once the storage event is noticed, with no reload",
    ).toBeTruthy();
  });

  /**
   * Codex review, PR #2046, rounds 2 and 4 together.
   *
   * Round 2: `clearGateStepWaivers` fires from ANOTHER tab too, and every
   * `removeItem` it makes is a deletion `storage` event here. Applying it on
   * that tab's word alone drops this tab's waiver against a `status` this tab
   * has not refreshed itself, and the gate reopens over a step the founder
   * already answered.
   *
   * Round 4 narrowed what "has not refreshed itself" may mean. Round 2 assumed
   * every removal meant "some tab saw `isActivated`" — monotonic on the host,
   * so this tab always catches up. Per-step clearing broke that: a removal can
   * now mean "some tab saw THIS STEP complete", and a step can go incomplete
   * again, so an indefinite deferral had no end condition and this tab masked
   * the step for its whole life.
   *
   * What survives is the half that was always the real protection: while this
   * tab cannot READ, it does not act on another tab's word. That is what this
   * test pins — the reads fail outright, so `status` never changes and the
   * removal is never applied.
   */
  it("does not apply another tab's removal while this tab's own reads are failing", async () => {
    let failReads = false;
    const client = new Proxy(
      {
        baseUrl: "",
        scopeFor: (company: string | null) => `/api/v1/companies/${company ?? ""}`,
        subscribeToEvents: () => () => {},
        get: (path: string) => {
          if (path.endsWith("/auth/me")) return Promise.resolve({ role: "admin" });
          if (path.endsWith("/activation")) {
            if (failReads) return Promise.reject(new Error("outage"));
            return Promise.resolve({
              nameConfirmed: true,
              integrationConnected: false,
              workflowRunSucceeded: true,
              isActivated: false,
            });
          }
          return hang();
        },
        status: hang,
        approvals: hang,
        listDesks: hang,
        listTeam: async () => [
          { id: "operations", role: "Analyst", inboxEnabled: false, global: true },
          { id: "ada", role: "Operations", inboxEnabled: false },
        ],
      },
      {
        get(target, prop, receiver) {
          if (prop in target) return Reflect.get(target, prop, receiver);
          return hang;
        },
      },
    ) as unknown as OpenCompanyClient;

    await act(async () => {
      root.render(
        createElement(HostsProvider, {
          value: HOSTS,
          children: createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(AppShell, {
              client,
              company: STATUS.id,
              initialStatus: STATUS,
              companies: [STATUS],
              onSwitchCompany: () => {},
            }),
          }),
        }),
      );
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });
    expect(container.querySelector("#main-content")).toBeNull();

    await act(async () => {
      markGateStepWaived(SCOPE, "integration");
      window.dispatchEvent(new StorageEvent("storage", { key: "irrelevant-to-the-browser" }));
      await Promise.resolve();
    });
    expect(
      container.querySelector("#main-content"),
      "the gate must be closed before the removal arrives, or this test proves nothing",
    ).toBeTruthy();

    // From here this tab can no longer read the funnel at all.
    failReads = true;

    await act(async () => {
      clearGateStepWaivers(SCOPE);
      window.dispatchEvent(new StorageEvent("storage", { key: "irrelevant-to-the-browser" }));
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });

    expect(
      container.querySelector("#main-content"),
      "with no successful read of its own, this tab must not act on another tab's removal",
    ).toBeTruthy();
  });

  /**
   * The other half of round 4: once this tab CAN read, and its own answer says
   * the step is genuinely outstanding, the deferred removal is applied rather
   * than held forever. Before this, `gateWaived` kept a step whose
   * `localStorage` key was already gone and masked it until reload.
   */
  it("applies a deferred removal once its own read confirms the step is still outstanding", async () => {
    const client = buildClient();

    await act(async () => {
      root.render(
        createElement(HostsProvider, {
          value: HOSTS,
          children: createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(AppShell, {
              client,
              company: STATUS.id,
              initialStatus: STATUS,
              companies: [STATUS],
              onSwitchCompany: () => {},
            }),
          }),
        }),
      );
      for (let i = 0; i < 20; i++) await Promise.resolve();
    });

    await act(async () => {
      markGateStepWaived(SCOPE, "integration");
      window.dispatchEvent(new StorageEvent("storage", { key: "irrelevant-to-the-browser" }));
      await Promise.resolve();
    });
    expect(container.querySelector("#main-content")).toBeTruthy();

    // Another tab removes the waiver. This tab keeps reading successfully, and
    // its own answer is that `integration` is still incomplete — so there is
    // no waiver and no completed step: the gate is owed.
    await act(async () => {
      clearGateStepWaivers(SCOPE);
      window.dispatchEvent(new StorageEvent("storage", { key: "irrelevant-to-the-browser" }));
      for (let i = 0; i < 60; i++) await Promise.resolve();
    });

    expect(
      waivedGateSteps(SCOPE),
      "storage is the truth once this tab has read for itself",
    ).not.toContain("integration");
    expect(
      container.querySelector("#main-content"),
      "a removal this tab's own read agrees with must not be deferred forever",
    ).toBeNull();
  });
});
