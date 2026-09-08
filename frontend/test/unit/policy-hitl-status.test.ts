// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import { autonomyPromptsNote } from "@/components/policy-settings";

/**
 * The console must state whether policy-generated approvals (the always-ask
 * list, the spend cap, the tier) are actually creating approval cards, and it
 * must read that fact from the host's own `policyHitlEnabled` rather than
 * assume it. Every deployed build reports it `false` today
 * (`src/runtime/builder.rs` disables it unconditionally), which is why every
 * other spec in this directory constructs a `PolicyStatus` with the field
 * omitted and still sees the disabled copy — that is the correct fallback,
 * not a hardcoded assumption. These specs pin the other half: what the card
 * says when the host reports the gate is actually live, which the component
 * could not previously say at all.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, toasts);
  return { toast };
});

const { PolicySettings } = await import("@/components/policy-settings");

const TIERS = [
  { value: "supervised", label: "Supervised", description: "Conservative execution restrictions." },
  { value: "full", label: "Full", description: "Broadest execution autonomy." },
];

function status(overrides: Partial<PolicyStatus> = {}): PolicyStatus {
  return {
    mode: "supervised",
    alwaysApprove: ["shell"],
    autoApproveUnderUsd: null,
    approvalTtlHours: 24,
    manifestMode: "supervised",
    manifestAlwaysApprove: ["shell"],
    manifestAutoApproveUnderUsd: null,
    manifestApprovalTtlHours: null,
    overridden: false,
    takesEffect: "on the next turn",
    tiers: TIERS,
    ...overrides,
  };
}

function makeClient(initial: PolicyStatus) {
  return {
    scopeFor: () => "/api/v1/acme",
    get: async (path: string) =>
      path.endsWith("/policy") ? initial : { slugs: [], unwired: [] },
    put: vi.fn(async () => initial),
    del: vi.fn(async () => initial),
  } as unknown as OpenCompanyClient;
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

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(PolicySettings, { client, company: "acme", canManage: true }));
    await Promise.resolve();
  });
}

describe("stating whether policy HITL is actually live", () => {
  it("reads the disabled state from the host rather than assuming it", async () => {
    await mount(makeClient(status({ policyHitlEnabled: false })));
    const banner = container.querySelector('[data-testid="policy-hitl-status"]');
    expect(banner?.textContent).toContain("Policy-based approval prompts are disabled");
    expect(banner?.textContent).toContain("paid-media");
    expect(banner?.textContent).toContain("requires_approval");
    expect(container.querySelector<HTMLInputElement>("#spend-cap")?.disabled).toBe(true);
    expect(container.querySelector<HTMLInputElement>("#always-approve")?.disabled).toBe(true);
    expect(container.textContent).toContain("Spend approval threshold (inactive)");
    expect(container.textContent).toContain("Always ask first (inactive)");
  });

  it("falls back to the disabled state on a host that predates the field", async () => {
    // No `policyHitlEnabled` key at all — the same shape every other test
    // fixture in this directory sends.
    await mount(makeClient(status()));
    const banner = container.querySelector('[data-testid="policy-hitl-status"]');
    expect(banner?.textContent).toContain("Policy-based approval prompts are disabled");
    expect(container.querySelector<HTMLInputElement>("#always-approve")?.disabled).toBe(true);
  });

  it("renders the always-ask list and spend cap as live when the host reports the gate is on", async () => {
    await mount(makeClient(status({ policyHitlEnabled: true })));
    const banner = container.querySelector('[data-testid="policy-hitl-status"]');
    expect(banner?.textContent).toContain("Policy-based approval prompts are active");
    // The "inactive" labels must not survive onto a company whose gate really
    // is turning always_approve into approval cards.
    expect(container.textContent).not.toContain("(inactive)");
    expect(container.querySelector<HTMLInputElement>("#always-approve")?.disabled).toBe(false);
    expect(container.querySelector<HTMLInputElement>("#spend-cap")?.disabled).toBe(false);
  });

  it("lets an operator actually save the always-ask list once the host reports the gate is live", async () => {
    const client = makeClient(status({ policyHitlEnabled: true }));
    await mount(client);
    const input = container.querySelector<HTMLInputElement>("#always-approve")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        window.HTMLInputElement.prototype,
        "value",
      )?.set;
      setter?.call(input, "shell, http_request");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const save = [...container.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Save list"),
    );
    expect(save?.disabled).toBe(false);
    await act(async () => {
      save?.click();
      await Promise.resolve();
    });
    expect(client.put).toHaveBeenCalledWith("/api/v1/acme/policy", {
      alwaysApprove: ["shell", "http_request"],
    });
  });

  it("keeps the matcher note from claiming anything about live gating either way", async () => {
    // The "is this a real tool?" note is a naming check, not a gating claim,
    // and it must read the same regardless of whether the gate is live.
    for (const policyHitlEnabled of [true, false]) {
      await mount(makeClient(status({ policyHitlEnabled, knownTools: ["shell"] })));
      const input = container.querySelector<HTMLInputElement>("#always-approve")!;
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(
          window.HTMLInputElement.prototype,
          "value",
        )?.set;
        setter?.call(input, "typo_tool");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(container.textContent).not.toContain("is gated today");
      expect(container.textContent).not.toContain("this is gated");
    }
  });
});

describe("autonomyPromptsNote", () => {
  it("names what still applies once the tier changes, honestly, in both states", () => {
    expect(autonomyPromptsNote(false)).toBe(
      "Approval prompts remain explicit through request_approval.",
    );
    expect(autonomyPromptsNote(true)).not.toBe(
      "Approval prompts remain explicit through request_approval.",
    );
    expect(autonomyPromptsNote(true).length).toBeGreaterThan(0);
  });
});
