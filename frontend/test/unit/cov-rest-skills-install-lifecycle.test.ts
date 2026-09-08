// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RegistrySkill, Skill } from "@/api/skills";
import { SkillsView } from "@/views/SkillsView";

/**
 * `install`/`uninstall` (`server/ops/skills.rs:306,373`) are `ScopedCompany`
 * — the slug-traversal guard the existing Rust suite covers is the only
 * thing gating them, not a role — so `SkillsView` offering both to a plain
 * member with no `canManage` check of its own is correct, matching the
 * route. What had no test at all was the lifecycle itself: install landing
 * moves a registry tile to "Installed", and uninstall a refusal must leave
 * the card in place with an honest toast rather than vanishing it on a
 * write that never actually landed.
 */

const REGISTRY: RegistrySkill = {
  id: "s-standup",
  name: "Standup writer",
  description: "Drafts a standup update.",
  category: "productivity",
  publisher: "opencompany",
};

const INSTALLED: Skill = {
  id: "s-standup",
  name: "Standup writer",
  description: "Drafts a standup update.",
  category: "productivity",
  enabled: true,
  source: "registry",
};

function clientAs(opts: {
  skills?: Skill[];
  install?: () => Promise<Skill>;
  uninstall?: () => Promise<void>;
}): OpenCompanyClient {
  const install = opts.install ?? (() => Promise.resolve(INSTALLED));
  const uninstall = opts.uninstall ?? (() => Promise.resolve());
  return {
    scopeFor: () => "/api/v1/companies/acme",
    get: (path: string) => {
      if (path.includes("/skills/registry")) return Promise.resolve([REGISTRY]);
      return Promise.resolve(opts.skills ?? []);
    },
    post: vi.fn((path: string) => {
      if (path.endsWith("/uninstall")) return uninstall();
      return install();
    }),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(element: React.ReactElement) {
  await act(async () => {
    root.render(element);
  });
}

function findButton(text: string): HTMLButtonElement | null {
  return (
    (Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes(text),
    ) as HTMLButtonElement | undefined) ?? null
  );
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

describe("SkillsView, a member with no canManage gate of its own", () => {
  it("offers install on the registry tab, matching the ungated ScopedCompany route", async () => {
    const client = clientAs({ skills: [] });
    await show(createElement(SkillsView, { client, company: "acme" }));
    await act(async () => {});
    findButton("Registry")!.click();
    await act(async () => {});

    expect(findButton("Install")).not.toBeNull();
  });
});

describe("install lifecycle", () => {
  it("flips the registry tile from Install to Installed once the write actually lands", async () => {
    const client = clientAs({ skills: [] });
    await show(createElement(SkillsView, { client, company: "acme" }));
    await act(async () => {});
    findButton("Registry")!.click();
    await act(async () => {});

    const card = container.querySelector('[data-testid="registry-card"]')!;
    expect(card.textContent).toContain("Install");

    await act(async () => {
      (card.querySelector("button") as HTMLButtonElement).click();
    });

    expect(card.querySelector("button")).toBeNull();
    expect(card.textContent).toContain("Installed");
  });
});

describe("uninstall lifecycle", () => {
  it("leaves the card installed and says why when the host refuses the uninstall", async () => {
    const client = clientAs({
      skills: [INSTALLED],
      uninstall: () => Promise.reject(new Error("this skill is pinned by the manifest")),
    });
    await show(createElement(SkillsView, { client, company: "acme" }));
    await act(async () => {});

    expect(container.textContent).toContain("Standup writer");
    const uninstallButton = container.querySelector<HTMLButtonElement>(
      '[data-testid="installed-card"] button[aria-label="Uninstall"]',
    )!;
    await act(async () => {
      uninstallButton.click();
    });

    // Still on screen — the refusal must not have removed the card.
    expect(container.textContent).toContain("Standup writer");
  });
});
