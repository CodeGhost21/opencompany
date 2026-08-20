// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { LedgerList, LedgerSummary } from "@/api/ledgers";
import { useLedgerNav } from "@/hooks/use-ledger-nav";

const GOALS: LedgerSummary = {
  slug: "goals",
  title: "Goals",
  purpose: "",
  source: "events",
  derived: "derived/GOALS.md",
  writtenBy: "",
  builtin: true,
  fields: [],
  statuses: [{ name: "open" }],
  sections: [],
  open: 1,
  closed: 0,
};

function fakeClient(get: (path: string) => Promise<unknown>): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async (path: string) => get(path),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let lastState: { ledgers: LedgerSummary[]; loading: boolean; refresh: () => Promise<void> } | null;

function Probe({ client, company }: { client: OpenCompanyClient; company: string | null }) {
  const nav = useLedgerNav(client, company);
  lastState = nav;
  return null;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  lastState = null;
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

describe("useLedgerNav", () => {
  it("fetches once for a company and exposes what came back", async () => {
    const get = vi.fn(async () => ({ ledgers: [GOALS], remaining: 11 }) satisfies LedgerList);
    await act(async () => {
      root.render(createElement(Probe, { client: fakeClient(get), company: "acme" }));
    });
    expect(get).toHaveBeenCalledTimes(1);
    expect(lastState?.ledgers).toEqual([GOALS]);
    expect(lastState?.loading).toBe(false);
  });

  it("does not fetch, and reports not-loading, with no company", async () => {
    const get = vi.fn(async () => ({ ledgers: [GOALS], remaining: 11 }) satisfies LedgerList);
    await act(async () => {
      root.render(createElement(Probe, { client: fakeClient(get), company: null }));
    });
    expect(get).not.toHaveBeenCalled();
    expect(lastState?.ledgers).toEqual([]);
    expect(lastState?.loading).toBe(false);
  });

  it("refresh() re-fetches on demand", async () => {
    let calls = 0;
    const get = vi.fn(async () => {
      calls += 1;
      return { ledgers: calls === 1 ? [] : [GOALS], remaining: 11 } satisfies LedgerList;
    });
    await act(async () => {
      root.render(createElement(Probe, { client: fakeClient(get), company: "acme" }));
    });
    expect(lastState?.ledgers).toEqual([]);

    await act(async () => {
      await lastState?.refresh();
    });
    expect(get).toHaveBeenCalledTimes(2);
    expect(lastState?.ledgers).toEqual([GOALS]);
  });

  it("re-fetches when the company changes", async () => {
    const get = vi.fn(async () => ({ ledgers: [GOALS], remaining: 11 }) satisfies LedgerList);
    const client = fakeClient(get);
    await act(async () => {
      root.render(createElement(Probe, { client, company: "acme" }));
    });
    expect(get).toHaveBeenCalledTimes(1);

    await act(async () => {
      root.render(createElement(Probe, { client, company: "other" }));
    });
    expect(get).toHaveBeenCalledTimes(2);
  });

  it("degrades to empty on a failed read rather than throwing", async () => {
    const get = vi.fn(async () => {
      throw new Error("boom");
    });
    await act(async () => {
      root.render(createElement(Probe, { client: fakeClient(get), company: "acme" }));
    });
    expect(lastState?.ledgers).toEqual([]);
    expect(lastState?.loading).toBe(false);
  });
});
