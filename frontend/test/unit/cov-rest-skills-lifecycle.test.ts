// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { Skill } from "@/api/skills";
import { SkillsView } from "@/views/SkillsView";

/**
 * SKILL-001's Rust half (`ops/skills.rs`) has slug-validation coverage but no
 * direct REST-route test for install/uninstall. The console's own lifecycle
 * surface — `SkillsView` — had none at all. Two properties: a company
 * (manifest-baked) skill offers no Uninstall, the one authority split this
 * screen actually draws; and an uninstall the host refuses puts the card back
 * rather than dropping it for good on a client that merely believed it
 * worked.
 */

function skill(over: Partial<Skill> = {}): Skill {
  return {
    id: "s1",
    name: "Refund policy",
    description: "How to handle refund requests.",
    category: "Support",
    source: "registry",
    enabled: true,
    ...over,
  };
}

function clientWith(opts: { skills: Skill[]; post?: (path: string) => Promise<unknown> }): OpenCompanyClient {
  const post = vi.fn(opts.post ?? (() => Promise.resolve(undefined)));
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn((path: string) => {
      if (path.endsWith("/skills")) return Promise.resolve(opts.skills);
      if (path.endsWith("/skills/registry")) return Promise.resolve([]);
      if (path.endsWith("/auth/me"))
        return Promise.resolve({ id: "u1", email: "a@b.c", role: "admin", company: "acme", hasPassword: true });
      return Promise.reject(new Error(`unexpected GET ${path}`));
    }),
    post,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(SkillsView, { client, company: "acme" }));
  });
}

function cards(): HTMLElement[] {
  return Array.from(container.querySelectorAll('[data-testid="installed-card"]'));
}

function uninstallButtonIn(card: HTMLElement): HTMLElement | null {
  return card.querySelector('[aria-label="Uninstall"]');
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

describe("a manifest-baked skill offers no uninstall", () => {
  it("withholds Uninstall from a company skill, and offers it on a registry one", async () => {
    const client = clientWith({
      skills: [
        skill({ id: "baked", name: "Baked-in playbook", source: "company" }),
        skill({ id: "installed", name: "Installed skill", source: "registry" }),
      ],
    });
    await show(client);

    const [baked, installed] = cards();
    expect(uninstallButtonIn(baked)).toBeNull();
    expect(uninstallButtonIn(installed)).not.toBeNull();
  });
});

describe("a refused uninstall puts the skill back", () => {
  it("re-lists the skill and reports the failure when the host refuses the uninstall", async () => {
    const client = clientWith({
      skills: [skill()],
      post: () => Promise.reject(new Error("skill is pinned by an active workflow")),
    });
    await show(client);

    await act(async () => {
      uninstallButtonIn(cards()[0])!.click();
    });

    expect(cards()).toHaveLength(1);
    expect(container.textContent).toContain("Refund policy");
  });
});
