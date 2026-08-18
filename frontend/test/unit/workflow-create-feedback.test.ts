// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import { WorkflowCreateDialog } from "@/views/WorkflowCreateDialog";

/**
 * What the create dialog says while it is working, and when it refuses (#1005).
 *
 * The three claims here are all about a moment that does not exist in any pure
 * helper: the window between the click and the host's answer. `validate()` can
 * be tested without a document because it maps a draft to a string; "the form
 * looks busy", "the draft survives a rejection" and "two clicks post once" are
 * properties of a mounted component holding an unresolved promise, so they earn
 * a jsdom render for the same reason `setup-wizard-finish-gate` does.
 *
 * They are also the three the operator actually notices. Before #1005 the
 * dialog's entire in-flight vocabulary was the word "Creating…" on one button,
 * and a host rejection landed in a banner that nothing scrolled to and no
 * screen reader announced — a create that failed was indistinguishable from a
 * create that was still going.
 */

/** The one call this suite cares about. `createWorkflow` is a thin wrapper over
 * `client.post`, so stubbing the client rather than the module keeps the real
 * request path — including the id/name trimming `submit()` does — in the test. */
function stubClient(post: (path: string, body?: unknown) => Promise<unknown>) {
  return {
    scopeFor: () => "/api/companies/acme",
    // Every GET the dialog fires on open is an optional picker source (roster,
    // sub-workflow list, wired channels, cognition path) and each has a
    // degrade-on-failure path, so one rejection stands in for "this host offers
    // none of them" and leaves the form fully authorable.
    get: () => Promise.reject(new Error("not offered by this host")),
    listTeam: () => Promise.reject(new Error("not offered by this host")),
    post,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let onOpenChange: ReturnType<typeof vi.fn>;

/** The dialog renders through a portal, so it is on `document`, not `container`. */
function inDialog<T extends Element>(selector: string): T | null {
  return document.querySelector<T>(`[data-slot="dialog-content"] ${selector}`);
}

function dialogContent(): HTMLElement {
  const el = document.querySelector<HTMLElement>('[data-slot="dialog-content"]');
  expect(el, "the dialog did not render").toBeTruthy();
  return el as HTMLElement;
}

function button(label: string): HTMLButtonElement {
  const match = Array.from(
    document.querySelectorAll<HTMLButtonElement>('[data-slot="dialog-content"] button'),
  ).find((b) => b.textContent?.trim().startsWith(label));
  expect(match, `no button labeled "${label}"`).toBeTruthy();
  return match as HTMLButtonElement;
}

function submitButton(): HTMLButtonElement {
  return inDialog<HTMLButtonElement>('[data-testid="workflow-dialog-submit"]')!;
}

/** Sets a controlled input the way a keystroke would: through the native value
 * setter, so React's own descriptor sees the change and `onChange` fires. */
function type(selector: string, value: string) {
  const input = inDialog<HTMLInputElement>(selector);
  expect(input, `no input matching ${selector}`).toBeTruthy();
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  setter.call(input, value);
  input!.dispatchEvent(new Event("input", { bubbles: true }));
}

async function open(client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(WorkflowCreateDialog, {
        open: true,
        onOpenChange,
        client,
        company: "acme",
        workflow: null,
      }),
    );
  });
}

/** The smallest draft `validate()` accepts: the starter trigger node already
 * carries an id and a name, so only the graph's own id and name are missing. */
