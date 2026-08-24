// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import type { ConnectionId, LocalScope } from "@/connections/types";
import { SetupController } from "@/setup/SetupController";
import { setupResuming } from "@/setup/state";

const SCOPE: LocalScope = { connection: "test-connection" as ConnectionId, company: null };
const BASELINE: TeamMemberDto[] = ["operations", "page_builder", "researcher", "writer"].map(
  (id) => ({ id, role: "Analyst", inboxEnabled: false, global: true }) as TeamMemberDto,
);

function clientWith(roster: TeamMemberDto[]): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company}`,
    listTeam: async () => roster,
    get: async () => ({ cognition: "echo" }),
    post: async () => ({ agents: [], template: "ecommerce", source: "fallback" }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  window.location.hash = "#/overview";
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

async function mount(client: OpenCompanyClient, deepLinked = false) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: SCOPE,
        children: createElement(SetupController, { client, company: null, deepLinked }),
      }),
    );
  });
}

const dialog = () => document.querySelector('[data-testid="setup-dialog"]');

const modelLink = () =>
  Array.from(document.querySelectorAll("a")).find((a) => a.textContent?.trim() === "Set up a model");

async function goTo(hash: string) {
  await act(async () => {
    window.location.hash = hash;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

async function leaveForModelSettings() {
  const link = modelLink();
  await act(async () => {
    (link as HTMLElement).click();
  });
  await goTo("#/settings/connections");
}

describe("debug exact test", () => {
  it("replicates the reload test", async () => {
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    await act(async () => root.unmount());

    root = createRoot(container);
    await mount(clientWith(BASELINE), true);
    console.log("after mount2 on settings, dialog:", !!dialog(), "resuming:", setupResuming(SCOPE));

    window.location.hash = "#/overview";
    await act(async () => root.unmount());
    root = createRoot(container);
    console.log("after hash set+unmount, resuming:", setupResuming(SCOPE));
    await mount(clientWith(BASELINE), true);

    console.log("after mount3, dialog:", !!dialog(), "resuming:", setupResuming(SCOPE));
    expect(true).toBe(true);
  });
});
