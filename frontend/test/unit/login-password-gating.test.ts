// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Login } from "@/views/Login";
import type { OpenCompanyClient } from "@/api/client";

/**
 * `AuthConfig.passwords` says whether this company offers password sign-in
 * alongside the magic link. The toggle that switches into password mode must
 * honor it — a host that answers `{ mode: "email", passwords: false }` must
 * never expose a form for a route that will refuse it (issue caught by
 * review: the toggle originally checked only `mode === "email"`).
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function client(passwords: boolean): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company",
    get: vi.fn().mockImplementation(async (path: string) => {
      if (path.endsWith("/auth/config")) return { mode: "email", passwords };
      if (path.endsWith("/auth/hub")) return { providers: [] };
      throw new Error(`unexpected GET ${path}`);
    }),
    post: vi.fn(),
  } as unknown as OpenCompanyClient;
}

async function renderLogin(passwords: boolean) {
  await act(async () => {
    root.render(
      createElement(Login, {
        client: client(passwords),
        company: "acme",
        onSignedIn: () => {},
      }),
    );
    // Flush the microtasks the two config-fetching effects resolve on.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function passwordToggleText(): string | null {
  const button = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Use a password instead"),
  );
  return button?.textContent ?? null;
}

describe("Login password toggle", () => {
  it("is hidden when the host reports passwords: false", async () => {
    await renderLogin(false);
    expect(passwordToggleText()).toBeNull();
  });

  it("is shown when the host reports passwords: true", async () => {
    await renderLogin(true);
    expect(passwordToggleText()).not.toBeNull();
  });
});
