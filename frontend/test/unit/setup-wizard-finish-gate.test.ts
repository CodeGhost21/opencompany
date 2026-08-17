// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { SetupStatus } from "@/api/setup";
import { SetupWizard } from "@/views/setup/SetupWizard";

/**
 * The zero-company dead end (CodeRabbit review on #908): a host with no
 * companies must not be able to finish setup without picking a template,
 * because that is exactly the "no companies running, no way back into setup"
 * dead end the flow exists to remove.
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

async function goToReview() {
  // template -> signin -> brain -> tools -> host -> review
  for (let i = 0; i < 5; i++) {
    await act(async () => {
      button("Next").click();
    });
  }
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

describe("finishing setup with no companies on the host", () => {
  it("disables Finish setup until a template is picked", async () => {
    await show(clientWith(status()));
    await goToReview();

    const finish = container.querySelector('[data-testid="setup-finish"]') as HTMLButtonElement;
    expect(finish.disabled).toBe(true);
    expect(container.querySelector('[data-testid="review-no-company-warning"]')).toBeTruthy();
  });

  it("enables Finish setup once a template is selected", async () => {
    await show(clientWith(status()));

    await act(async () => {
      (
        container.querySelector('[data-testid="template-starter"]') as HTMLButtonElement
      ).click();
    });
    await goToReview();

    const finish = container.querySelector('[data-testid="setup-finish"]') as HTMLButtonElement;
    expect(finish.disabled).toBe(false);
    expect(container.querySelector('[data-testid="review-no-company-warning"]')).toBeNull();
  });

  it("does not gate finishing when the host already has a company", async () => {
    await show(clientWith(status({ companies: ["acme"] })));
    await goToReview();

    const finish = container.querySelector('[data-testid="setup-finish"]') as HTMLButtonElement;
    expect(finish.disabled).toBe(false);
  });
});
