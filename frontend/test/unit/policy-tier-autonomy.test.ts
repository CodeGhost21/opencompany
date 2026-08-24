// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import { widensAutonomy, gatedBy } from "@/components/policy-settings";

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

/** A status carrying an operator override that differs from the manifest. */
function overridden(mode: string, manifestMode: string): PolicyStatus {
  return {
    ...status(mode),
    manifestMode,
    overridden: true,
    setBy: "someone",
  };
}

function makeClient(initial: PolicyStatus) {
  const put = vi.fn(async (_path: string, body: { mode?: string }) =>
    status(body.mode ?? initial.mode),
  );
  const del = vi.fn(async () => initial);
  return {
    client: {
      scopeFor: () => "/api/v1/acme",
      get: async (path: string) =>
        path.endsWith("/policy") ? initial : { slugs: [], unwired: [] },
      put,
      del,
    } as unknown as OpenCompanyClient,
    put,
    del,
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

/** Types into a field the way an operator does, so React's state updates. */
async function type(box: HTMLInputElement, value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(box, value);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("the autonomy direction", () => {
  it("uses the host's ordered list to identify widening moves", () => {
    expect(widensAutonomy(TIERS, "supervised", "full")).toBe(true);
    expect(widensAutonomy(TIERS, "full", "readonly")).toBe(false);
    expect(widensAutonomy(TIERS, "supervised", "unknown")).toBe(false);
    expect(widensAutonomy(TIERS, "unknown", "full")).toBe(false);
  });

  it("gates the same way the host matcher does", () => {
    expect(gatedBy(["shell"], "shell")).toBe(true);
    expect(gatedBy(["payment"], "payment.send")).toBe(true);
    expect(gatedBy(["payment.send"], "payment")).toBe(false);
    expect(gatedBy(["pay"], "payroll.export")).toBe(false);
    expect(gatedBy(["Shell"], "shell")).toBe(true);
    expect(gatedBy([], "shell")).toBe(false);
    expect(gatedBy([""], "shell")).toBe(false);
  });

  it("folds ASCII case only, like the host matcher", () => {
    // `"Ä".toLowerCase() === "ä"` in JS, but the host's `eq_ignore_ascii_case`
    // compares bytes, so a non-ASCII case pair is NOT a match — the
    // confirmation must not think a fence survives a reset when the gate does
    // not.
    expect(gatedBy(["ä"], "Ä")).toBe(false);
    expect(gatedBy(["shell"], "SHELL")).toBe(true);
  });

  it("shows the looser end of the scale in the console's amber risk tone", async () => {
    await mount(makeClient(status("supervised")).client);
    expect(container.querySelector("[data-testid=policy-tier-auto]")?.className).toContain(
      "status-blocked",
    );
    expect(container.querySelector("[data-testid=policy-tier-full]")?.className).toContain(
      "status-blocked",
    );
    expect(container.textContent).toContain("More autonomy");
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

  it("keeps the dialog up when Escape is pressed while the save is in flight", async () => {
    // The PUT stays unresolved, so `saving` is true while the dialog is open —
    // exactly the window where Base UI forwards an Escape close request.
    let resolvePut: (saved: PolicyStatus) => void = () => {};
    const put = vi.fn(
      async (_path: string) =>
        new Promise<PolicyStatus>((resolve) => {
          resolvePut = resolve;
        }),
    );
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async (path: string) =>
        path.endsWith("/policy") ? status("supervised") : { slugs: [], unwired: [] },
      put,
      del: async () => status("supervised"),
    } as unknown as OpenCompanyClient;
    await mount(client);

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid=policy-tier-full]")!.click();
    });
    await act(async () => {
      document
        .querySelector<HTMLButtonElement>("[data-testid=policy-tier-confirm]")!
        .click();
      await Promise.resolve();
    });
    expect(put).toHaveBeenCalledWith("/api/v1/acme/policy", { mode: "full" });

    // Escape while the PUT is still in flight must not dismiss the dialog:
    // the request is still running and its outcome owns this screen.
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      await Promise.resolve();
    });
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).not.toBeNull();

    // The request resolving (here: succeeding) is what closes the dialog.
    await act(async () => {
      resolvePut(status("full"));
      await Promise.resolve();
    });
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).toBeNull();
  });

  it("drops a pending confirmation when the company changes underneath it", async () => {
    const { client, put } = makeClient(status("supervised"));
    await mount(client);

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid=policy-tier-full]")!.click();
    });
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).not.toBeNull();

    // The scope moves to another company while the dialog is open: the pending
    // choice was reviewed against "acme"'s policy and must not apply to the
    // new one.
    await act(async () => {
      root.render(createElement(PolicySettings, { client, company: "other" }));
      await Promise.resolve();
    });
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).toBeNull();
    expect(put).not.toHaveBeenCalled();
  });

  it("qualifies the always-ask reassurance when edits are unsaved", async () => {
    const { client } = makeClient(status("supervised"));
    await mount(client);

    await type(
      container.querySelector<HTMLInputElement>("#always-approve")!,
      "shell, http_request",
    );
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid=policy-tier-full]")!.click();
    });
    expect(document.body.textContent).toContain(
      "Your saved always-ask list still wins, even on Full — save the list to enforce new gates.",
    );
  });
});

