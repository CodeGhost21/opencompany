// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import { widensAutonomy } from "@/components/policy-settings";

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
  {
    value: "readonly",
    label: "Read-only",
    description: "The agents can look at things but change nothing and spend nothing.",
  },
  {
    value: "supervised",
    label: "Supervised",
    description: "The agents ask before every change, including their own scratch files.",
  },
  {
    value: "auto",
    label: "Auto",
    description: "The agents work on their own and stop before anything that leaves the company or spends money.",
  },
  {
    value: "full",
    label: "Full",
    description: "The agents act without asking, except for the few things on the always-ask list.",
  },
];

function status(mode: string): PolicyStatus {
  return {
    mode,
    alwaysApprove: ["shell"],
    manifestMode: mode,
    manifestAlwaysApprove: ["shell"],
    overridden: false,
    takesEffect: "on the next turn",
    tiers: TIERS,
  };
}

function makeClient(initial: PolicyStatus) {
  const put = vi.fn(async (_path: string, body: { mode?: string }) =>
    status(body.mode ?? initial.mode),
  );
  return {
    client: {
      scopeFor: () => "/api/v1/acme",
      get: async (path: string) =>
        path.endsWith("/policy") ? initial : { slugs: [], unwired: [] },
      put,
      del: async () => initial,
    } as unknown as OpenCompanyClient,
    put,
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

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(PolicySettings, { client, company: "acme" }));
    await Promise.resolve();
  });
}

describe("the autonomy direction", () => {
  it("uses the host's ordered list to identify widening moves", () => {
    expect(widensAutonomy(TIERS, "supervised", "full")).toBe(true);
    expect(widensAutonomy(TIERS, "full", "readonly")).toBe(false);
    expect(widensAutonomy(TIERS, "supervised", "unknown")).toBe(false);
    expect(widensAutonomy(TIERS, "unknown", "full")).toBe(false);
  });

  it("shows the looser end of the scale in the console's amber risk tone", async () => {
    await mount(makeClient(status("supervised")).client);
    expect(container.querySelector("[data-testid=policy-tier-auto]")?.className).toContain(
      "status-blocked",
    );
    expect(container.querySelector("[data-testid=policy-tier-full]")?.className).toContain(
      "status-blocked",
    );
    expect(container.textContent).toContain("More freedom to act");
  });
});

describe("changing the autonomy tier", () => {
  it("confirms a widening move with the before-and-after consequences", async () => {
    const { client, put } = makeClient(status("supervised"));
    await mount(client);

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid=policy-tier-full]")!.click();
    });
    expect(put).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("The agents ask before every change");
    expect(document.body.textContent).toContain("The agents act without asking");
    expect(document.body.textContent).toContain("always-ask list still wins");

    await act(async () => {
      document.querySelector<HTMLButtonElement>("[data-testid=policy-tier-confirm]")!.click();
      await Promise.resolve();
    });
    expect(put).toHaveBeenCalledWith("/api/v1/acme/policy", { mode: "full" });
  });

  it("keeps a narrowing move to one click", async () => {
    const { client, put } = makeClient(status("full"));
    await mount(client);

    await act(async () => {
      container
        .querySelector<HTMLButtonElement>("[data-testid=policy-tier-supervised]")!
        .click();
      await Promise.resolve();
    });
    expect(put).toHaveBeenCalledWith("/api/v1/acme/policy", { mode: "supervised" });
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).toBeNull();
  });
});
