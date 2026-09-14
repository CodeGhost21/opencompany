// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { captureKeyLink } from "@/lib/pending-key-link";
import { ConnectTinyHumansButton } from "@/views/connections/ConnectTinyHumansButton";

/**
 * The one-click key grant, from the console's side.
 *
 * What matters here is what the browser is and is not trusted with: it carries
 * an opaque handle out and a single-use code back, and never a verifier or a
 * key. The rest is the host's, and is tested in `server::ops::company_key`.
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  captureKeyLink(null, false);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

function client(over: Partial<Record<string, unknown>> = {}) {
  return {
    scopeFor: (company: string | null) =>
      company ? `/api/v1/companies/${company}` : "/api/v1/company",
    post: async () => ({ authorizeUrl: "https://hub.example.com/auth/key?callback_url=x" }),
    ...over,
  } as unknown as OpenCompanyClient;
}

async function mount(props: Record<string, unknown>) {
  await act(async () => {
    root.render(
      createElement(ConnectTinyHumansButton, {
        client: client(),
        company: "acme",
        available: true,
        canManage: true,
        configured: false,
        ...props,
      } as never),
    );
  });
}

function button(): HTMLButtonElement | null {
  return container.querySelector('[data-testid="connect-tinyhumans"]');
}

describe("the connect button appears only where it could work", () => {
  it("renders nothing on a host that cannot grant a key", async () => {
    // `available` is the host's own `hubLink`. A button here could only 404.
    await mount({ available: false });
    expect(button()).toBeNull();
  });

  it("renders nothing for a member", async () => {
    // The same authority the paste field needs: this key is the company's
    // wallet, and minting it rather than pasting it does not change that.
    await mount({ canManage: false });
    expect(button()).toBeNull();
  });

  it("renders for an admin on a host with a hub", async () => {
    await mount({});
    expect(button()).not.toBeNull();
  });
});

describe("starting a grant leaves for the hub", () => {
  it("navigates top-level to the URL the host built", async () => {
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, assign });

    await mount({});
    await act(async () => {
      button()!.click();
    });

    // A navigation, not a fetch: the person signs in and approves on the hub's
    // own origin, which they must be able to see in the address bar.
    expect(assign).toHaveBeenCalledWith("https://hub.example.com/auth/key?callback_url=x");
  });
});

describe("coming back finishes the exchange", () => {
  it("redeems a captured grant exactly once, even under StrictMode's double effect", async () => {
    const post = vi.fn(async () => ({
      status: { configured: true, source: "company", notice: "" },
      note: "connected",
    }));
    captureKeyLink({ state: "s1", code: "c1" }, false);

    const onConnected = vi.fn();
    await act(async () => {
      root.render(
        createElement(ConnectTinyHumansButton, {
          client: client({ post }),
          company: "acme",
          available: true,
          canManage: true,
          configured: false,
          onConnected,
        } as never),
      );
    });

    // The code is single-use. A second redemption would spend nothing and
    // report the host's "expired" refusal over a connection that succeeded.
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/api/v1/companies/acme/credential/link/finish", {
      state: "s1",
      code: "c1",
    });
    expect(onConnected).toHaveBeenCalledTimes(1);
  });

  it("redeems nothing when the person cancelled on the hub", async () => {
    const post = vi.fn();
    captureKeyLink(null, true);

    await mount({ client: client({ post }) });

    expect(post).not.toHaveBeenCalled();
  });

  it("redeems nothing on an ordinary load", async () => {
    const post = vi.fn();
    await mount({ client: client({ post }) });
    expect(post).not.toHaveBeenCalled();
  });
});
