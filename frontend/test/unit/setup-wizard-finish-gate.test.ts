// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { SetupStatus } from "@/api/setup";
import { SetupWizard } from "@/views/setup/SetupWizard";

/**
 * The zero-company dead end (CodeRabbit review on #908): a host with no
 * companies must not be able to finish setup without a company to finish
 * *into*, because that is exactly the "no companies running, no way back into
 * setup" dead end the flow exists to remove.
 *
 * The **condition** changed when the two setups merged — it used to be "a
 * template was picked", and is now "a roster was designed and reviewed" —
 * but the invariant is the same one and is why this file still exists.
 *
 * A pure test cannot reach this — the claim is about a *button's disabled
 * state* changing as the operator moves through the wizard, which only
 * exists once the component is mounted and rendering. Same earned exception
 * as `provider-detail-render` and `working-indicator`.
 */

function status(over: Partial<SetupStatus> = {}): SetupStatus {
  return {
    complete: false,
    config_path: "/data/config.toml",
    fields: [],
    templates: [
      { id: "starter", name: "Starter", agent_count: 2, output: null },
    ],
    auth_modes: ["email"],
    build: {
      acp_in_build: false,
      acp_transport_mounted: false,
      mcp_in_build: false,
      harness_in_build: false,
      oauth_in_build: false,
    },
    companies: [],
    ...over,
  };
}

function clientWith(s: SetupStatus): OpenCompanyClient {
  return {
    get: async () => s,
    post: async () => ({
      complete: true,
      config_path: s.config_path,
      restart_required: [],
      seeded_company: null,
    }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(SetupWizard, { client, onDone: () => {} }));
  });
}

function button(label: string): HTMLButtonElement {
  const buttons = Array.from(container.querySelectorAll("button"));
  const match = buttons.find((b) => b.textContent?.trim() === label);
  expect(match, `no button labeled "${label}"`).toBeTruthy();
  return match as HTMLButtonElement;
}

/** Types into a step's field, so a required one can be left. */
async function fill(testId: string, value: string) {
  const field = container.querySelector(`[data-testid="${testId}"]`) as
    | HTMLInputElement
    | HTMLTextAreaElement;
  expect(field, `no field ${testId}`).toBeTruthy();
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      field instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** business -> team -> automate -> account -> power -> review. */
async function goToReview() {
  await fill("setup-field-industry", "E-commerce — homeware");
  // business -> team -> automate -> account
  for (let i = 0; i < 3; i++) {
    await act(async () => {
      button("Next").click();
    });
  }
  // The address is required on any host that asks people to sign in — leaving
  // it blank holds the wizard here, which is its own assertion below.
  await fill("setup-field-email", "ada@example.com");
  // account -> power -> review
  for (let i = 0; i < 2; i++) {
    await act(async () => {
      button("Next").click();
    });
  }
  // Entering Review kicks off the design call. Let it settle, or the assertions
  // below run against the spinner rather than the outcome.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const finishButton = () =>
  container.querySelector('[data-testid="setup-finish"]') as HTMLButtonElement | null;

describe("finishing setup with no companies on the host", () => {
  /**
   * The design call fails in this environment (no host behind the client), so
   * Review renders its error rather than a roster — which is precisely the
   * state that must not be finishable: there is no team to build, and applying
   * would leave a configured instance with nothing to sign in to.
   */
  it("refuses to finish when no team was designed", async () => {
    await show(clientWith(status()));
    await goToReview();

    expect(container.querySelector('[data-testid="setup-design-error"]')).toBeTruthy();
    expect(finishButton()?.disabled).toBe(true);
  });

  /**
   * The first question is the only required one, and it gates leaving step one
   * — so an operator cannot skip past every screen into a company with no
   * description behind it.
   */
  it("will not leave the first question empty", async () => {
    await show(clientWith(status()));

    await act(async () => {
      button("Next").click();
    });
    expect(container.querySelector('[data-testid="setup-problem"]')).toBeTruthy();
    // Still on the first question.
    expect(
      container.querySelector('[data-testid="setup-field-industry"]'),
    ).toBeTruthy();
  });

  /**
   * The other half of the dead end, and the one that was actually reachable in
   * shipped code: an operator who finishes setup on an email-sign-in host
   * without an address can then sign in as nobody, because no shipped template
   * invites anyone.
   */
  it("will not pass the email step on a host that asks people to sign in", async () => {
    await show(clientWith(status()));
    await fill("setup-field-industry", "E-commerce — homeware");
    for (let i = 0; i < 3; i++) {
      await act(async () => {
        button("Next").click();
      });
    }

    await act(async () => {
      button("Next").click();
    });
    expect(container.querySelector('[data-testid="setup-problem"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="setup-field-email"]')).toBeTruthy();
  });

  /**
   * A host that already serves a company is not at risk of the dead end: there
   * is somewhere to sign in to regardless of what this flow does, so an
   * operator reconfiguring one may finish without designing anything.
   */
  it("does not gate finishing when the host already has a company", async () => {
    await show(clientWith(status({ companies: ["acme"] })));
    await goToReview();

    expect(finishButton()?.disabled).toBe(false);
  });
});
