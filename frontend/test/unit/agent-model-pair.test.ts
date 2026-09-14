import { describe, expect, it } from "vitest";

import {
  agentPairBrokenCopy,
  companyDefaultLabel,
  pairLabel,
  providerEdit,
  resolveAgentDefault,
} from "@/lib/agent";
import type { AgentDetailDto } from "@/api/types";
import type { Provider } from "@/inference/types";

/**
 * The agent pair editor's pure derivations (keys rework, issue #2306, slice
 * 3b). An agent on a built-in harness may pin its own `{provider, model}`
 * pair; unpinned, it falls back to the company default; with neither, it
 * fails closed (D-model). Every sentence here is decision X9's, verbatim.
 */

function agent(over: Partial<AgentDetailDto> = {}): AgentDetailDto {
  return {
    id: "researcher",
    name: "Researcher",
    role: "Researcher",
    source: "overlay",
    editable: ["name", "role", "harness", "model", "provider"],
    isOrchestrator: false,
    tools: { requested: null, companyAllow: ["*"], deskAllow: [], deskCeilingActive: false, effective: ["*"] },
    desks: [],
    inboxEnabled: false,
    ...over,
  };
}

function provider(over: Partial<Provider> = {}): Provider {
  return {
    id: "prv_anthropic",
    slug: "anthropic",
    label: "Anthropic",
    kind: "anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    models: {},
    enabled: true,
    keyConfigured: true,
    ...over,
  };
}

describe("providerEdit — the sibling of modelEdit/harnessEdit", () => {
  it("sends nothing when the draft did not change", () => {
    expect(providerEdit(undefined, "")).toBeUndefined();
    expect(providerEdit("anthropic", "anthropic")).toBeUndefined();
  });

  it("sends the slug when a provider is chosen", () => {
    expect(providerEdit(undefined, "anthropic")).toBe("anthropic");
  });

  it("sends null to clear back to the company default", () => {
    expect(providerEdit("anthropic", "")).toBeNull();
  });
});

describe("companyDefaultLabel", () => {
  const providers = [provider({ slug: "openrouter", label: "OpenRouter" })];

  it("names the provider and model when the default is full", () => {
    expect(companyDefaultLabel({ provider: "openrouter", model: "acme/test-model" }, providers)).toBe(
      "Company default · OpenRouter · acme/test-model",
    );
  });

  it("falls back to the slug when the provider is not in the list", () => {
    expect(companyDefaultLabel({ provider: "ghost", model: "x" }, providers)).toBe(
      "Company default · ghost · x",
    );
  });

  it("says none chosen for a bare-slug default or no default at all", () => {
    expect(companyDefaultLabel({ provider: "openrouter", model: null }, providers)).toBe(
      "Company default · none chosen",
    );
    expect(companyDefaultLabel(null, providers)).toBe("Company default · none chosen");
    expect(companyDefaultLabel(undefined, providers)).toBe("Company default · none chosen");
  });
});

describe("pairLabel", () => {
  it("names the provider by label and the model", () => {
    const providers = [provider({ slug: "anthropic", label: "Anthropic" })];
    expect(pairLabel("anthropic", "test-model-large", providers)).toBe("Anthropic · test-model-large");
  });

  it("falls back to the slug for a provider not in the list (a gone or disabled pin, F6)", () => {
    expect(pairLabel("gone", "x", [])).toBe("gone · x");
  });
});

describe("resolveAgentDefault — what an unpinned agent's fallback line says (decisions X9, X14)", () => {
  const healthy = [provider({ slug: "openrouter", label: "OpenRouter", enabled: true })];

  it("shows the full company default", () => {
    const resolution = resolveAgentDefault({ provider: "openrouter", model: "acme/test-model" }, healthy, "Writer");
    expect(resolution).toEqual({ kind: "full", label: "Company default · OpenRouter · acme/test-model" });
  });

  it("says nothing is chosen when the default is entirely unset", () => {
    const resolution = resolveAgentDefault(null, healthy, "Writer");
    expect(resolution.kind).toBe("none");
    expect(resolution.kind === "none" && resolution.message).toBe(
      "No model is chosen. Choose a provider and model for Writer, or set the company default in API Keys → LLM.",
    );
  });

  it("says nothing is chosen for a bare-slug default (no model)", () => {
    const resolution = resolveAgentDefault({ provider: "openrouter", model: null }, healthy, "Writer");
    expect(resolution.kind).toBe("none");
  });

  it("names a removed or disabled default provider — durable per X14, never silently cleared", () => {
    const removed = resolveAgentDefault({ provider: "ghost", model: "x" }, healthy, "Writer");
    expect(removed).toEqual({
      kind: "broken",
      message: "The company default uses ghost, which is removed. Choose a new default in API Keys → LLM.",
    });

    const disabled = resolveAgentDefault(
      { provider: "openrouter", model: "x" },
      [provider({ slug: "openrouter", label: "OpenRouter", enabled: false })],
      "Writer",
    );
    expect(disabled.kind).toBe("broken");
    expect(disabled.kind === "broken" && disabled.message).toContain("turned off");
  });
});

describe("agentPairBrokenCopy — a pinned pair naming a gone or disabled provider (X9, F6)", () => {
  const providers = [provider({ slug: "anthropic", label: "Anthropic", enabled: true })];

  it("says nothing when the pin is healthy", () => {
    expect(agentPairBrokenCopy(agent({ provider: "anthropic", model: "test-model-large" }), providers)).toBeNull();
  });

  it("says nothing when there is no pin at all", () => {
    expect(agentPairBrokenCopy(agent({ provider: undefined, model: undefined }), providers)).toBeNull();
  });

  it("names the agent, the provider, and the fix — for a removed provider", () => {
    expect(agentPairBrokenCopy(agent({ provider: "gone", model: "x" }), providers)).toBe(
      "Researcher uses gone, which is removed. Choose another provider and model for Researcher, or clear its model to use the company default.",
    );
  });

  it("names a switched-off provider", () => {
    const off = [provider({ slug: "anthropic", label: "Anthropic", enabled: false })];
    expect(agentPairBrokenCopy(agent({ provider: "anthropic", model: "x" }), off)).toContain("turned off");
  });

  it("falls back to 'This teammate' for a manifest agent with no name", () => {
    const manifest = agent({ name: undefined, provider: "gone", model: "x" });
    expect(agentPairBrokenCopy(manifest, providers)).toContain("This teammate uses gone");
  });
});
