// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import type { ConnectionId, LocalScope } from "@/connections/types";
import { SetupController } from "@/setup/SetupController";
import { markSetupSkipped, setupRedesign, setupResuming } from "@/setup/state";

/**
 * The way back into setup after leaving it to wire a model.
 *
 * Not recording a skip for that navigation is only half the fix. This
 * controller stays mounted across hash changes, its gate re-evaluates only on
 * `(client, company, scope, deepLinked)`, and `evaluatedOnce` bars a second
 * unprompted open — so with nothing else done, an operator who followed "Set up
 * a model", configured one, and came back would find no dialog and an unstaffed
 * company, reachable only through the Team page's separate prompt. That is the
 * same dead end the skip caused, arrived at differently.
 *
 * So the departure records a debt and the return pays it, on both the routes a
 * return can take: an ordinary hash change, and a full reload (wiring a provider
 * can ask for a restart).
 */

/** One connection's view of a single-company host. */
const SCOPE: LocalScope = { connection: "test-connection" as ConnectionId, company: null };

/** The baseline every company carries — present, and not "staffed". */
const BASELINE: TeamMemberDto[] = ["operations", "page_builder", "researcher", "writer"].map(
  (id) => ({ id, role: "Analyst", inboxEnabled: false, global: true }) as TeamMemberDto,
);

const STAFFED: TeamMemberDto[] = [
  ...BASELINE,
  { id: "ada", role: "Operations", inboxEnabled: false } as TeamMemberDto,
];

