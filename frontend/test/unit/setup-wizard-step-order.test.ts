// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { SetupStatus } from "@/api/setup";
import { SetupWizard } from "@/views/setup/SetupWizard";

/**
 * The sign-in question comes before its consequences.
 *
 * The wizard used to ask "what's your email?" one screen before it offered the
 * choice that decides whether an address is needed at all — and it offered that
 * choice buried inside Advanced, under copy inviting the operator to press
 * straight past it. So an operator on a laptop was asked for an address they
 * would never have had to supply had they seen that "no sign-in" was on the
 * table.
 *
 * What this file pins is therefore about *order and visibility*, not about any
 * one screen: the mode is asked first, and a host told it needs no sign-in is
 * never shown the address step at all — not disabled, not optional, absent,
 * with no slot of its own in the progress bar.
 */

function status(over: Partial<SetupStatus> = {}): SetupStatus {
  return {
    complete: false,
    config_path: "/data/config.toml",
    fields: [],
    templates: [],
    auth_modes: ["email", "none"],
    build: {
      acp_in_build: false,
      acp_transport_mounted: false,
      mcp_in_build: false,
      harness_in_build: false,
      oauth_in_build: false,
    },
    companies: [],
    inference: { ready: false, provider: null, base_url: null },
    mail: { wired: false, echoes_code: true },
    ...over,
  };
}

function clientWith(s: SetupStatus): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company",
    get: async () => s,
    post: async () => ({
      complete: true,
      config_path: s.config_path,
      restart_required: [],
      seeded_company: null,
    }),
  } as unknown as OpenCompanyClient;
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
    root.render(createElement(SetupWizard, { client, onDone: () => {} }));
  });
}

const find = (testId: string) => container.querySelector(`[data-testid="${testId}"]`);

async function click(testId: string) {
  const el = find(testId) as HTMLElement | null;
  expect(el, `no element ${testId}`).toBeTruthy();
  await act(async () => {
    el!.click();
  });
}

function labelled(...wanted: string[]): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll("button")).find((b) =>
    wanted.includes(b.textContent?.trim() ?? ""),
  );
  expect(match, `no button labeled ${wanted.join("/")}`).toBeTruthy();
  return match as HTMLButtonElement;
}

/** "Looks good" is the same control under a different word, on Advanced. */
/** The wizard's own progress line, e.g. `Review · step 4 of 4`. */
function stepLabel(): string {
  const match = container.textContent?.match(/(\w[\w -]*) · step \d+ of \d+/);
  return match ? match[0] : "";
}

const next = async () =>
  act(async () => {
    labelled("Next", "Looks good").click();
  });

const back = async () =>
  act(async () => {
    labelled("Back").click();
  });

async function fill(testId: string, value: string) {
  const field = find(testId) as HTMLInputElement | HTMLTextAreaElement | null;
  expect(field, `no field ${testId}`).toBeTruthy();
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      field instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(field!, value);
    field!.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** model (skipped) -> business (answered) -> sign-in. */
async function goToSignIn() {
  await click("setup-skip-model");
  await next(); // -> business
  await fill("setup-field-industry", "E-commerce — homeware");
  await next(); // -> sign-in
}

/** The slots the progress bar is actually drawing. */
const slots = () =>
  Array.from(container.querySelectorAll("[data-testid^='step-']")).map((el) =>
    el.getAttribute("data-testid"),
  );

describe("where the sign-in question sits", () => {
  it("asks how people sign in before it asks for an address", async () => {
    await show(clientWith(status()));
    await goToSignIn();

    expect(find("auth-mode-email")).toBeTruthy();
    // The consequence has not been asked for yet.
    expect(find("setup-field-email")).toBeNull();

    await next();
    expect(find("setup-field-email")).toBeTruthy();
  });

  it("gives the sign-in question its own heading, not Advanced's", async () => {
    // It was written to sit inside the Advanced accordion, where the group
    // header supplied the question. Standing on its own it has to ask it.
    await show(clientWith(status()));
    await goToSignIn();

    expect(find("setup-question")?.textContent).toContain("sign in");
    expect(find("setup-advanced")).toBeNull();
  });

  it("has no Advanced step left to offer the mode a second time in", async () => {
    // Advanced held one group — how this host runs — and hiding it leaves the
    // step with nothing on it, so the step goes too rather than rendering blank.
    await show(clientWith(status()));
    await goToSignIn();
    await next(); // -> account
    await fill("setup-field-email", "ada@example.com");
    await next(); // -> review

    // Landed on Review, asserted before anything about what is absent. Without
    // this the three checks below would also pass if the press had gone nowhere
    // — a test that cannot fail for the reason it exists.
    //
    // Read off the step label rather than `setup-review`: entering Review kicks
    // off the roster design, so the screen is its spinner first and the review
    // body only once that settles. The label is true in both.
    expect(stepLabel(), "the press should have reached Review").toMatch(/^Review · step/);
    expect(find("setup-advanced")).toBeNull();
    expect(find("auth-mode-email")).toBeNull();
    expect(find("field-auth_mode")).toBeNull();
  });
});

describe("a step this host does not need gets no slot", () => {
  it("draws five steps on a host that asks people to sign in", async () => {
    await show(clientWith(status()));
    await goToSignIn();

    expect(slots()).toEqual([
      "step-power",
      "step-business",
      "step-signin",
      "step-account",
      "step-review",
    ]);
    expect(container.textContent).toContain("step 3 of 5");
  });

  it("draws four, and never the address field, once no sign-in is chosen", async () => {
    await show(clientWith(status()));
    await goToSignIn();
    await click("auth-mode-none");

    expect(slots()).toEqual(["step-power", "step-business", "step-signin", "step-review"]);
    // The bar must renumber too: a four-step flow that says "of 5" is telling
    // the operator about a screen they will never be shown.
    expect(container.textContent).toContain("step 3 of 4");

    await next();
    // Review, not "wherever the press left us": absence proves nothing about a
    // screen that never changed.
    expect(stepLabel(), "the press should have reached Review").toMatch(/^Review · step/);
    expect(find("setup-advanced")).toBeNull();
    expect(find("setup-field-email")).toBeNull();
  });
});

describe("the position survives the list changing under it", () => {
  /**
   * The reason the wizard holds *which step* rather than *which index*.
   *
   * Choosing a mode adds or removes a screen behind the operator's back, and an
   * index means a different screen the moment the list changes length. Here the
   * flow goes forward under `none`, comes back, and switches to `email` — after
   * which the very next press must land on the address it now needs, not skip
   * over it into Advanced.
   */
  it("lands on the address step after switching back to email sign-in", async () => {
    await show(clientWith(status()));
    await goToSignIn();
    await click("auth-mode-none");
    await next(); // -> review, with no address step in the list
    await back(); // -> sign-in
    expect(find("auth-mode-email")).toBeTruthy();

    await click("auth-mode-email");
    await next();

    expect(find("setup-field-email")).toBeTruthy();
    expect(find("setup-advanced")).toBeNull();
  });
});
