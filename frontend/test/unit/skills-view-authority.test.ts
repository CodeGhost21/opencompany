// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { SkillsView } from "@/views/SkillsView";

/**
 * Install, uninstall, toggle and custom-authoring are all `AdminScopedCompany`
 * on the host — a skill's content lands in every agent's effective prompt,
 * company-wide, so these are decisions for the company rather than for the
 * caller alone (codex review). Before this, the console had no role check at
 * all: a member saw every write control enabled and learned only from a 403
 * toast that pasting one in was never going to work.
 */

const INSTALLED: Array<{
  id: string;
  name: string;
  description: string;
  category: string;
  source: string;
  enabled: boolean;
}> = [
  {
    id: "seo-audit",
    name: "SEO audit",
    description: "Checks a site's on-page SEO.",
    category: "Marketing",
    source: "registry",
    enabled: true,
  },
];

const REGISTRY: Array<{
  id: string;
  name: string;
  description: string;
  category: string;
  publisher: string;
}> = [
  {
    id: "cold-outreach",
    name: "Cold outreach",
    description: "Drafts a cold email sequence.",
    category: "Marketing",
    publisher: "OpenCompany",
  },
];

/**
 * A client answering the two reads with fixed data, and `/auth/me` as an
 * admin by default — matching `HostingView`'s own fixture convention.
 * `carriesPlatformBearer` defaults to `false`, matching a browser session
 * authenticating by cookie.
 */
function clientWith(role: "admin" | "member" = "admin", carriesPlatformBearer = false): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/companies/acme",
    carriesPlatformBearer,
    get: (path: string) => {
      if (path.endsWith("/auth/me")) {
        return Promise.resolve({ id: "u1", email: "a@b.c", role, company: "acme", hasPassword: true });
      }
      if (path.endsWith("/skills/registry")) {
        return Promise.resolve(REGISTRY);
      }
      return Promise.resolve(INSTALLED);
    },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(SkillsView, { client, company: "acme" }));
  });
  // `refresh` resolves both reads as one `Promise.allSettled`, and canManage
  // resolves through its own `/auth/me` round trip — each lands a tick after
  // the initial render, so give React those ticks rather than assuming one
  // flush covers every source.
  await act(async () => {});
  await act(async () => {});
}

function at(testid: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
}

/** Switches to the Registry tab, whose panel is not mounted until active. */
async function openRegistryTab() {
  const tab = Array.from(container.querySelectorAll<HTMLElement>('[role="tab"]')).find((t) =>
    t.textContent?.includes("Registry"),
  );
  await act(async () => {
    tab?.click();
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("SkillsView authority", () => {
  it("offers a member no way to add, install, enable, or uninstall a skill", async () => {
    await show(clientWith("member"));

    expect(at("skills-admin-only")?.textContent).toContain("Only an admin");
    // The page-level "Add skill" action is gone, not merely disabled.
    expect(
      Array.from(container.querySelectorAll("button")).some((b) => b.textContent?.includes("Add skill")),
    ).toBe(false);

    const toggle = at("installed-card")?.querySelector('[aria-label="Enable skill"]');
    expect(toggle).not.toBeNull();
    expect(toggle?.getAttribute("aria-disabled")).toBe("true");

    expect(at("installed-card")?.querySelector('[aria-label="Uninstall"]')).toBeNull();

    // Not a blank page: the installed skill's name and reach are still shown.
    // Checked before switching tabs — the Installed panel unmounts once the
    // Registry tab takes its place.
    expect(container.textContent).toContain("SEO audit");
    expect(container.textContent).toContain("Teammates can read this");

    await openRegistryTab();
    expect(
      Array.from(at("registry-card")?.querySelectorAll("button") ?? []).some((b) =>
        b.textContent?.includes("Install"),
      ),
    ).toBe(false);
  });

  it("offers an admin every control, with no read-only notice", async () => {
    await show(clientWith("admin"));

    expect(at("skills-admin-only")).toBeNull();
    expect(
      Array.from(container.querySelectorAll("button")).some((b) => b.textContent?.includes("Add skill")),
    ).toBe(true);

    const toggle = at("installed-card")?.querySelector('[aria-label="Enable skill"]');
    expect(toggle?.getAttribute("aria-disabled")).not.toBe("true");

    await openRegistryTab();
    expect(
      Array.from(at("registry-card")?.querySelectorAll("button") ?? []).some((b) =>
        b.textContent?.includes("Install"),
      ),
    ).toBe(true);
  });

  it("offers a platform bearer every control without calling /auth/me", async () => {
    const client = clientWith("member", true);
    const getSpy = vi.spyOn(client, "get");
    await show(client);

    expect(at("skills-admin-only")).toBeNull();
    expect(
      Array.from(container.querySelectorAll("button")).some((b) => b.textContent?.includes("Add skill")),
    ).toBe(true);
    expect(getSpy.mock.calls.some(([path]) => String(path).endsWith("/auth/me"))).toBe(false);
  });
});
