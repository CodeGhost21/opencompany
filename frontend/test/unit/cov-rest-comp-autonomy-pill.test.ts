// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import { AutonomyPill } from "@/components/autonomy-pill";
import { useAutonomy } from "@/hooks/use-autonomy";
import { ConsoleProvider } from "@/lib/console-context";

/**
 * Two gaps `autonomy-pill.test.ts` leaves open. Its own list-fidelity and
 * confirm-gate cases fix the tier list at four-or-five entries and never at
 * zero, and none of them puts the write itself under contention with the
 * confirm dialog.
 */

const toasts = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast: { success: toasts.success, error: toasts.error, warning: vi.fn(), info: vi.fn() } }));

function policy(overrides: Partial<PolicyStatus> = {}): PolicyStatus {
  return {
    mode: "auto",
    alwaysApprove: [],
    autoApproveUnderUsd: 5,
    approvalTtlHours: 24,
    manifestMode: "auto",
    manifestAlwaysApprove: [],
    manifestAutoApproveUnderUsd: 5,
    manifestApprovalTtlHours: null,
    overridden: false,
    tiers: [
      { value: "readonly", label: "Read-only", description: "Looks only." },
      { value: "supervised", label: "Supervised", description: "A person signs off." },
      { value: "auto", label: "Auto", description: "Acts on its own." },
      { value: "full", label: "Full", description: "No ceiling." },
    ],
    takesEffect: "on the next turn",
    ...overrides,
  };
}

function client(over: {
  get?: () => Promise<PolicyStatus>;
  put?: (path: string, body: unknown) => Promise<PolicyStatus>;
}): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(over.get ?? (() => Promise.resolve(policy()))),
    put: vi.fn(over.put ?? (() => Promise.resolve(policy()))),
  } as unknown as OpenCompanyClient;
}

function Harness({ api }: { api: OpenCompanyClient }): ReactNode {
  const status = useAutonomy(api, "acme");
  return createElement(ConsoleProvider, {
    client: api,
    company: "acme",
    children: createElement(AutonomyPill, { status, canManage: true }),
  });
}

let container: HTMLDivElement;
let root: Root;

async function mount(api: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(Harness, { api }));
  });
}

function pill(): HTMLElement | null {
  return container.querySelector("[data-testid=autonomy-pill]");
}

async function openMenu() {
  await act(async () => {
    (pill() as HTMLButtonElement).click();
  });
}

function row(mode: string): HTMLElement | null {
  return document.querySelector(`[data-testid=autonomy-tier-${mode}]`);
}

async function pick(mode: string) {
  await act(async () => {
    row(mode)!.click();
  });
}

function confirmButton(): HTMLButtonElement | null {
  return document.querySelector("[data-testid=autonomy-tier-confirm]");
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  toasts.success.mockClear();
  toasts.error.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("the menu never falls back to a hardcoded four tiers", () => {
  it("opens with no tier rows at all when the host names none", async () => {
    // A host that answers `tiers: []` — not four modes this console invented,
    // an empty menu, exactly as the host stated it.
    const api = client({ get: () => Promise.resolve(policy({ tiers: [] })) });
    await mount(api);
    await openMenu();

    for (const mode of ["readonly", "supervised", "auto", "full"]) {
      expect(row(mode), `row(${mode}) should not exist`).toBeNull();
    }
  });

  it("still states the current mode word on the pill itself with no tiers to match it against", async () => {
    const api = client({ get: () => Promise.resolve(policy({ tiers: [], mode: "auto" })) });
    await mount(api);

    expect(pill()!.textContent).toContain("auto");
  });
});

describe("widening is blocked from the title bar until it is accepted", () => {
  it("sends no write while the confirm dialog is still up", async () => {
    const api = client({ get: () => Promise.resolve(policy({ mode: "readonly" })) });
    await mount(api);
    await openMenu();
    await pick("full");

    // The confirm dialog is open; the PUT must not have fired yet.
    expect(confirmButton()).not.toBeNull();
    expect(api.put).not.toHaveBeenCalled();
  });

  it("only writes the wider tier once Confirm is pressed", async () => {
    const api = client({ get: () => Promise.resolve(policy({ mode: "readonly" })) });
    await mount(api);
    await openMenu();
    await pick("full");
    await act(async () => {
      confirmButton()!.click();
    });

    expect(api.put).toHaveBeenCalledTimes(1);
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", { mode: "full" });
  });

  it("skips the confirm dialog entirely for a narrowing pick, and writes at once", async () => {
    const api = client({ get: () => Promise.resolve(policy({ mode: "full" })) });
    await mount(api);
    await openMenu();
    await pick("readonly");

    expect(confirmButton()).toBeNull();
    expect(api.put).toHaveBeenCalledTimes(1);
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", { mode: "readonly" });
  });
});
