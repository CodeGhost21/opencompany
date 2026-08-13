// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ComposioConnection } from "@/api/composio";
import { AccountChoiceSection } from "@/views/connections/AccountChoiceSection";

/**
 * Which account a company acts as, while the operator moves between companies
 * (issue #820).
 *
 * A pure test cannot reach this. The bug is one of *ordering* between a
 * mutation, the read that follows it, and a prop change that lands in between —
 * three things that only exist once the component is mounted and rendering. It
 * is the same exception `provider-detail-render` earns: what is under test is
 * what the operator ends up looking at.
 *
 * The failure it pins: a choose/clear that resolves after the view has moved to
 * another company re-reads through the closure it was created with, and paints
 * the previous company's accounts onto the current company's page. Every id on
 * screen then belongs to a company the operator is not looking at — and the
 * next click sends one of them.
 */

/** Two accounts under one toolkit — the only shape this section renders at all. */
function twoAccounts(company: string): ComposioConnection[] {
  return [
    {
      toolkit: "gmail",
      connected: true,
      accounts: [
        {
          id: `${company}-ops`,
          status: "ACTIVE",
          connected: true,
          account: `ops@${company}.test`,
        },
        {
          id: `${company}-billing`,
          status: "ACTIVE",
          connected: true,
          account: `billing@${company}.test`,
        },
      ],
    },
  ];
}

/** A promise this test resolves by hand, so a request can be left in flight. */
function deferred<T>() {
  let settle!: (value: T) => void;
  const promise = new Promise<T>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

interface Host {
  client: OpenCompanyClient;
  /** Every company the connection list was read for, oldest first. */
  reads: string[];
  /** Release the pending `PUT …/default`. */
  finishChoose: () => void;
}

/**
 * A host that serves each company its own accounts and holds the choice write
 * open until the test lets it go.
 *
 * The scope prefix is the real one (`/api/v1/companies/{id}` vs the unscoped
 * path), so the company a call was aimed at is recoverable from the URL — which
 * is the whole assertion.
 */
function host(): Host {
  const reads: string[] = [];
  const pending = deferred<{ toolkit: string; connectionId: string; note: string }>();
  const companyOf = (path: string) => path.match(/companies\/([^/]+)\//)?.[1] ?? "";
  const client = {
    scopeFor: (company: string | null) =>
      company === null ? "/api/v1/company" : `/api/v1/companies/${company}`,
    get: async (path: string) => {
      const company = companyOf(path);
      reads.push(company);
      return twoAccounts(company);
    },
    put: async (path: string) => {
      const company = companyOf(path);
      // Only the write under test is deferred; a second company's writes would
      // deadlock the test rather than fail it.
      if (company !== "alpha") return { toolkit: "gmail", connectionId: "", note: "done" };
      return pending.promise;
    },
  } as unknown as OpenCompanyClient;
  return {
    client,
    reads,
    finishChoose: () =>
      pending.settle({ toolkit: "gmail", connectionId: "alpha-billing", note: "Acting as billing" }),
  };
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient, company: string) {
  await act(async () => {
    root.render(
      createElement(AccountChoiceSection, { client, company, canManage: true }),
    );
  });
}

function text(): string {
  return container.textContent ?? "";
}

/** The "Act as this" button on a named account row. */
function actAs(id: string): HTMLButtonElement {
  const row = container.querySelector(`[data-testid="account-${id}"]`);
  expect(row, `no row for ${id}`).not.toBeNull();
  const button = row!.querySelector("button");
  expect(button, `no button on ${id}`).not.toBeNull();
  return button as HTMLButtonElement;
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

describe("choosing an account while the view changes company", () => {
  it("does not answer the new company's page with the previous one's accounts", async () => {
    const { client, reads, finishChoose } = host();
    await show(client, "alpha");
    expect(text()).toContain("ops@alpha.test");

    // Send the choice, and leave it in flight.
    await act(async () => {
      actAs("alpha-billing").click();
    });

    // The operator moves to another company before it lands.
    await show(client, "beta");
    expect(text()).toContain("ops@beta.test");

    // Now the write completes. The refresh it would have run is bound to
    // `alpha` — the closure it was created in — so running it here is exactly
    // the bug.
    await act(async () => {
      finishChoose();
    });

    expect(text()).toContain("ops@beta.test");
    expect(text()).not.toContain("alpha");
    expect(
      reads.filter((company) => company === "alpha"),
      "the settled mutation must not re-read the company that is no longer shown",
    ).toHaveLength(1);
  });

  it("still re-reads when the operator stayed put", async () => {
    // The guard has to be a *company* check and not a blanket "never refresh
    // after a mutation" — the mark the console draws comes from the host's own
    // answer, so the ordinary path must still go back for it.
    const { client, reads, finishChoose } = host();
    await show(client, "alpha");

    await act(async () => {
      actAs("alpha-billing").click();
    });
    await act(async () => {
      finishChoose();
    });

    expect(
      reads.filter((company) => company === "alpha"),
      "the initial read plus the one the choice triggered",
    ).toHaveLength(2);
  });
});
