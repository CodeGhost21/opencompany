// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";

/**
 * Issue #1313 / PR review: the "No desks yet" empty state must only fire for
 * a genuinely empty company, not for one whose `GET /desks` merely failed.
 *
 * `Overview` reads six sources best-effort. A rejected `/desks` is normalized
 * to `[]` so the other five rings can still draw — but that same `[]` used to
 * satisfy the empty-state condition, overlaying "No desks yet" and hiding the
 * graph controls over a company that may well have desks whose read just
 * failed. The graph keeps `emptyState` off unless the desks read itself was
 * fulfilled.
 */

// Captured on every render of the (mocked) graph so a test can read what
// `Overview` actually decided.
const { graphProps } = vi.hoisted(() => ({
  graphProps: { emptyState: false },
}));

vi.mock("@/views/overview/kg/KnowledgeGraph", () => ({
  KnowledgeGraph: (props: { emptyState?: boolean }) => {
    graphProps.emptyState = !!props.emptyState;
    return null;
  },
}));

const { Overview } = await import("@/views/Overview");

/** Every `client.get` path this component reads, keyed by its suffix. */
const HEALTHY_GET: Record<string, unknown> = {
  "/tasks": [],
  "/users": [],
  // `GET /memory` answers with `{ items, totalContext, contextTruncated }`.
  "/memory": { items: [], totalContext: 0, contextTruncated: false },
  "/workflows": [],
};

/**
 * A fake host whose other five reads all succeed, so the only variable under
 * test is the desks read. Same shape as the sibling `overview-unreachable-host`
 * fixture: `get` is dispatched by path suffix, desks and team are their own
 * client methods.
 */
function fakeClient() {
  const get = vi.fn((path: string) => {
    const suffix = Object.keys(HEALTHY_GET).find((k) => path.endsWith(k));
    return Promise.resolve(suffix ? HEALTHY_GET[suffix] : []);
  });
  const listDesks = vi.fn().mockResolvedValue([]);
  const listTeam = vi.fn().mockResolvedValue([]);
  const client = {
    scopeFor: () => "/api/v1/company/acme",
    get,
    listDesks,
    listTeam,
  } as unknown as OpenCompanyClient;
  return { client, listDesks };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  graphProps.emptyState = false;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(host: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(Overview, { client: host, company: "acme" }));
  });
  // The six sources resolve as one `Promise.allSettled`, but the state it sets
  // lands a tick later; give React that tick rather than assuming one flush
  // covers it.
  await act(async () => {});
  await act(async () => {});
}

describe("the overview empty state", () => {
  it("appears only for a company a successful desks read found empty", async () => {
    const { client } = fakeClient();
    await render(client);

    // `listDesks` answered `[]` and nothing else failed: genuinely no desks.
    expect(graphProps.emptyState).toBe(true);
  });

  it("stays hidden when the desks read failed, even if the other reads answered", async () => {
    const mocks = fakeClient();
    mocks.listDesks.mockRejectedValue(new Error("desks unavailable"));

    await render(mocks.client);

    // The company may well have desks; the request just failed. Drawing the
    // graph without pillars is honest — claiming "No desks yet" is not.
    expect(graphProps.emptyState).toBe(false);
  });
});
