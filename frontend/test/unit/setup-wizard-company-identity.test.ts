// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { SetupStatus } from "@/api/setup";
import { SetupWizard, suggestedCompanyName } from "@/views/setup/SetupWizard";

/**
 * What the wizard sends back about the company itself: its **name**, and
 * whether the thing to build is a template or a designed team.
 *
 * Both were decided silently before. The name was derived host-side from the
 * *industry* answer — a field labelled "what kind of company are you setting
 * up?" — and it mints the company id, which is then permanent and has no
 * rename anywhere in the product. And a picked template was never sent: the
 * wizard only ever posted a designed company, so choosing "Agentic Marketing
 * Agency" and skipping the model produced a rebuilt approximation of it,
 * without the roster, tool belt or prompts that template ships.
 *
 * Mounted rather than pure, for the same earned reason the sibling gate test
 * gives: the claim is about what a submit *carries* after the operator has
 * walked the steps, which only exists once the component is rendering.
 */

const TEMPLATE = {
  id: "agentic_marketing_agency",
  name: "Agentic Marketing Agency",
  agent_count: 8,
  output: "Campaigns across every channel",
};

function status(over: Partial<SetupStatus> = {}): SetupStatus {
  return {
    complete: false,
    config_path: "/data/config.toml",
    fields: [],
    templates: [TEMPLATE],
    // `none` keeps the walk short: it removes the address step, which is the
    // only one that would demand an answer this file is not about.
    auth_modes: ["none", "email"],
    build: {
      acp_in_build: false,
      acp_transport_mounted: false,
      mcp_in_build: false,
      harness_in_build: false,
      oauth_in_build: false,
    },
    companies: [],
    inference: { ready: false, provider: null, base_url: null },
    mail: { wired: false, echoes_code: false },
    ...over,
  };
}

/** The roster the host proposes, and the apply body it is asked for. */
function clientWith(
  s: SetupStatus,
  source: "preset" | "fallback",
  applied: { body?: unknown },
): OpenCompanyClient {
  return {
    get: async () => s,
    post: async (path: string, body: unknown) => {
      if (path.includes("/setup/roster")) {
        return {
          agents: [
            { name: "Creative Director", role: "Creative Director", description: "Concepts." },
            { name: "Copywriter", role: "Copywriter", description: "Words." },
          ],
          template: TEMPLATE.id,
          source,
          jobs: [],
          uncovered: [],
          reason: "no_model",
        };
      }
      if (path === "/api/v1/setup") {
        applied.body = body;
        return {
          complete: true,
          config_path: s.config_path,
          restart_required: [],
          seeded_company: "whatever-they-called-it",
        };
      }
      return {};
    },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function button(label: string): HTMLButtonElement {
  const wanted = label === "Next" ? ["Next", "Looks good"] : [label];
  const match = Array.from(container.querySelectorAll("button")).find((b) =>
    wanted.includes(b.textContent?.trim() ?? ""),
  );
  expect(match, `no button labeled "${label}"`).toBeTruthy();
  return match as HTMLButtonElement;
}

const next = async () =>
  act(async () => {
    button("Next").click();
  });

async function fill(testId: string, value: string) {
  const field = container.querySelector(`[data-testid="${testId}"]`) as
    | HTMLInputElement
    | HTMLTextAreaElement;
  expect(field, `no field ${testId}`).toBeTruthy();
  await act(async () => {
    const proto =
      field instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function pickTemplate(id: string) {
  const select = container.querySelector("select") as HTMLSelectElement;
  expect(select, "no template dropdown").toBeTruthy();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, id);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

/** model -> business -> sign-in (none) -> advanced -> review. */
async function walkToReview(client: OpenCompanyClient, template: string | null) {
  await act(async () => {
    root.render(createElement(SetupWizard, { client, onDone: () => {} }));
  });
  await act(async () => {
    (container.querySelector('[data-testid="setup-skip-model"]') as HTMLElement).click();
  });
  await next(); // -> business
  if (template) await pickTemplate(template);
  else await fill("setup-field-industry", "E-commerce — homeware online");
  await next(); // -> sign-in
  // "No sign-in", which also removes the address step.
  await act(async () => {
    (container.querySelector('[data-testid="auth-mode-none"]') as HTMLElement).click();
  });
  await next(); // -> advanced (or account, on a host that signs people in)
  await next(); // -> review
}

describe("what a finished wizard says the company is", () => {
  it("suggests the template's name, and sends it as the company's", async () => {
    const applied: { body?: unknown } = {};
    await walkToReview(clientWith(status(), "preset", applied), TEMPLATE.id);

    const name = container.querySelector(
      '[data-testid="setup-company-name"]',
    ) as HTMLInputElement;
    expect(name, "the review step must ask what to call it").toBeTruthy();
    expect(name.value).toBe(TEMPLATE.name);

    await act(async () => {
      button("Build my company").click();
    });
    expect((applied.body as { name?: string }).name).toBe(TEMPLATE.name);
  });

  it("sends a name the operator typed over the suggestion", async () => {
    const applied: { body?: unknown } = {};
    await walkToReview(clientWith(status(), "preset", applied), TEMPLATE.id);
    await fill("setup-company-name", "Northwind Studio");

    await act(async () => {
      button("Build my company").click();
    });
    const body = applied.body as { name?: string; template?: string | null };
    expect(body.name).toBe("Northwind Studio");
    // Renaming is not designing: the template is still what gets seeded.
    expect(body.template).toBe(TEMPLATE.id);
  });

  it("sends an untouched template roster back as the template itself", async () => {
    const applied: { body?: unknown } = {};
    await walkToReview(clientWith(status(), "preset", applied), TEMPLATE.id);

    await act(async () => {
      button("Build my company").click();
    });
    const body = applied.body as { template?: string | null; company?: unknown };
    expect(body.template).toBe(TEMPLATE.id);
    expect(
      body.company,
      "a template the host can seed whole must not be rebuilt from this screen",
    ).toBeNull();
  });

  it("still sends a designed company when the roster was matched, not picked", async () => {
    const applied: { body?: unknown } = {};
    // No templates offered, so the step asks for the business in the operator's
    // own words — which is the path the curated roster is matched from.
    await walkToReview(clientWith(status({ templates: [] }), "fallback", applied), null);

    await act(async () => {
      button("Build my company").click();
    });
    const body = applied.body as { template?: string | null; company?: { agents: unknown[] } };
    expect(body.template).toBeNull();
    expect(body.company?.agents).toHaveLength(2);
  });

  it("does not offer to edit a roster it is going to seed whole", async () => {
    await walkToReview(clientWith(status(), "preset", {}), TEMPLATE.id);
    expect(
      container.querySelector('[data-testid="setup-review-remove"]'),
      "an edited template roster could only go back as a designed company, which is capped at six",
    ).toBeNull();
  });
});

describe("the name offered for a company nobody has named", () => {
  it("prefers a picked template's own name", () => {
    expect(suggestedCompanyName("anything at all", "Agentic Law Firm")).toBe("Agentic Law Firm");
  });

  it("takes the first clause of the industry answer, as the host does", () => {
    expect(suggestedCompanyName("E-commerce — I sell homeware online", null)).toBe("E-commerce");
    expect(suggestedCompanyName("Consulting, mostly public sector", null)).toBe("Consulting");
  });

  it("never splits a hyphenated word", () => {
    expect(suggestedCompanyName("E-commerce", null)).toBe("E-commerce");
  });

  it("offers nothing when there is nothing to offer", () => {
    expect(suggestedCompanyName("   ", null)).toBe("");
  });
});
