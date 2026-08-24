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

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  window.location.hash = "#/overview";
  container = document.createElement("div");
  document.body.appendChild(container);
});

afterEach(() => {
  container.remove();
  localStorage.clear();
});

async function mount(client: OpenCompanyClient, deepLinked = false) {
  await act(async () => {
    createRoot(container).render(
      createElement(ConnectionScopeProvider, {
        scope: SCOPE,
        children: createElement(SetupController, { client, company: null, deepLinked }),
      }),
    );
  });
}

describe("debug reload", () => {
  it("traces debt across settings reload", async () => {
    window.location.hash = "#/overview";
    const root = createRoot(container);
    await act(async () => {
      root.render(
        createElement(ConnectionScopeProvider, {
          scope: SCOPE,
          children: createElement(SetupController, { client: clientWith(BASELINE), company: null }),
        }),
      );
    });
    // simulate leave: mark resuming
    const { markSetupResuming } = await import("@/setup/state");
    markSetupResuming(SCOPE);
    await act(async () => root.unmount());
    console.log("after mount1+leave, resuming:", setupResuming(SCOPE));

    // mount #2 on settings
    window.location.hash = "#/settings/connections";
    const root2 = createRoot(container);
    await act(async () => {
      root2.render(
        createElement(ConnectionScopeProvider, {
          scope: SCOPE,
          children: createElement(SetupController, { client: clientWith(BASELINE), company: null, deepLinked: true }),
        }),
      );
    });
    console.log("after mount2, resuming:", setupResuming(SCOPE));

    window.location.hash = "#/overview";
    console.log("after hash set, resuming:", setupResuming(SCOPE));
    await act(async () => root2.unmount());
    console.log("after unmount2, resuming:", setupResuming(SCOPE));

    const root3 = createRoot(container);
    await act(async () => {
      root3.render(
        createElement(ConnectionScopeProvider, {
          scope: SCOPE,
          children: createElement(SetupController, { client: clientWith(BASELINE), company: null, deepLinked: true }),
        }),
      );
    });
    console.log("after mount3, resuming:", setupResuming(SCOPE));
    expect(true).toBe(true);
  });
});
