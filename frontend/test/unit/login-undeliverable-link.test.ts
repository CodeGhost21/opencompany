// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Login } from "@/views/Login";
import type { OpenCompanyClient } from "@/api/client";

/**
 * `AuthConfig.magicLink` false is a routable host with no mail transport: the
 * mode is still `email`, and hub OAuth and passwords still sign people in, but
 * a link asked for here reaches nobody. Nothing else in the flow reveals that —
 * `auth/request` answers `sent: true` exactly as a host that delivered would —
 * so this screen is the only place it can be said.
 *
 * The form is not removed. An operator can wire mail up without restarting this
 * console, and a person who knows a link is coming should still be able to ask
 * for one. What changes is which way in is presented as *the* way in.
 */

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

function client(config: {
  magicLink: boolean;
  passwords: boolean;
  providers?: { id: string; label: string; startUrl: string }[];
}): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company",
    get: vi.fn().mockImplementation(async (path: string) => {
      if (path.endsWith("/auth/config")) {
        return { mode: "email", passwords: config.passwords, magicLink: config.magicLink };
      }
      if (path.endsWith("/auth/hub")) return { providers: config.providers ?? [] };
      throw new Error(`unexpected GET ${path}`);
    }),
    post: vi.fn(),
  } as unknown as OpenCompanyClient;
}

async function renderLogin(c: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(Login, { client: c, company: "acme", onSignedIn: () => {} }),
    );
    // Flush the microtasks the two config-fetching effects resolve on.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

const find = (testId: string) => container.querySelector(`[data-testid="${testId}"]`);

const labels = () =>
  Array.from(container.querySelectorAll("button")).map((b) => b.textContent?.trim() ?? "");

const submitLabel = () =>
  (container.querySelector('button[type="submit"]')?.textContent ?? "").trim();

describe("a host that cannot deliver a magic link", () => {
  it("says so, rather than offering the link form as if it worked", async () => {
    await renderLogin(client({ magicLink: false, passwords: true }));

    expect(find("login-no-mail")).toBeTruthy();
  });

  it("leads with the password form when that is the way in", async () => {
    await renderLogin(client({ magicLink: false, passwords: true }));

    expect(container.querySelector("#password")).toBeTruthy();
    expect(submitLabel()).toContain("Sign in");
    // Not removed: asking for a link is still one press away, for the operator
    // who has just configured mail on the host behind this screen.
    expect(labels().some((l) => l.includes("Email me a link instead"))).toBe(true);
  });

  it("points at the ecosystem buttons when the host has them", async () => {
    await renderLogin(
      client({
        magicLink: false,
        passwords: true,
        providers: [{ id: "google", label: "Google", startUrl: "https://hub.test/google" }],
      }),
    );

    const links = Array.from(container.querySelectorAll("a")).map((a) => a.textContent ?? "");
    expect(links.some((l) => l.includes("Continue with Google"))).toBe(true);
    expect(find("login-no-mail")?.textContent).toMatch(/above|button/i);
  });

  it("still says it plainly when there is no password to fall back on", async () => {
    // Nothing to offer instead, which is exactly when saying nothing is worst:
    // the person would type an address and wait forever.
    await renderLogin(client({ magicLink: false, passwords: false }));

    expect(find("login-no-mail")).toBeTruthy();
    expect(container.querySelector("#email")).toBeTruthy();
  });

  it("leaves an ordinary host alone", async () => {
    await renderLogin(client({ magicLink: true, passwords: true }));

    expect(find("login-no-mail")).toBeNull();
    expect(submitLabel()).toContain("Email me a link");
  });
});
