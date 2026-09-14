// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ActivationStatus } from "@/api/activation";
import type { OpenCompanyClient } from "@/api/client";
import { useActivationGate } from "@/onboarding/useActivationGate";

const STATUS_A: ActivationStatus = {
  nameConfirmed: true,
  integrationConnected: true,
  workflowRunSucceeded: true,
  isActivated: true,
};

const STATUS_B: ActivationStatus = {
  nameConfirmed: false,
  integrationConnected: false,
  workflowRunSucceeded: false,
  isActivated: false,
};

/** A promise plus the callback that settles it, so a test can control when a
 * read "lands" relative to a company switch. */
function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/**
 * A minimal `OpenCompanyClient` double: company A's `/activation` read is
 * whatever promise the test hands in (so it can be left pending across a
 * switch and resolved afterwards), company B's resolves immediately with
 * `STATUS_B`, and everything else hangs — the same shape
 * `onboarding-gate-waiver-scope-switch.test.ts` and
 * `onboarding-gate-stuck-escape.test.ts` use.
 */
function buildClient(companyAActivation: Promise<ActivationStatus>): OpenCompanyClient {
  const known = {
    baseUrl: "",
    scopeFor: (company: string | null | undefined) => `/api/v1/companies/${company ?? ""}`,
    subscribeToEvents: () => () => {},
    get: (path: string) => {
      if (path === "/api/v1/companies/acme-a/activation") return companyAActivation;
      if (path === "/api/v1/companies/acme-b/activation") return Promise.resolve(STATUS_B);
      return new Promise<never>(() => {});
    },
  };
  return new Proxy(known, {
    get(target, prop, receiver) {
      if (prop in target) return Reflect.get(target, prop, receiver);
      return () => new Promise<never>(() => {});
    },
  }) as unknown as OpenCompanyClient;
}

function Probe({
  client,
  company,
}: {
  client: OpenCompanyClient;
  company: string | null;
}): ReturnType<typeof createElement> {
  const gate = useActivationGate(client, company, true);
  return createElement(
    "div",
    { "data-testid": "probe" },
    JSON.stringify({ checked: gate.checked, status: gate.status }),
  );
}

/** Flushes whatever microtask chain a resolved/rejected promise needs to
 * reach a `setState` call and the resulting re-render. */
async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

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

describe("useActivationGate company switch", () => {
  /**
   * `load`'s `gen = ++generation.current` guard (`useActivationGate.ts:178`)
   * exists so a response for the company the gate has already switched away
   * from cannot land after the new company's own read and overwrite it. This
   * pins the scenario the guard's own doc names but no test exercised: a real
   * `company` prop change while the outgoing company's read is still in
   * flight, not two overlapping ticks for the *same* company
   * (`onboarding-gate-stuck-escape.test.ts` covers that one) and not the
   * `AppShell` waiver-cleanup race
   * (`onboarding-gate-waiver-scope-switch.test.ts` covers that one).
   */
  it("discards company A's late read after switching to company B", async () => {
    const companyA = deferred<ActivationStatus>();
    const client = buildClient(companyA.promise);

    await act(async () => {
      root.render(createElement(Probe, { client, company: "acme-a" }));
      await flush();
    });

    // Switch before A's read has landed.
    await act(async () => {
      root.render(createElement(Probe, { client, company: "acme-b" }));
      await flush();
    });

    const afterSwitch = JSON.parse(container.textContent ?? "{}") as {
      checked: boolean;
      status: ActivationStatus | null;
    };
    expect(afterSwitch.checked).toBe(true);
    expect(afterSwitch.status).toEqual(STATUS_B);

    // A's read finally lands, after B's has already settled the gate.
    await act(async () => {
      companyA.resolve(STATUS_A);
      await flush();
    });

    const afterStaleLand = JSON.parse(container.textContent ?? "{}") as {
      checked: boolean;
      status: ActivationStatus | null;
    };
    expect(
      afterStaleLand.status,
      "company A's late-arriving read must not overwrite company B's already-settled status",
    ).toEqual(STATUS_B);
  });
});
