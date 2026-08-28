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
 * PR #1875 review finding, round 15.
 *
 * `useActivationGate` never settles `checked` on a non-terminal
 * `getActivation` failure — it retries forever, by design, because a blip
 * must not be mistaken for an answer. `shouldHoldShellPending` holds the
 * shell for as long as `checked` is false, and the pending branch renders
 * only `<RouteLoading>`.
 *
 * Put together, a *durable* backend fault — a malformed event that fails the
 * host's whole-journal activation scan on every read — locked the operator
 * out of the entire console behind a loader that could never resolve. The
 * "skip for now" escape exists only inside `OnboardingGate`, which this
 * branch never mounts, so there was no way forward at all.
 *
 * The fix gives the hook a `stuck` signal after `STUCK_AFTER_FAILURES`
 * consecutive failures and renders a recovery affordance instead of the
 * loader. Polling continues underneath, so a backend that recovers still
 * settles the gate on its own.
 */

const SCOPE: LocalScope = { connection: "test-connection" as ConnectionId, company: null };

const STATUS: CompanyStatus = {
  id: "co",
  name: "Acme",
  lifecycle: "running",
  pending_approvals: 0,
};

/** Staffed, so `SetupController` closes and `setupChecked` lands true. */
const STAFFED: TeamMemberDto[] = [
  { id: "operations", role: "Analyst", inboxEnabled: false, global: true } as TeamMemberDto,
  { id: "ada", role: "Operations", inboxEnabled: false } as TeamMemberDto,
];

function hang(): Promise<never> {
  return new Promise<never>(() => {});
}

/**
 * `/activation` fails every time with a non-terminal error — the shape
 * `resolveActivationReadError` does NOT settle, so the hook retries rather
 * than answering. `/auth/me` is left hanging: an unresolved admin check does
 * not release the hold, which keeps this test aimed at the activation read
 * alone.
 */
function buildClient(activationCalls: { count: number }): OpenCompanyClient {
  const known = {
    baseUrl: "",
    scopeFor: (company: string | null) => `/api/v1/companies/${company ?? ""}`,
    listTeam: vi.fn(async () => STAFFED),
    subscribeToEvents: () => () => {},
    get: (path: string) => {
      if (path.includes("/activation")) {
        activationCalls.count += 1;
        return Promise.reject(new Error("activation scan failed"));
      }
      return hang();
    },
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
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  vi.useRealTimers();
  act(() => root.unmount());
  container.remove();
});

describe("a durable activation-read failure does not strand the operator", () => {
  it("offers a way into the console once the read keeps failing", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const activationCalls = { count: 0 };
    const client = buildClient(activationCalls);

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

    // One failure is routine: still the neutral loader, no error shown.
    expect(container.textContent).toContain("Loading");
    expect(container.textContent).not.toContain("Continue to the console");

    // Drive the retries. Each non-terminal failure schedules the next attempt
    // at ACTIVATION_READ_RETRY_MS; STUCK_AFTER_FAILURES of them flips `stuck`.
    for (let i = 0; i < 4; i += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
    }

    expect(activationCalls.count).toBeGreaterThanOrEqual(3);
    // The operator must now have a way forward rather than an endless loader.
    expect(container.textContent).toContain("Continue to the console");
  });
});