function clientWith(roster: TeamMemberDto[]): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company}`,
    listTeam: async () => roster,
    // The dialog's own readiness check; `echo` keeps the notice on screen.
    get: async () => ({ cognition: "echo" }),
    post: async () => ({ agents: [], template: "ecommerce", source: "fallback" }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  window.location.hash = "#/overview";
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

async function mount(client: OpenCompanyClient, deepLinked = false) {
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: SCOPE,
        children: createElement(SetupController, { client, company: null, deepLinked }),
      }),
    );
  });
}

const dialog = () => document.querySelector('[data-testid="setup-dialog"]');
const find = (testId: string) => document.querySelector(`[data-testid="${testId}"]`);

const modelLink = () =>
  Array.from(document.querySelectorAll("a")).find((a) => a.textContent?.trim() === "Set up a model");

const addModelLink = () =>
  Array.from(document.querySelectorAll("a")).find(
    (a) => a.textContent?.trim() === "Add a model in Settings",
  );

/** Navigate as the console does — a hash change the controller can hear. */
async function goTo(hash: string) {
  await act(async () => {
    window.location.hash = hash;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

/** Answer the three questions and let the build-out finish. */
async function runFlow() {
  const setField = async (testId: string, value: string) => {
    const field = document.querySelector(`[data-testid="${testId}"]`) as
      | HTMLInputElement
      | HTMLTextAreaElement
      | null;
    expect(field, `no field ${testId}`).toBeTruthy();
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        field instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(field, value);
      field!.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      (document.querySelector('[data-testid="setup-next"]') as HTMLElement).click();
    });
  };
  await setField("setup-field-industry", "E-commerce — homeware");
  await setField("setup-field-teamHint", "");
  await setField("setup-field-automate", "");
  for (let i = 0; i < 40 && !addModelLink(); i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 60));
    });
  }
  expect(addModelLink(), "completion CTA never appeared").toBeTruthy();
}

/** Follow "Set up a model", which both closes the dialog and navigates. */
async function leaveForModelSettings() {
  const link = modelLink();
  expect(link, "no model link on the notice").toBeTruthy();
  await act(async () => {
    (link as HTMLElement).click();
  });
  // jsdom does not follow the anchor's href, so drive the navigation it implies.
  await goTo("#/settings/connections");
}

describe("leaving to wire a model", () => {
  it("records a debt rather than a skip", async () => {
    await mount(clientWith(BASELINE));
    expect(dialog(), "setup should have opened by itself").toBeTruthy();

    await leaveForModelSettings();

    expect(dialog(), "should have closed for the navigation").toBeNull();
    expect(setupResuming(SCOPE)).toBe(true);
  });

  it("reopens setup when the operator navigates back", async () => {
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    expect(dialog()).toBeNull();

    await goTo("#/overview");

    expect(dialog(), "the flow they went to enable is unreachable").toBeTruthy();
    expect(find("setup-question")).toBeTruthy();
  });

  it("stays shut while they are still on the settings page", async () => {
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    // A sub-page of the same settings area is not "coming back".
    await goTo("#/settings/connections?provider=openrouter");
    expect(dialog()).toBeNull();
  });

  it("reopens after a reload on the way back, not one on the page itself", async () => {
    // Wiring a provider can ask for a restart, so the return is not always a
    // hash change. Two fresh mounts stand in for the two reloads.
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    await act(async () => root.unmount());

    root = createRoot(container);
    // `deepLinked` as `AppShell` computes it for this address: a reload on
    // `#/settings/connections` is a named view, so nothing opens unprompted and
    // the resume is the only thing that could.
    await mount(clientWith(BASELINE), true);
    expect(dialog(), "reloaded on the settings page — they have not returned yet").toBeNull();

    window.location.hash = "#/overview";
    await act(async () => root.unmount());
    root = createRoot(container);
    await mount(clientWith(BASELINE), true);

    expect(dialog(), "a reload back on the console should resume setup").toBeTruthy();
  });

  it("does not reopen over a company someone else staffed meanwhile", async () => {
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    await act(async () => root.unmount());

    root = createRoot(container);
    window.location.hash = "#/overview";
    await mount(clientWith(STAFFED), true);

    expect(dialog(), "a setup dialog over a team that already exists").toBeNull();
    expect(setupResuming(SCOPE), "debt should be dropped").toBe(
      false,
    );
  });

  it("re-reads the roster on a hash-change return instead of trusting the old answer", async () => {
    // A colleague staffs the company while the operator is wiring a model. The
    // hash-change return goes through the same `arrive` listener as any other,
    // which must re-read the roster — the empty answer captured before the
    // navigation is a snapshot, not a fact, and opening setup over a team that
    // now exists would stack a second one.
    const roster: TeamMemberDto[] = [...BASELINE];
    const client = clientWith(roster);
    await mount(client);
    await leaveForModelSettings();

    roster.push({ id: "ada", role: "Operations", inboxEnabled: false } as TeamMemberDto);

    await goTo("#/overview");

    expect(dialog(), "a setup dialog over a team that exists").toBeNull();
    expect(setupResuming(SCOPE), "debt should be dropped").toBe(false);
  });

  it("keeps the debt when the return's roster read fails, so a later return can retry", async () => {
    // A transient failure must not consume the resume. The dialog stays shut
    // because the roster is unknown, but the debt survives so the next arrival
    // or reload can retry — otherwise the flow the operator went to enable is
    // reachable again only through the Company-page prompt.
    let failing = false;
    const roster: TeamMemberDto[] = [...BASELINE];
    const client = {
      ...clientWith(roster),
      listTeam: async () => {
        if (failing) throw new Error("transient");
        return roster;
      },
    } as unknown as OpenCompanyClient;
    await mount(client);
    await leaveForModelSettings();

    failing = true;
    await goTo("#/overview");
    expect(dialog(), "unknown roster — must stay shut").toBeNull();
    expect(setupResuming(SCOPE), "debt must survive the failed read").toBe(true);

    // The next return reads successfully: the debt pays out and setup reopens.
    failing = false;
    await goTo("#/settings/connections");
    await goTo("#/overview");
    expect(dialog(), "the retried return should resume setup").toBeTruthy();
  });

  it("ignores a stale return read that resolves after the company switches", async () => {
    // A return from model settings starts a roster read. If the operator
    // switches companies before it resolves, the callback must not reopen setup
    // over the new company: the listener is removed on the switch, but the
    // in-flight read is not cancelled, so without a guard the callback's
    // `setOpen` would land on a controller rendering a company its read never
    // saw — and the dialog it opened would then run replacement against the
    // wrong company's roster.
    let acmeReads = 0;
    let resolveAcme!: (roster: TeamMemberDto[]) => void;
    const acmeRead = new Promise<TeamMemberDto[]>((resolve) => {
      resolveAcme = resolve;
    });
    const client = {
      ...clientWith([...BASELINE]),
      listTeam: async (company: string | null) => {
        if (company === "acme" && ++acmeReads >= 2) return acmeRead;
        return [...STAFFED];
      },
    } as unknown as OpenCompanyClient;
    const render = (company: string | null) =>
      act(async () => {
        root.render(
          createElement(ConnectionScopeProvider, {
            scope: SCOPE,
            children: createElement(SetupController, { client, company, deepLinked: false }),
          }),
        );
      });

    // Mount on acme; the gate read is served and setup offers itself.
    await render("acme");
    expect(dialog(), "setup should have opened").toBeTruthy();
    await leaveForModelSettings();

    // Return to acme: `arrive` starts the roster read we hold open.
    await goTo("#/overview");
    expect(dialog(), "return read in flight").toBeNull();

    // Switch to a second company before the read lands. The stale callback
    // must not open setup over it — the second company's own gate read is the
    // only thing allowed to decide that.
    await render("globex");
    await act(async () => {
      resolveAcme([...BASELINE]);
    });

    expect(dialog(), "stale return read reopened setup over the new company").toBeNull();
  });

  it('drops the debt when the operator then says "I\'ll do this later"', async () => {
    await mount(clientWith(BASELINE));
    await leaveForModelSettings();
    await goTo("#/overview");
    expect(dialog()).toBeTruthy();

    await act(async () => {
      (find("setup-skip") as HTMLElement).click();
    });

    expect(dialog()).toBeNull();
    expect(setupResuming(SCOPE)).toBe(false);

    // And it stays shut across a return trip: "later" means later.
    await goTo("#/settings/connections");
    await goTo("#/overview");
    expect(dialog()).toBeNull();
  });
});

describe("the skip still suppresses the unprompted offer", () => {
  it("does not open on a company the operator already skipped", async () => {
    markSetupSkipped(SCOPE);
    await mount(clientWith(BASELINE));
    expect(dialog()).toBeNull();
  });
});

describe("leaving the completion screen to wire a model", () => {
  it("records a redesign debt, and the return reopens in redesign mode", async () => {
    const client = {
      scopeFor: () => "/api/v1/companies/acme",
      listTeam: async () => [...BASELINE],
      get: async () => ({ cognition: "echo" }),
      post: async () => ({
        agents: [{ name: "Ada", role: "Operations", description: "Runs the desk." }],
        template: "ecommerce",
        source: "fallback",
        reason: "no_model",
      }),
      addTeamMember: async () => ({}),
    } as unknown as OpenCompanyClient;
    await mount(client);
    await runFlow();

    await act(async () => {
      (addModelLink() as HTMLElement).click();
    });

    // The completion CTA is a *navigation*, not a finish: the shipped team is
    // to be redesigned on the return, and the run must not be treated as done.
    expect(dialog(), "should close for the navigation").toBeNull();
    expect(setupRedesign(SCOPE), "no redesign debt recorded").toBe(true);

    await goTo("#/overview");

    expect(dialog(), "the redesign they were owed never reopened").toBeTruthy();
    expect(find("setup-redesign-notice"), "not reopened in replacing mode").toBeTruthy();
  });
});
