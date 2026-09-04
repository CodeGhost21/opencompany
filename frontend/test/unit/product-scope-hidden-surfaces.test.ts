// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ComposioStatus } from "@/api/composio";
import type { InferenceStatus } from "@/api/inference";
import { INFERENCE_PROVIDERS, SETUP_INFERENCE_OPTIONS } from "@/api/setup";
import { HostSwitcher, hostSwitcherMenu } from "@/components/host-switcher";
import { ComposioSection } from "@/views/connections/ComposioSection";
import { InferenceSection } from "@/views/connections/InferenceSection";
import { HostsProvider, type HostsValue } from "@/connections/HostsContext";
import type { Connection, ConnectionId } from "@/connections/types";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * One company, and nothing else selectable.
 *
 * These pin the *absence* of controls, which is the only thing a hide can be
 * checked by. Each fails against a tree where the matching flag in
 * `product-scope.ts` is false — that is what makes them a test of the hide
 * rather than of the layout that happens to be on screen.
 */

const CONNECTION: Connection = {
  id: "c1" as ConnectionId,
  defaultCompany: null,
  label: "This computer",
  baseUrl: "",
  credential: { kind: "cookie" },
  status: "live",
  identity: null,
  companies: [],
  connector: { kind: "local" },
};

const SECOND: Connection = { ...CONNECTION, id: "c2" as ConnectionId, label: "Acme" };

function hosts(connections: Connection[]): HostsValue {
  return {
    connections,
    selected: connections[0]?.id ?? null,
    onSelect: () => {},
    onAdd: () => {},
    localInstances: [],
    onEditHost: () => {},
    onRemoveHost: () => {},
    hub: false,
  };
}

let container: HTMLDivElement;
let root: Root;

/** jsdom ships no `matchMedia`, and `SidebarProvider` reaches for it unguarded. */
function stubMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

