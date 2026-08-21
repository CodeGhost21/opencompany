// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { Login } from "@/views/Login";
import type { OpenCompanyClient } from "@/api/client";

/**
 * The screen a `none`-mode host puts in front of somebody who arrived from
 * outside it.
 *
 * This branch is not new. What is new is that it describes the **packaged
 * desktop app**, which now runs `none` host-wide — so it has stopped being a
 * misconfiguration nobody should ever see and become a real statement about a
 * real product, and it is worth pinning as one.
 *
 * On the desktop itself this view still never renders: every request from that
 * machine resolves the local owner before a session is looked for, so the
 * console is simply open. Reaching this screen means something addressed a
 * desktop company from somewhere that is *not* the desktop — a remote device
 * that was paired before the mode changed being the concrete case — and the
 * only honest answer is to say there is nothing to sign in with rather than to
 * draw a form whose every field is refused.
 *
 * The absence of the address field is the assertion that matters. The desktop
 * used to hand this view a synthetic `operator@opencompany.local` to prefill,
 * and the whole channel that carried it is gone; a form reappearing here would
 * be offering a credential that cannot exist.
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

/** A host that answers `/auth/config` the way a desktop install does. */
function noSignInClient(): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company",
    get: vi.fn().mockImplementation(async (path: string) => {
      if (path.endsWith("/auth/config")) {
        return { mode: "none", passwords: false, magicLink: false };
      }
      if (path.endsWith("/auth/hub")) return { providers: [] };
      throw new Error(`unexpected GET ${path}`);
    }),
    post: vi.fn(),
  } as unknown as OpenCompanyClient;
}

it("offers no way in, and no address to try it with", async () => {
  await act(async () => {
    root.render(
      createElement(Login, {
        client: noSignInClient(),
        company: "acme",
        onSignedIn: () => {},
      }),
    );
    await Promise.resolve();
  });

  expect(container.textContent).toContain("There is no sign-in here");
  expect(container.querySelector("#email")).toBeNull();
  expect(container.querySelector("#password")).toBeNull();
  expect(container.querySelector("[data-testid='suggested-email-hint']")).toBeNull();
});
