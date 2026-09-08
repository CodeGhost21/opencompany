// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { MemoryEntry, MemoryList, MemoryStats } from "@/api/memory";
import { MemoryView } from "@/views/MemoryView";

/**
 * `memory.rs`'s fact CRUD (`create_fact`, `delete_fact`, list, stats) is all
 * `ScopedCompany` — no admin gate, unlike the engine picker underneath it
 * (`memory_engine.rs`, `AdminScopedCompany`, covered separately). What this
 * file pins: a plain member gets the same create/delete affordances an admin
 * would, matching that route; and a delete that the host actually refuses
 * puts the card straight back and says so — it is never dropped for good on a
 * client that merely believed it worked.
 */

const STATS: MemoryStats = {
  facts: 1,
  factsUpdatedAtMillis: 1000,
  lastUpdatedAtMillis: 1000,
  totalItems: 1,
  teammateMemory: 0,
  documentMemory: 0,
  taskOutcomes: 0,
};

function entry(over: Partial<MemoryEntry> = {}): MemoryEntry {
  return {
    id: "f1",
    kind: "fact",
    origin: "fact",
    editable: true,
    title: "Renewal window",
    body: "Acme renews every March.",
    source: "operator",
    updatedAt: 1000,
    ...over,
  };
}

function clientAs(opts: {
  del?: (path: string) => Promise<void>;
  memoryList?: MemoryEntry[];
}): OpenCompanyClient {
  const list: MemoryList = {
    items: opts.memoryList ?? [entry()],
    totalContext: 0,
    contextTruncated: false,
  } as MemoryList;
  return {
    scopeFor: () => "/api/v1/companies/acme",
    get: (path: string) => {
      if (path.endsWith("/memory/stats")) return Promise.resolve(STATS);
      if (path.endsWith("/memory/engine")) {
        // Admin-only route; a member's own read fails, which `EngineSection`
        // renders as its own error state — not this file's concern.
        return Promise.reject(new ApiError(403, "forbidden", "only an admin can do that"));
      }
      return Promise.resolve(list);
    },
    post: vi.fn(() => Promise.resolve(entry())),
    del: vi.fn(opts.del ?? (() => Promise.resolve())),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
  });
}

function at(testid: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
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

describe("MemoryView, fact CRUD with no admin gate", () => {
  it("offers a plain member both New memory and per-card delete, matching the ungated fact routes", async () => {
    const client = clientAs({});
    await show(createElement(MemoryView, { client, company: "acme" }));
    await act(async () => {});

    expect(at("memory-add")).not.toBeNull();
    const card = at("memory-card")!;
    expect(card.querySelector("button[aria-label='Delete memory']")).not.toBeNull();
  });

  it("puts the card back and says why when the host refuses the delete", async () => {
    const client = clientAs({
      del: () => Promise.reject(new ApiError(409, "conflict", "this fact was already removed")),
    });
    await show(createElement(MemoryView, { client, company: "acme" }));
    await act(async () => {});

    expect(container.textContent).toContain("Renewal window");
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>("button[aria-label='Delete memory']")!
        .click();
    });
    // The optimistic remove and the failed write both settle inside this act.
    expect(container.textContent).toContain("Renewal window");
  });
});
