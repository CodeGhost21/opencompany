// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { SetupStatus } from "@/api/setup";
import { SETUP_HANDOFF_FRAGMENT } from "@/setup/state";
import { SetupWizard } from "@/views/setup/SetupWizard";

/**
 * The console button's destination on completions that do not hand over a link.
 *
 * The sign-in hand-off navigates the whole document to a URL that already
 * carries the landing fragment (`#/company?from=setup`). The other three
 * outcomes — a host that asks nobody to sign in, a mailed link the operator
 * skips, and a link that could not be sent — all finish through the same
 * `setup-open-console` button, which remounts the shell in place (`onDone`
 * re-probes and boots a fresh `AppShell`). That remount must carry the same
 * fragment, or it lands on Overview with the tour free to open over the roster
 * setup just built — the exact miss the link path was fixed to avoid.
 */

function status(over: Partial<SetupStatus> = {}): SetupStatus {
  return {
    complete: false,
    config_path: "/data/config.toml",
    fields: [],
    templates: [],
    auth_modes: ["email", "wallet", "none"],
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

/**
 * Routed by path: the wizard makes different calls through `post` (the roster
 * design, the sign-in request, and the apply), so a blanket override would
 * silently change what the other two see.
 */
function clientWith(s: SetupStatus): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company}`,
    get: async () => s,
    post: async (path: string) => {
      if (path.endsWith("/setup/roster")) {
        return {
          agents: [{ name: "Ada", role: "Operations", description: "Runs the desk." }],
          template: "ecommerce",
          source: "fallback",
        };
      }
      if (path.endsWith("/auth/request")) return { sent: true };
      return {
        complete: true,
        config_path: s.config_path,
        restart_required: [],
        seeded_company: "acme",
      };
    },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let done: () => void;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  done = () => {};
  window.location.hash = "";
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
});

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(SetupWizard, { client, onDone: done }));
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

const next = async () =>
  act(async () => {
    const match = Array.from(container.querySelectorAll("button")).find((b) =>
      ["Next", "Looks good"].includes(b.textContent?.trim() ?? ""),
    );
    expect(match, "no advance button").toBeTruthy();
    match!.click();
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

/** Lets the design, apply and sign-in requests settle. */
const settle = async () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

/**
 * The desktop install: a host that asks nobody to sign in, so the wizard
 * preselects `none` and the address step is absent.
 */
async function finishNoSignIn() {
  (window as unknown as { __TAURI__: unknown }).__TAURI__ = { core: {} };
  await click("setup-skip-model");
  await next(); // -> business
  await fill("setup-field-industry", "E-commerce — homeware");
  await next(); // -> sign-in (none preselected)
  await next(); // -> advanced
  await next(); // -> review
  await settle();
  await click("setup-finish");
  await settle();
}

/** An email-sign-in host that cannot send mail: the "anyway" escape. */
async function finishUnmailable() {
  await click("setup-skip-model");
  await next(); // -> business
  await fill("setup-field-industry", "E-commerce — homeware");
  await next(); // -> sign-in
  await next(); // -> account
  await fill("setup-field-email", "ada@example.com");
  await next(); // -> advanced
  await next(); // -> review
  await settle();
  await click("setup-finish");
  await settle();
}

describe("the console button after setup applies without a hand-off link", () => {
  it("carries the roster destination out of a no-sign-in setup", async () => {
    let calls = 0;
    done = () => {
      calls += 1;
    };
    await show(clientWith(status()));
    await finishNoSignIn();

    // Nobody to invite, so there is no link to hand over — only the console.
    expect(find("setup-handoff-link")).toBeNull();
    expect(find("setup-open-console")).toBeTruthy();

    await click("setup-open-console");

    // The fresh `AppShell` this hands off to reads the fragment: it routes to
    // `#/company`, suppresses the tour welcome, and clears the marker.
    expect(window.location.hash).toBe(SETUP_HANDOFF_FRAGMENT);
    expect(calls).toBe(1);
  });

  it("carries the same destination out of the unmailable escape", async () => {
    await show(clientWith(status({ mail: { wired: false, echoes_code: false } })));
    await finishUnmailable();

    expect(find("setup-handoff-unmailable")).toBeTruthy();
    expect(find("setup-open-console")?.textContent).toContain("anyway");

    await click("setup-open-console");

    expect(window.location.hash).toBe(SETUP_HANDOFF_FRAGMENT);
  });
});
