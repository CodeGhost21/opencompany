// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App, magicLinkNotice } from "@/App";
import { ApiError } from "@/api/types";
import { resetConnections } from "@/connections/registry";

/**
 * A magic link that will not redeem has to *say so* (issue #1305).
 *
 * The failure it guards is the quietest one in the console. `App` sets
 * `failed: true` on a refused redemption, which forces the sign-in view — and
 * that view is byte-identical to the one a cold visit gets, because `notice` is
 * the only field that reaches the screen. So the routine outcome of clicking a
 * fifteen-minute link out of a mailbox the next morning was a blank form, an
 * empty email box, and nothing anywhere saying a link had been tried at all.
 *
 * Two things are pinned here, and they fail independently:
 *
 *   1. the *wording* — one line per distinguishable failure, none of which may
 *      name an address or admit that one has an account here;
 *   2. the *wiring* — that a refused `verifyCode` actually puts one on screen.
 *      A regression to `failed`-only would leave every mapping below passing.
 */

describe("magicLinkNotice", () => {
  /** The host's single answer for expired, spent, unknown and forged alike. */
  const refused = new ApiError(401, "invalid_login", "that didn't work", true);

  it("names both causes a refusal could have, because the host will not say", () => {
    const notice = magicLinkNotice(refused);
    expect(notice).toMatch(/expire/i);
    expect(notice).toMatch(/once/i);
    // And what to do about it.
    expect(notice).toMatch(/request a new one/i);
  });

  it("blames the network, not the link, when the host was never reached", () => {
    // A link that was never checked may well still be good; sending someone off
    // to request a second one would only fail the same way.
    const notice = magicLinkNotice(new ApiError(0, "network", "Can't reach the host"));
    expect(notice).toMatch(/reach/i);
    expect(notice).not.toMatch(/expire/i);
  });

  it("points at the replacement when the company stopped using links", () => {
    const notice = magicLinkNotice(new ApiError(409, "auth_mode", "wrong mode", true));
    expect(notice).not.toMatch(/request a new one/i);
    expect(notice).toMatch(/below/i);
  });

  it("still says something for a failure it does not recognise", () => {
    expect(magicLinkNotice(new Error("boom"))).not.toBe("");
    expect(magicLinkNotice(new Error("boom"))).toMatch(/didn't work/i);
  });

  it("never names a person, an address, or an account", () => {
    const every = [
      refused,
      new ApiError(0, "network", "nope"),
      new ApiError(409, "auth_mode", "nope", true),
      new Error("boom"),
    ].map(magicLinkNotice);
    for (const notice of every) {
      // The rule the whole sign-in surface is built around: nothing here may be
      // used to test whether an address is a member of this company.
      expect(notice).not.toMatch(/account|@|address|member|user|registered|unknown/i);
    }
  });
});

/**
 * The wiring, end to end through `App`: a landing on `?company=&code=`, a host
 * that refuses the code, and a sign-in screen that has to explain itself.
 */
describe("a refused magic link on the landing", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
      true;
    localStorage.clear();
    resetConnections();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    window.history.replaceState({}, "", "/");
    vi.unstubAllGlobals();
  });

  /**
   * Answers the console the way a host answers a spent code: `401
   * invalid_login` on `auth/verify`, and an ordinary email-mode sign-in screen
   * for everything the login view asks for afterwards.
   */
  function stubHost(): void {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string | URL) => {
        const path = String(url);
        const body = (status: number, payload: unknown) =>
          new Response(JSON.stringify(payload), {
            status,
            headers: { "content-type": "application/json" },
          });
        if (path.includes("/auth/verify")) {
          return body(401, { error: "that didn't work", code: "invalid_login" });
        }
        if (path.includes("/auth/config")) return body(200, { mode: "email", passwords: true });
        if (path.includes("/auth/hub")) return body(200, { providers: [] });
        // Everything else — the probe's `/spec`, its company list — is beside
        // the point here and answers like a host that wants a session.
        return body(401, { error: "not signed in", code: "unauthorized" });
      }),
    );
  }

  async function landOn(search: string): Promise<void> {
    window.history.replaceState({}, "", search);
    stubHost();
    await act(async () => {
      root.render(createElement(App));
    });
    // The redemption and the login view's two config fetches each resolve a
    // microtask deep; flushing generously is cheaper than guessing the depth.
    for (let i = 0; i < 12; i += 1) {
      await act(async () => {
        await Promise.resolve();
      });
    }
  }

  it("explains itself instead of rendering an ordinary sign-in form", async () => {
    await landOn("/?company=acme&code=already-spent");

    const notice = container.querySelector('[data-testid="login-notice"]');
    expect(notice).not.toBeNull();
    expect(notice?.textContent ?? "").toMatch(/expire/i);
    expect(notice?.textContent ?? "").toMatch(/request a new one/i);
    // And the form is still there to act on, with the caret already in it.
    const email = container.querySelector<HTMLInputElement>("#email");
    expect(email).not.toBeNull();
    expect(document.activeElement).toBe(email);
  });

  it("still strips the single-use code out of the address bar", async () => {
    await landOn("/?company=acme&code=already-spent");

    expect(window.location.search).not.toContain("code=");
  });

  it("says nothing at all when there was no link to refuse", async () => {
    await landOn("/");

    expect(container.querySelector('[data-testid="login-notice"]')).toBeNull();
  });
});