describe("resetting to the manifest's policy", () => {
  it("confirms a reset that restores a more autonomous manifest tier", async () => {
    const { client, del } = makeClient(overridden("readonly", "full"));
    await mount(client);

    await act(async () => {
      const button = [...container.querySelectorAll("button")].find((b) =>
        b.textContent?.includes("manifest's policy"),
      )!;
      button.click();
    });
    expect(del).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain(
      "Give teammates more autonomy?",
    );
    expect(document.body.textContent).toContain(
      "The agents act without asking",
    );
    expect(document.body.textContent).toContain(
      "This also replaces the current always-ask list",
    );
    expect(document.body.textContent).toContain(
      "Reset replaces the whole policy override",
    );

    await act(async () => {
      document
        .querySelector<HTMLButtonElement>("[data-testid=policy-tier-confirm]")!
        .click();
      await Promise.resolve();
    });
    expect(del).toHaveBeenCalledWith("/api/v1/acme/policy");
  });

  it("restores focus to the checked tier after a successful reset", async () => {
    // The DELETE resolves to the manifest state — overridden=false — so the
    // reset button unmounts before the dialog closes, the exact moment the
    // `finalFocus` reset branch must fall back to the checked tier radio.
    const del = vi.fn(async () => status("full"));
    const client = {
      scopeFor: () => "/api/v1/acme",
      get: async (path: string) =>
        path.endsWith("/policy")
          ? overridden("readonly", "full")
          : { slugs: [], unwired: [] },
      put: vi.fn(async () => status("full")),
      del,
    } as unknown as OpenCompanyClient;
    await mount(client);

    await act(async () => {
      const button = [...container.querySelectorAll("button")].find((b) =>
        b.textContent?.includes("manifest's policy"),
      )!;
      button.click();
    });
    await act(async () => {
      document
        .querySelector<HTMLButtonElement>("[data-testid=policy-tier-confirm]")!
        .click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(del).toHaveBeenCalledWith("/api/v1/acme/policy");
    // The override cleared, so the reset button is gone; focus must land on
    // the checked tier radio rather than falling out of the interface.
    expect(
      [...container.querySelectorAll("button")].some((b) =>
        b.textContent?.includes("manifest's policy"),
      ),
    ).toBe(false);
    expect(document.activeElement).toBe(
      document.querySelector("[data-testid=policy-tier-full]"),
    );
  });

  it("confirms a reset that drops an always-ask gate the manifest does not carry", async () => {
    const initial: PolicyStatus = {
      mode: "full",
      alwaysApprove: ["shell"],
      manifestMode: "full",
      manifestAlwaysApprove: [],
      overridden: true,
      setBy: "someone",
      takesEffect: "on the next turn",
      tiers: TIERS,
    };
    const { client, del } = makeClient(initial);
    await mount(client);

    await act(async () => {
      const button = [...container.querySelectorAll("button")].find((b) =>
        b.textContent?.includes("manifest's policy"),
      )!;
      button.click();
    });
    expect(del).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain(
      "Give teammates more autonomy?",
    );
    expect(document.body.textContent).not.toContain("Instead of:");
    expect(document.body.textContent).toContain(
      "This replaces the current always-ask list with the manifest's list: none",
    );
    expect(document.body.textContent).toContain(
      "shell stops always asking for approval",
    );

    await act(async () => {
      document
        .querySelector<HTMLButtonElement>("[data-testid=policy-tier-confirm]")!
        .click();
      await Promise.resolve();
    });
    expect(del).toHaveBeenCalledWith("/api/v1/acme/policy");
  });

  it("keeps a reset that tightens the tier to one click", async () => {
    const { client, del } = makeClient(overridden("full", "readonly"));
    await mount(client);

    await act(async () => {
      const button = [...container.querySelectorAll("button")].find((b) =>
        b.textContent?.includes("manifest's policy"),
      )!;
      button.click();
      await Promise.resolve();
    });
    expect(del).toHaveBeenCalledWith("/api/v1/acme/policy");
    expect(document.querySelector("[data-testid=policy-tier-confirm]")).toBeNull();
  });
});
