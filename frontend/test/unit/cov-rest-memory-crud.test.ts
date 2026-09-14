// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { MemoryEntry, MemoryStats } from "@/api/memory";
import { MemoryView } from "@/views/MemoryView";

/**
 * The Brain's fact CRUD has zero role gate of its own — `POST`/`DELETE
 * …/memory` are `ScopedCompany`, not admin-only — so the authority this axis
 * actually tests is per-row: `editable` (`api/memory.ts`) is what separates an
 * operator's own fact, which Delete may touch, from the agents' own read-only
 * memory, which must never offer one. Paired with the CRUD path itself, which
 * had no test at all: a delete that the host refuses must put the card back,
 * not leave it silently gone.
 */

function entry(over: Partial<MemoryEntry> = {}): MemoryEntry {
  return {
    id: "m1",
    origin: "fact",
    kind: "fact",
    editable: true,
    title: "Client prefers Friday reviews",
    body: "Always confirm before Thursday.",
    source: "operator",
    updatedAt: 0,
    ...over,
  };
}

const STATS: MemoryStats = {
  facts: 1,
  factsUpdatedAtMillis: 0,
  lastUpdatedAtMillis: 0,
  totalItems: 1,
  teammateMemory: 0,
  documentMemory: 0,
  taskOutcomes: 0,
};

function clientWith(opts: {
  entries: MemoryEntry[];
  del?: () => Promise<void>;
}): OpenCompanyClient {
  const get = vi.fn((path: string) => {
    if (path.endsWith("/memory/stats")) return Promise.resolve(STATS);
    if (path.endsWith("/memory/engine")) return Promise.reject(new Error("no engine route"));
    if (path.includes("/memory")) {
      return Promise.resolve({ items: opts.entries, totalContext: 0, contextTruncated: false });
    }
    return Promise.reject(new Error(`unexpected GET ${path}`));
  });
  return {
    scopeFor: () => "/api/v1/company/acme",
    get,
    post: vi.fn(() => Promise.resolve(entry())),
    del: vi.fn(opts.del ?? (() => Promise.resolve())),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(MemoryView, { client, company: "acme" }));
  });
}

function cards(): HTMLElement[] {
  return Array.from(container.querySelectorAll('[data-testid="memory-card"]'));
}

function deleteButtonIn(card: HTMLElement): HTMLElement | null {
  return card.querySelector('[aria-label="Delete memory"]');
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

describe("delete is offered per row, by what the row itself is", () => {
  it("offers no delete control on the agents' own read-only memory", async () => {
    const client = clientWith({
      entries: [entry({ id: "ctx1", origin: "agent-memory", editable: false, title: "Learned fact" })],
    });
    await show(client);

    const card = cards()[0];
    expect(card).toBeTruthy();
    expect(deleteButtonIn(card)).toBeNull();
  });

  it("offers delete on an operator-authored fact", async () => {
    const client = clientWith({ entries: [entry()] });
    await show(client);

    const card = cards()[0];
    expect(deleteButtonIn(card)).not.toBeNull();
  });
});

describe("a refused delete puts the card back, honestly", () => {
  it("re-inserts the entry and reports the failure when the host refuses the delete", async () => {
    const client = clientWith({
      entries: [entry()],
      del: () => Promise.reject(new Error("memory engine is unreachable")),
    });
    await show(client);

    await act(async () => {
      deleteButtonIn(cards()[0])!.click();
    });

    // A false success is exactly what this file exists to catch: the card
    // must come back once the host has actually refused the write.
    expect(cards()).toHaveLength(1);
    expect(container.textContent).toContain("Client prefers Friday reviews");
  });
});
