// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";

/**
 * Custom domain and Email (SMTP) are gated as coming soon (issue #2131).
 *
 * The brief is that they are *genuinely* inert: nothing reachable by mouse,
 * nothing in the tab order, nothing that can be submitted. So the assertions
 * here are about what is in the DOM rather than about how it looks — a blur and
 * an `opacity` are the affordance, not the gate, and a screenshot cannot tell
 * a disabled password field from an absent one.
 *
 * Why absent rather than `disabled`: an inert `<button>` still runs its handler
 * when something calls `.click()` on it, and a disabled `<input>` is still an
 * input whose value a script or a password manager can set. The SMTP card takes
 * a password and saves it into the host's secret store, so "you cannot Tab to
 * it" is not a strong enough promise for it.
 *
 * `domain-settings-host-backed.test.ts` covers the other half: the two cards
 * still behave, rendered directly, for the release that switches them on.
 */

vi.mock("sonner", () => {
  const fn = () => {};
  const toast = Object.assign(fn, {
    success: fn,
    error: fn,
    warning: fn,
    info: fn,
    message: fn,
  });
  return { toast };
});

const { DomainSettings } = await import("@/components/domain-settings");

/**
 * A client that records every call rather than answering one.
 *
 * A gated card must not read the host either — a request fired by a surface
 * nobody can use is a request nobody can explain, and on a company with a
 * domain configured it would paint real values behind the blur.
 */
function forbiddenClient(): { client: OpenCompanyClient; touched: string[] } {
  const touched: string[] = [];
  const record = (verb: string) => (path: string) => {
    touched.push(`${verb} ${path}`);
    return Promise.resolve({});
  };
  const client = {
    scopeFor: () => "/api/v1/companies/acme",
    get: record("GET"),
    put: record("PUT"),
    post: record("POST"),
    del: record("DELETE"),
  } as unknown as OpenCompanyClient;
  return { client, touched };
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

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(DomainSettings, { client, company: "acme" }));
  });
}

function card(testid: string): HTMLElement {
  const el = container.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
  expect(el, `${testid} should be on the page`).not.toBeNull();
  return el!;
}

describe("Custom domain and Email (SMTP) are gated as coming soon (#2131)", () => {
  it("still shows both cards, and says why each is not usable", async () => {
    // Gated, not deleted. An operator who came looking for "where do I put my
    // domain" has to find the answer where they expect it, and the answer has
    // to be a sentence rather than a control that does nothing.
    const { client } = forbiddenClient();
    await show(client);

    expect(card("domain-card").textContent).toContain("Custom domain");
    expect(card("domain-card").textContent).toContain("not switched on yet");
    expect(card("smtp-card").textContent).toContain("Email (SMTP)");
    expect(card("smtp-card").textContent).toContain("not switched on yet");

    // Said in the header too, where the eye lands before the paragraph.
    expect(card("domain-card-coming-soon").textContent).toBe("Coming soon");
    expect(card("smtp-card-coming-soon").textContent).toBe("Coming soon");
  });

  it("renders no control anyone or anything could operate", async () => {
    // The assertion the issue is actually about. Not "the controls are
    // disabled" — there are none. Nothing to Tab to, nothing to click, nothing
    // for `document.querySelector(…).click()` to reach, and in particular no
    // password field for an autofill to put a credential into.
    const { client } = forbiddenClient();
    await show(client);

    for (const testid of ["domain-card", "smtp-card"]) {
      const scope = card(testid);
      expect(scope.querySelectorAll("input"), `${testid} inputs`).toHaveLength(0);
      expect(scope.querySelectorAll("button"), `${testid} buttons`).toHaveLength(0);
      expect(scope.querySelectorAll("select"), `${testid} selects`).toHaveLength(0);
      expect(scope.querySelectorAll("textarea"), `${testid} textareas`).toHaveLength(0);
      expect(scope.querySelectorAll("a[href]"), `${testid} links`).toHaveLength(0);
      // Anything that made itself focusable another way — a `tabindex`, a
      // `contenteditable`, a role that implies interaction.
      expect(scope.querySelectorAll("[tabindex]"), `${testid} tabindex`).toHaveLength(0);
      expect(scope.querySelectorAll("[contenteditable]"), `${testid} editable`).toHaveLength(0);
    }
  });

  it("marks the preview inert and hides it from a screen reader", async () => {
    // Belt and braces over the assertion above: if someone later drops a real
    // control into a preview, `inert` keeps it out of the tab order and out of
    // the accessibility tree while the review catches up. Which is also why the
    // card's meaning lives in the header and description, not in the blur —
    // `aria-hidden` means nothing under it is announced at all.
    const { client } = forbiddenClient();
    await show(client);

    for (const testid of ["domain-card", "smtp-card"]) {
      const preview = card(`${testid}-preview`);
      expect(preview.getAttribute("aria-hidden"), `${testid} aria-hidden`).toBe("true");
      expect(preview.hasAttribute("inert"), `${testid} inert`).toBe(true);
      expect(preview.className, `${testid} pointer events`).toContain("pointer-events-none");
    }
  });

  it("asks the host for nothing at all", async () => {
    // A gated card that still reads `/domain` and `/smtp` would fire two
    // requests for a surface nobody can use — and on a company that already has
    // a domain it would paint real values behind the blur.
    const { client, touched } = forbiddenClient();
    await show(client);

    expect(touched).toEqual([]);
  });
});
