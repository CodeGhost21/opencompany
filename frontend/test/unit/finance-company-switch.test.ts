// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { CompanyFeed } from "@/hooks/use-company";
import { SettingsSection } from "@/views/SettingsSection";

/**
 * Billing settings must not carry one company's credentials into another's.
 *
 * This suite is normally for pure functions. The exception is earned the same
 * way `provider-detail-render` earns it: the behaviour under test is not a
 * function anybody can call, it is *composition* — `SettingsSection` giving
 * `BillingView` a `key` that changes with the company. A unit test of either
 * component alone would pass with the key removed, which is exactly the
 * regression that happened: clearing fields by hand inside `BillingView`
 * covered the ones somebody remembered and left the API key, webhook secret and
 * both PayPal halves behind, so an operator who typed a key, switched company,
 * and pressed Save wrote that credential into the wrong company's secret store.
 */

/** A client that answers the two billing reads and nothing else. */
function clientFor(company: string): OpenCompanyClient {
  return {
    scopeFor: () => `/api/v1/companies/${company}`,
    get: async (path: string) =>
      path.endsWith("/billing/paypal")
        ? {
            clientIdConfigured: false,
            clientSecretConfigured: false,
            environment: "sandbox",
            granted: true,
            inBuild: true,
          }
        : {
            apiKeyConfigured: false,
            site: null,
            webhookConfigured: false,
            webhookUrl: null,
            granted: true,
            inBuild: true,
          },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function showBilling(company: string) {
  await act(async () => {
    root.render(
      createElement(SettingsSection, {
        client: clientFor(company),
        company,
        feed: { messages: [] } as unknown as CompanyFeed,
        sub: "billing",
        onNavigate: () => {},
        onFlag: () => {},
      }),
    );
  });
}

function apiKeyBox(): HTMLInputElement {
  const box = container.querySelector<HTMLInputElement>('[data-testid="billing-api-key"]');
  if (!box) throw new Error("the API key input is not on the page");
  return box;
}

/** Types into the field the way an operator does, so React's state updates. */
async function type(box: HTMLInputElement, value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(box, value);
    box.dispatchEvent(new Event("input", { bubbles: true }));
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
});

describe("billing settings across a company switch", () => {
  it("drops a typed-but-unsaved credential when the company changes", async () => {
    await showBilling("acme");
    await type(apiKeyBox(), "cb_live_for_acme");
    expect(apiKeyBox().value).toBe("cb_live_for_acme");

    // The operator switches company without saving.
    await showBilling("globex");

    // The key must NOT still be sitting in the box, where the next Save would
    // send it to globex.
    expect(apiKeyBox().value).toBe("");
  });

  it("drops it again on a switch back, not just the first time", async () => {
    // A `key` that only changed once — or a clear that ran on mount only —
    // would pass the test above and fail this one.
    await showBilling("acme");
    await type(apiKeyBox(), "cb_live_for_acme");
    await showBilling("globex");
    await type(apiKeyBox(), "cb_live_for_globex");
    expect(apiKeyBox().value).toBe("cb_live_for_globex");

    await showBilling("acme");
    expect(apiKeyBox().value).toBe("");
  });
});
