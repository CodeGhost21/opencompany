// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ObservatoryView } from "@/views/observatory/ObservatoryView";

/**
 * `ObservatoryView` has deep unit coverage for its hash grammar
 * (`observatory-hash.test.ts`, `observatory-model.test.ts`, …) but nothing
 * had mounted the component itself. It carries no admin gate — `Company.
 * agentRuns` is read the same for every session that can open the company at
 * all — so what is worth pinning is the FAIL half: a read that genuinely
 * fails must land on an honest, retryable error rather than an empty run list
 * a reader could mistake for "nothing has run yet", and a refused query
 * (`graphql_refused`) must say so without offering a retry that would only
 * repeat the same refusal.
 */

function clientAs(
  graphqlRequest: () => Promise<{ data?: unknown; errors?: unknown[] }>,
): OpenCompanyClient {
  return {
    graphqlRequest: vi.fn(graphqlRequest),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
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
  vi.restoreAllMocks();
});

describe("ObservatoryView, a genuinely failed read", () => {
  it("shows the failure and offers a retry, not an empty run list that reads as 'nothing has run'", async () => {
    const client = clientAs(() => Promise.reject(new Error("network is offline")));
    await show(
      createElement(ObservatoryView, {
        client,
        company: "acme",
        runId: null,
        eventTick: 0,
      }),
    );
    await act(async () => {});

    expect(container.textContent).toContain("network is offline");
    const retry = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Try again"),
    );
    expect(retry).toBeDefined();
  });

  it("says a refused query was refused, and offers no retry that would only repeat the same refusal", async () => {
    const client = clientAs(() =>
      Promise.resolve({ errors: [{ message: "forbidden" }] }),
    );
    await show(
      createElement(ObservatoryView, {
        client,
        company: "acme",
        runId: null,
        eventTick: 0,
      }),
    );
    await act(async () => {});

    expect(container.textContent).toContain("isn't visible to your account");
    const retry = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Try again"),
    );
    expect(retry).toBeUndefined();
  });
});
