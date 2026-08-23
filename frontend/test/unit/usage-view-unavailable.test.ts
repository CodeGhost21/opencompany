// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { UsageDto } from "@/api/types";
import { UsageView } from "@/views/UsageView";

const ZERO_USAGE: UsageDto = {
  series: [],
  byAgent: [],
  byProvider: [],
  totals: {
    inputTokens: 0,
    outputTokens: 0,
    tokens: 0,
    costUsd: 0,
    oauthCalls: 0,
    connections: 0,
    searchCalls: 0,
  },
};

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(UsageView, { client, company: null }));
  });
}

function text(): string {
  return document.body.textContent ?? "";
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

describe("UsageView on a host without usage surfaces", () => {
  it("does not present rejected usage and capability reads as zero or unconfigured", async () => {
    await render({
      usage: async () => {
        throw new Error("usage route not found");
      },
      capabilityStatus: async () => {
        throw new Error("capability route not found");
      },
    } as unknown as OpenCompanyClient);

    expect(document.querySelector('[data-testid="usage-unavailable"]')?.textContent).toContain(
      "does not report usage",
    );
    expect(document.querySelector('[data-testid="capability-status-unavailable"]')?.textContent).toContain(
      "does not report capability status",
    );
    expect(text()).toContain("Total tokens—");
    expect(text()).toContain("Cost—");
    expect(text()).not.toContain("No token plan configured.");
  });

  it("keeps a successful zero-usage read distinct from an unavailable one", async () => {
    await render({
      usage: async () => ZERO_USAGE,
      capabilityStatus: async () => ({ configured: false }),
    } as unknown as OpenCompanyClient);

    expect(document.querySelector('[data-testid="usage-unavailable"]')).toBeNull();
    expect(text()).toContain("Total tokens0");
    expect(text()).toContain("Cost$0.00");
    expect(text()).toContain("No token plan configured.");
  });
});
