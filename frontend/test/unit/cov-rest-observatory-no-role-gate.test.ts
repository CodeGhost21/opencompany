// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ObservatoryView } from "@/views/observatory/ObservatoryView";

/**
 * `Company.agentRuns` (the GraphQL field this view reads) is answered the
 * same for every session that can open the company at all — there is no
 * admin-only slice of run history the way Settings gates a write. This pins
 * that the view never adds a role check of its own: no `/auth/me`-shaped
 * `client.get` call precedes the read, so a member sees exactly what an
 * admin does.
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("the observatory reads run history with no role check of its own", () => {
  it("never calls a member/admin resolving read before fetching runs", async () => {
    const get = vi.fn(() => Promise.reject(new Error("no REST reads expected here")));
    const graphqlRequest = vi.fn(() => Promise.resolve({ data: { runs: [] } }));
    const client = { get, graphqlRequest } as unknown as OpenCompanyClient;

    await act(async () => {
      root.render(
        createElement(ObservatoryView, { client, company: "acme", runId: null, eventTick: 0 }),
      );
    });
    await act(async () => {});

    expect(graphqlRequest).toHaveBeenCalled();
    expect(get).not.toHaveBeenCalled();
  });
});
