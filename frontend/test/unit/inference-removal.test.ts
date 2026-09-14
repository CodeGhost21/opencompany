// What acting on a provider costs, and the sentences built from it — the
// keys-rework replacement for the routing-era `removalImpact`/`removalWarnings`
// (issue #2306, phase 5b: per-workload routing is gone, so nothing here is
// about routes resetting any more; see `removal.ts`'s own module doc).

import { describe, expect, it } from "vitest";

import { removalImpact, removalWarnings } from "@/inference/removal";
import type { Provider } from "@/inference/types";

function provider(over: Partial<Provider> = {}): Provider {
  return {
    id: "prv_acme",
    slug: "acme",
    label: "Acme",
    kind: "openai_compatible",
    baseUrl: "https://acme.example/v1",
    models: {},
    enabled: true,
    keyConfigured: true,
    ...over,
  };
}

describe("removalImpact", () => {
  it("carries the host's own usedBy straight through", () => {
    const p = provider({ usedBy: { default: true, agents: [{ id: "a1", name: "Researcher" }] } });
    const impact = removalImpact(p, [p]);
    expect(impact.usedBy).toEqual({ default: true, agents: [{ id: "a1", name: "Researcher" }] });
  });

  it("says nothing is used when usedBy is absent", () => {
    const p = provider();
    expect(removalImpact(p, [p]).usedBy).toBeUndefined();
  });

  it("flags the last enabled provider", () => {
    const p = provider({ enabled: true });
    const other = provider({ slug: "beta", enabled: false });
    expect(removalImpact(p, [p, other]).lastEnabled).toBe(true);
  });

  it("does not flag lastEnabled when another provider is still on", () => {
    const p = provider({ enabled: true });
    const other = provider({ slug: "beta", enabled: true });
    expect(removalImpact(p, [p, other]).lastEnabled).toBe(false);
  });
});

describe("removalWarnings: provider (full delete)", () => {
  it("names the credential loss first", () => {
    const [first] = removalWarnings("provider", "Acme", { lastEnabled: false });
    expect(first).toContain("deletes Acme");
    expect(first).toContain("entering the credential again");
  });

  it("says the default is never cleared, even by deleting the row (decision X14)", () => {
    const lines = removalWarnings("provider", "Acme", { lastEnabled: false, usedBy: { default: true } });
    expect(lines.some((l) => l.includes("does not change the default"))).toBe(true);
  });

  it("names every pinned agent, correctly pluralised", () => {
    const one = removalWarnings("provider", "Acme", {
      lastEnabled: false,
      usedBy: { agents: [{ id: "a1", name: "Researcher" }] },
    });
    expect(one.join(" ")).toContain("Pinned by 1 agent: Researcher.");

    const two = removalWarnings("provider", "Acme", {
      lastEnabled: false,
      usedBy: { agents: [{ id: "a1", name: "Researcher" }, { id: "a2", name: "Web search" }] },
    });
    expect(two.join(" ")).toContain("Pinned by 2 agents: Researcher, Web search.");
  });

  it("says when nothing else would be able to think", () => {
    const lines = removalWarnings("provider", "Acme", { lastEnabled: true });
    expect(lines.some((l) => l.includes("only provider switched on"))).toBe(true);
  });

  it("says nothing extra when nothing depends on it and it is not the last one", () => {
    expect(removalWarnings("provider", "Acme", { lastEnabled: false })).toHaveLength(1);
  });
});

describe("removalWarnings: key (clear the credential)", () => {
  it("is explicit that this is not a removal", () => {
    const [first] = removalWarnings("key", "Acme", { lastEnabled: false });
    expect(first).toContain("stays on this page");
    expect(first).toContain("keeping its endpoint");
  });

  it("says the default does NOT change (decision X14)", () => {
    const lines = removalWarnings("key", "Acme", { lastEnabled: false, usedBy: { default: true } });
    expect(lines.join(" ")).toContain("does not change the default");
  });
});

describe("removalWarnings: disable / enable — every toggle confirms now", () => {
  it("disable says it is reversible", () => {
    const [first] = removalWarnings("disable", "Acme", { lastEnabled: false });
    expect(first).toContain("nothing is deleted");
  });

  it("disable says the default does NOT change (decision X14)", () => {
    const lines = removalWarnings("disable", "Acme", { lastEnabled: false, usedBy: { default: true } });
    expect(lines.join(" ")).toContain("does not change the default");
  });

  it("disable names the last-enabled case with different wording from remove", () => {
    const lines = removalWarnings("disable", "Acme", { lastEnabled: true });
    expect(lines.some((l) => l.includes("while it is off"))).toBe(true);
  });

  it("enable is light — nothing it depends on can be stranded by turning it on", () => {
    const lines = removalWarnings("enable", "Acme", { lastEnabled: false });
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain("back on");
  });
});

describe("removalWarnings: other surfaces sharing the credential", () => {
  it("names them, excluding llm itself", () => {
    const lines = removalWarnings("key", "Acme", {
      lastEnabled: false,
      usedBy: { surfaces: ["llm", "composio", "search"] },
    });
    expect(lines.join(" ")).toContain("also used by composio and search");
  });

  it("says nothing when llm is the only surface listed", () => {
    const lines = removalWarnings("key", "Acme", { lastEnabled: false, usedBy: { surfaces: ["llm"] } });
    expect(lines).toHaveLength(1);
  });
});
