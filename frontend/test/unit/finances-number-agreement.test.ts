// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { FinancesDto } from "@/api/types";
import { FinancesView } from "@/views/FinancesView";

/**
 * The Finance overview folds one ledger: `spentUsd` is the sum of the same
 * negative entries the transaction list renders, so the tile and the rows are
 * two views of one number and may never disagree about its size.
 *
 * A spend of $0.16 read as "$0" above a list of this month's spending is the
 * failure being pinned. Both halves are checked from one render, because a
 * formatter agreeing with itself in isolation is what the two-formatter version
 * also did.
 */

/** This month's only spend: sixteen cents, in one transaction. */
const SUB_DOLLAR_SPEND: FinancesDto = {
  balanceUsd: -0.16,
  budgetUsd: null,
  spentUsd: 0.16,
  revenueUsd: 0,
  netUsd: -0.16,
  byCategory: [{ category: "Inference", amount: 0.16 }],
  transactions: [
    {
      id: "tx-0",
      date: "2026-09-02",
      description: "Inference spend",
      category: "Inference",
      amountUsd: 0.16,
      direction: "out",
    },
  ],
};

/** A spend too small for cent precision, which is still not nothing. */
const SUB_CENT_SPEND: FinancesDto = {
  ...SUB_DOLLAR_SPEND,
  balanceUsd: -0.004,
  spentUsd: 0.004,
  netUsd: -0.004,
  byCategory: [{ category: "Inference", amount: 0.004 }],
  transactions: [{ ...SUB_DOLLAR_SPEND.transactions[0]!, amountUsd: 0.004 }],
};

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
});

async function render(finances: FinancesDto): Promise<void> {
  const client = {
    finances: vi.fn(() => Promise.resolve(finances)),
  } as unknown as OpenCompanyClient;
  await act(async () => {
    root.render(createElement(FinancesView, { client, company: "acme" }));
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** The text of the tile whose label is `label`. */
function tile(label: string): string {
  const heading = [...container.querySelectorAll("span")].find(
    (node) => node.textContent === label,
  );
  expect(heading, `no tile labelled "${label}"`).toBeDefined();
  return heading!.closest("div.space-y-2")?.textContent ?? "";
}

describe("Finance overview number agreement", () => {
  it("states this month's sub-dollar spend as the same amount the rows total", async () => {
    await render(SUB_DOLLAR_SPEND);

    expect(tile("Spend")).toContain("$0.16");
    expect(tile("Wallet balance")).toContain("$0.16");
    // The defect: a whole-dollar tile over a cent-precision list.
    expect(tile("Spend")).not.toMatch(/\$0(?!\.)/);
    expect(container.textContent).toContain("−$0.16");
  });

  it("never renders a non-zero spend as nothing", async () => {
    await render(SUB_CENT_SPEND);

    expect(tile("Spend")).toContain("<$0.01");
    expect(tile("Spend")).not.toContain("$0.00");
    expect(tile("Spend")).not.toMatch(/\$0(?!\.)/);
  });

  it("rounds no amount up past the rows it summarises", async () => {
    // Half a dollar is the case a whole-dollar tile rounds to "$1" while the
    // one transaction under it says fifty cents.
    await render({
      ...SUB_DOLLAR_SPEND,
      balanceUsd: -0.5,
      spentUsd: 0.5,
      netUsd: -0.5,
      byCategory: [{ category: "Inference", amount: 0.5 }],
      transactions: [{ ...SUB_DOLLAR_SPEND.transactions[0]!, amountUsd: 0.5 }],
    });

    expect(tile("Spend")).toContain("$0.50");
    expect(tile("Spend")).not.toContain("$1");
    expect(container.textContent).toContain("−$0.50");
  });

  it("states the budget and what is left at the precision the spend is stated in", async () => {
    await render({ ...SUB_DOLLAR_SPEND, budgetUsd: 10 });

    const budget = container.textContent ?? "";
    expect(budget).toContain("$0.16 of $10.00 used");
    expect(budget).toContain("$9.84 left");
  });
});