async function fillValidDraft() {
  await act(async () => {
    type('[placeholder="e.g. campaign_pipeline"]', "campaign_pipeline");
  });
  await act(async () => {
    type('[placeholder="e.g. Campaign pipeline"]', "Campaign pipeline");
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  // jsdom does no layout and ships no `scrollIntoView`; `showError` calls it on
  // the next frame, where a throw would be an unhandled rejection rather than a
  // test failure. Stubbed, not asserted — WHERE the page scrolls is a browser
  // property and belongs to the e2e suite.
  Element.prototype.scrollIntoView = vi.fn();
  onOpenChange = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the create dialog while a create is in flight", () => {
  it("shows a spinner and stops the form being edited under the request", async () => {
    // Never settles: the assertions below are about the window between the
    // click and the answer, which is the whole point.
    const post = vi.fn(() => new Promise<never>(() => {}));
    await open(stubClient(post));
    await fillValidDraft();

    expect(dialogContent().getAttribute("aria-busy")).toBe("false");
    expect(submitButton().querySelector(".animate-spin")).toBeNull();

    await act(async () => {
      submitButton().click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(dialogContent().getAttribute("aria-busy")).toBe("true");
    expect(submitButton().querySelector(".animate-spin")).toBeTruthy();
    expect(submitButton().disabled).toBe(true);
    // The graph being posted is the one on screen, so every control that would
    // change it is off for the duration.
    expect(button("Add node").disabled).toBe(true);
    expect(button("Add edge").disabled).toBe(true);
    expect(inDialog<HTMLButtonElement>('[data-testid="workflow-copilot-draft"]')!.disabled).toBe(
      true,
    );
  });

  it("posts once when the operator clicks Create twice in one tick", async () => {
    const post = vi.fn(() => new Promise<never>(() => {}));
    await open(stubClient(post));
    await fillValidDraft();

    // BOTH clicks inside ONE `act`, deliberately. React commits state at the
    // end of the block, so the button is still enabled for the second one and
    // `submit()` is genuinely re-entered — which is the only way to reach the
    // guard. Split across two `act`s the first commit has landed, `disabled` is
    // set, React drops the event on the disabled fiber, and the assertion below
    // passes for a reason that has nothing to do with `submit()`.
    //
    // So this is the assertion that pins `submittingRef`: against a guard
    // reading the `submitting` STATE both calls see the pre-commit `false` and
    // this fails with two posts.
    await act(async () => {
      submitButton().click();
      submitButton().click();
    });

    expect(post).toHaveBeenCalledTimes(1);
  });
});

describe("the create dialog when the host refuses", () => {
  it("keeps the dialog open with the draft intact and announces the reason", async () => {
    const post = vi.fn(() =>
      Promise.reject(new ApiError(409, "conflict", "a workflow named “Campaign pipeline” already exists", true)),
    );
    await open(stubClient(post));
    await fillValidDraft();

    await act(async () => {
      submitButton().click();
    });

    const banner = inDialog<HTMLElement>('[data-testid="create-error"]');
    expect(banner).toBeTruthy();
    expect(banner!.textContent).toContain("already exists");
    // Announced, not merely rendered: the banner can be well below the fold on
    // a long graph, so being in the DOM is not the same as being reported.
    expect(banner!.getAttribute("role")).toBe("alert");
    expect(banner!.getAttribute("aria-live")).toBe("assertive");
    // ...and taken to, on the frame after it renders. This is the treatment the
    // client-validation branch already had and the server branch did not, which
    // is the whole of #1005's "a failed create is never announced": on a long
    // graph the banner renders below the fold and nothing moves to it.
    await act(async () => {
      await new Promise((r) => requestAnimationFrame(r));
    });
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();

    // The dialog never asked to close, and what the operator typed is still
    // there to fix — a rejected create must not cost them the graph.
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(inDialog<HTMLInputElement>('[placeholder="e.g. campaign_pipeline"]')!.value).toBe(
      "campaign_pipeline",
    );
    expect(inDialog<HTMLInputElement>('[placeholder="e.g. Campaign pipeline"]')!.value).toBe(
      "Campaign pipeline",
    );

    // ...and the form is live again, so the fix can be made and retried.
    expect(dialogContent().getAttribute("aria-busy")).toBe("false");
    expect(submitButton().disabled).toBe(false);
  });

  // The complaint was about the graph as it was, so any edit to that graph
  // retires it; leaving it up would make the next failure indistinguishable
  // from this one. Both entry points are covered because they are separate
  // code paths — the name is top-level state, a node row goes through
  // `updateNode`.
  for (const [what, selector] of [
    ["the name", '[placeholder="e.g. Campaign pipeline"]'],
    ["a node", '[placeholder="display name"]'],
  ] as const) {
    it(`drops the stale banner as soon as the operator edits ${what}`, async () => {
      await open(
        stubClient(() =>
          Promise.reject(new ApiError(400, "bad_request", "that id is taken", true)),
        ),
      );
      await fillValidDraft();

      await act(async () => {
        submitButton().click();
      });
      expect(inDialog('[data-testid="create-error"]')).toBeTruthy();

      await act(async () => {
        type(selector, "Something else");
      });

      expect(inDialog('[data-testid="create-error"]')).toBeNull();
    });
  }
});