beforeEach(() => {
  stubMatchMedia();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function show(value: HostsValue, props: Record<string, unknown> = {}) {
  await act(async () => {
    root.render(
      createElement(
        SidebarProvider,
        null,
        createElement(
          HostsProvider,
          { value, children: null } as never,
          createElement(HostSwitcher, {
            // What the app shell actually renders (`app-shell.tsx`): the window
            // title row, which is where an operator sees this.
            variant: "titlebar",
            companyName: "Acme",
            companies: [
              { id: "a", name: "Acme" },
              { id: "b", name: "Other" },
            ],
            activeCompany: "a",
            onSwitchCompany: () => {},
            onCreateCompany: () => {},
            canCreateCompany: true,
            ...props,
          } as never),
        ),
      ),
    );
  });
}

const find = (testId: string) =>
  document.querySelector(`[data-testid="${testId}"]`) as HTMLElement | null;

/**
 * Press whatever the switcher put on screen, then look.
 *
 * Menu content is portalled and only mounts once the trigger is pressed, so an
 * assertion made without this is vacuous — it passes against a tree that still
 * has the whole roster, because the roster simply had not been opened yet.
 */
async function openWhateverExists() {
  const trigger = container.querySelector("button");
  if (!trigger) return;
  await act(async () => {
    trigger.click();
    trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    trigger.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    trigger.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  });
}

describe("the company switcher is a label, not a menu", () => {
  it("opens nothing, even with two hosts and two companies to offer", async () => {
    await show(hosts([CONNECTION, SECOND]));

    // The trigger still names the company — that is the whole surface now.
    expect(container.textContent).toContain("Acme");
    // Nothing to click at all: not a disabled control, not a chevron over an
    // empty popup, no button element for a keyboard to land on.
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector("[aria-haspopup]")).toBeNull();
    expect(container.querySelector("[aria-expanded]")).toBeNull();
  });

  it("offers no host roster, no way to add one and no way to manage one", async () => {
    await show(hosts([CONNECTION, SECOND]));
    await openWhateverExists();

    expect(find("host-row-c1")).toBeNull();
    expect(find("host-row-c2")).toBeNull();
    expect(find("host-switcher-add")).toBeNull();
    expect(find("host-switcher-manage")).toBeNull();
  });

  it("offers no company switching and no way to make another company", async () => {
    await show(hosts([CONNECTION]));
    await openWhateverExists();

    expect(find("company-row-a")).toBeNull();
    expect(find("company-row-b")).toBeNull();
    expect(find("switcher-new-company")).toBeNull();
    expect(container.textContent).not.toContain("All companies");
  });

  it("keeps the trigger a nameplate rather than a chevron over an empty popup", async () => {
    // The trap this guards: `hostSwitcherMenu` still answers "any host at all
    // opens a menu", and with every group hidden that would be a chevron over a
    // popup with nothing in it. The switcher must not consult it alone.
    expect(hostSwitcherMenu(1)).toBe(true);

    await show(hosts([CONNECTION]));
    await openWhateverExists();

    // Nothing was openable, so nothing opened.
    expect(document.querySelector("[role='menu']")).toBeNull();
    expect(find("host-switcher-add")).toBeNull();
  });
});

/**
 * BYOK only, on both credential surfaces.
 *
 * The pair that matters: the managed route must not be *selectable*, and a
 * company already on it must still be *legible*. Hiding a route by deleting its
 * descriptor would satisfy the first and break the second — the label tables
 * keep every route for exactly that reason.
 */

function composioStatus(over: Partial<ComposioStatus> = {}): ComposioStatus {
  return {
    inBuild: true,
    granted: true,
    credentialSource: "company",
    mode: "managed",
    backendUrl: "https://api.tinyhumans.ai",
    toolkits: [],
    openMode: true,
    effectiveToolkits: [],
    effectiveCatalog: [],
    catalogSource: "manifest",
    catalogNotice: null,
    ...over,
  } as ComposioStatus;
}

function composioClient(status: ComposioStatus) {
  return {
    scopeFor: (company: string | null) =>
      company ? `/api/v1/companies/${company}` : "/api/v1/company",
    get: async () => status,
    put: async () => ({ status, note: "" }),
    post: async () => ({ status, note: "" }),
    del: async () => ({ status, note: "" }),
  } as unknown as OpenCompanyClient;
}

async function mountComposio(status: ComposioStatus) {
  await act(async () => {
    root.render(
      createElement(ComposioSection, {
        client: composioClient(status),
        company: "acme",
        canManage: true,
        onChanged: () => {},
      }),
    );
  });
}

describe("Composio offers this company's own account and nothing else", () => {
  it("does not offer the OpenHuman-managed route as a choice", async () => {
    await mountComposio(composioStatus({ mode: "byok" }));

    expect(find("composio-mode-managed")).toBeNull();
    expect(find("composio-mode-byok")).not.toBeNull();
  });

  it("still tells a company already on managed which route it is on", async () => {
    // No tile can be marked Current for a route that is not rendered, so the
    // card would otherwise read as "unconfigured" to a company that is working.
    await mountComposio(composioStatus({ mode: "managed" }));

    expect(find("composio-current-mode")).not.toBeNull();
    expect(find("composio-current-mode")!.textContent).toContain("through OpenHuman");
  });

  it("still lets a BYOK company clear its key, and does not name the hidden route", async () => {
    // Clearing was reached by picking the managed tile. With no tile to pick, a
    // company could rotate its key but never remove it — so the control has to
    // stand on its own, without advertising where clearing lands.
    await mountComposio(composioStatus({ mode: "byok" }));

    expect(find("composio-clear-key")).not.toBeNull();
    expect(container.textContent).not.toContain("use OpenHuman-managed");
  });
});

function inferenceClient(status: InferenceStatus) {
  return {
    scopeFor: (company: string | null) =>
      company ? `/api/v1/companies/${company}` : "/api/v1/company",
    get: async (path: string) => (path.endsWith("/inference/models") ? [] : status),
    put: async () => ({ status, note: "" }),
    del: async () => ({ status, note: "" }),
    post: async () => ({ status, note: "" }),
  } as unknown as OpenCompanyClient;
}

function inferenceStatus(over: Partial<InferenceStatus> = {}): InferenceStatus {
  return {
    provider: "openrouter",
    slug: "openrouter",
    baseUrl: "https://openrouter.ai/api/v1",
    models: {},
    defaultTierModels: {},
    source: "runtime",
    keyConfigured: true,
    cognition: "echo",
    usageMetering: "none",
    restartRequired: false,
    harnessReachable: true,
    canRebuildInPlace: true,
    ...over,
  };
}

async function mountInference(status: InferenceStatus) {
  await act(async () => {
    root.render(
      createElement(InferenceSection, {
        client: inferenceClient(status),
        company: "acme",
        canManage: true,
      }),
    );
  });
}

describe("inference asks the operator to name a provider", () => {
  it("does not offer the managed provider in the list", async () => {
    await mountInference(inferenceStatus());

    // The list is portalled and only mounts once the select is opened — asserted
    // without this the test passes against a tree that still offers managed.
    const trigger = document.querySelector("#inference-provider") as HTMLElement | null;
    expect(trigger, "no provider select").toBeTruthy();
    await act(async () => {
      trigger!.click();
      trigger!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      trigger!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      trigger!.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    });

    const options = Array.from(document.querySelectorAll("[role='option']")).map((o) =>
      o.textContent?.trim(),
    );
    expect(options.length, "the provider list did not open").toBeGreaterThan(0);
    expect(options).not.toContain("Managed (TinyHumans)");
    expect(options).toContain("OpenRouter");
  });

  it("still labels a company whose stored provider is the hidden one", async () => {
    // `LEGACY_MANAGED` resolves to the default provider on the host, so this
    // company keeps working; the console must still say what it is on rather
    // than printing the raw stored slug.
    await mountInference(inferenceStatus({ provider: "managed", slug: "managed" }));

    expect(container.textContent).toContain("Managed (TinyHumans)");
    expect(container.textContent).not.toMatch(/\bmanaged\b(?![\s)])/);
  });
});

describe("the wizard's model step asks the operator to name a provider too", () => {
  it("does not offer the managed endpoint as the thing to think with", () => {
    // The wizard has its own provider list, and it is the FIRST screen of a
    // first run — hiding the option only on the settings card would leave the
    // managed route selectable at the one moment every operator passes through.
    const offered = SETUP_INFERENCE_OPTIONS.map((option) => option.id);

    expect(offered).not.toContain("managed");
    expect(offered).toContain("openrouter");
    // Still resolvable, so a host already reporting it keeps its label.
    expect(INFERENCE_PROVIDERS.find((p) => p.id === "managed")?.label).toBe("TinyHumans");
  });
});
