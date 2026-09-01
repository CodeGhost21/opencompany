// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import { useAutonomy } from "@/hooks/use-autonomy";

/**
 * What the title row is handed, as distinct from what any one surface knows.
 *
 * The autonomy pill is mounted for the entire life of the console, on every
 * view, and it makes a claim in a sentence: what the agents in the company
 * named an inch to its left are allowed to do without asking. This is a way
 * that claim can be **wrong rather than merely stale**:
 *
 * **A company switch.** `useState` survives a change of `company`; the effect
 * that cleared it is passive, so React ran it AFTER paint. The first frame of
 * the new company therefore carried the previous company's tier — a confident,
 * attributed answer about a different company.
 *
 * Tested through the hook rather than through the pill, because the pill is a
 * faithful renderer of whatever it is given and the bug does not live there.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => ({ toast: Object.assign(toasts.base, toasts) }));

const TIERS = [
  { value: "readonly", label: "Read-only", description: "Looks, changes nothing." },
  { value: "supervised", label: "Supervised", description: "Asks before acting." },
  { value: "auto", label: "Auto", description: "Acts on its own." },
  { value: "full", label: "Full", description: "No ceiling." },
];

function policy(mode: string): PolicyStatus {
  return {
    mode,
    alwaysApprove: [],
    autoApproveUnderUsd: null,
    approvalTtlHours: 24,
    manifestMode: mode,
    manifestAlwaysApprove: [],
    manifestAutoApproveUnderUsd: null,
    manifestApprovalTtlHours: null,
    overridden: false,
    takesEffect: "on the next turn",
    tiers: TIERS,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------

describe("a company switch", () => {
  /**
   * Records what the hook returned **during render**, before any effect runs.
   *
   * That is the only place the bug was visible: `setPolicy(null)` lives in a
   * passive effect, so by the time an assertion could read the DOM after `act`
   * the clear had already happened and the wrong frame was gone. Pushing from
   * the render body keeps it.
   */
  function Probe({
    api,
    company,
    seen,
  }: {
    api: OpenCompanyClient;
    company: string;
    seen: Array<[string, string | null]>;
  }): ReactNode {
    const status = useAutonomy(api, company);
    seen.push([company, status?.mode ?? null]);
    return null;
  }

  it("answers null for the new company rather than the previous company's tier", async () => {
    // Two companies on one host, on different tiers. `globex` is the one that
    // matters: whatever the console says about it must not have come from
    // `acme`.
    const client = {
      scopeFor: (company: string | null) => `/api/v1/company/${company}`,
      get: async (path: string) =>
        path.startsWith("/api/v1/company/acme") ? policy("full") : policy("readonly"),
      put: vi.fn(),
      del: vi.fn(),
    } as unknown as OpenCompanyClient;

    const seen: Array<[string, string | null]> = [];
    await act(async () => {
      root.render(createElement(Probe, { api: client, company: "acme", seen }));
    });
    expect(seen.at(-1)).toEqual(["acme", "full"]);

    await act(async () => {
      root.render(createElement(Probe, { api: client, company: "globex", seen }));
    });

    const forGlobex = seen.filter(([company]) => company === "globex");
    expect(forGlobex.length).toBeGreaterThan(0);
    // THE assertion: the very first frame drawn for `globex`. `full` here is
    // `acme`'s tier being attributed to a company that is on `readonly` — the
    // pill would have said so in a sentence, beside `globex`'s own name.
    expect(forGlobex[0][1], "the first frame of the new company").toBeNull();
    // And no frame of `globex` ever carried it, not just the first.
    expect(forGlobex.map(([, mode]) => mode)).not.toContain("full");
    // Still resolves, so this is a fence and not a hook that stopped answering.
    expect(seen.at(-1)).toEqual(["globex", "readonly"]);
  });
});
