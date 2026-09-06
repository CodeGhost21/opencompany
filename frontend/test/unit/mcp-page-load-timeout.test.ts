// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_REQUEST_TIMEOUT_MS, OpenCompanyClient } from "@/api/client";
import type {
  StreamHandlers,
  Transport,
  TransportRequest,
  TransportResponse,
} from "@/api/transport";
import { McpServersSection } from "@/views/connections/McpServersSection";

/**
 * The MCP servers page against a stalled host (issue #2014).
 *
 * `McpServersSection` has carried a real load-error state all along — it just
 * never reached it, because `listMcpServers` → `client.get` sat on a `fetch`
 * that never settled and the section's `catch` never ran, leaving the page on
 * its loading skeleton forever. The section is unchanged by the fix; the shared
 * request deadline turns the stall into the rejection its existing `catch`
 * already knows how to render.
 *
 * On the pre-fix client this test fails: the skeleton is still on screen and
 * `mcp-load-error` never appears after the deadline passes.
 */

/** A transport that accepts every request and answers none. */
class StallingTransport implements Transport {
  request(_req: TransportRequest): Promise<TransportResponse> {
    return new Promise<TransportResponse>(() => {});
  }
  subscribe(_url: string, _handlers: StreamHandlers): () => void {
    return () => {};
  }
}

function stalledClient(): OpenCompanyClient {
  return new OpenCompanyClient(
    { baseUrl: "https://host.test", company: "acme", operatorToken: null, sessionHeader: null },
    new StallingTransport(),
  );
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function at(testid: string): HTMLElement | null {
  return container.querySelector(`[data-testid="${testid}"]`);
}

describe("MCP servers page on a stalled host", () => {
  it("replaces the skeleton with a load error once the read deadline passes", async () => {
    const client = stalledClient();

    await act(async () => {
      root.render(
        createElement(McpServersSection, {
          client,
          company: "acme",
          canManage: false,
          chrome: "standalone",
        }),
      );
    });

    // Before the deadline: the loading skeleton, and no error yet.
    expect(container.querySelector(".h-24")).not.toBeNull();
    expect(at("mcp-load-error")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS);
    });

    // After the deadline: the error state the section always had, no skeleton.
    expect(at("mcp-load-error")).not.toBeNull();
    expect(at("mcp-load-error")?.textContent).toContain("Couldn't load");
    expect(container.querySelector(".h-24")).toBeNull();
  });
});
