// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InferenceModel, InferenceStatus } from "@/api/inference";
import { InferenceSection } from "@/views/connections/InferenceSection";

let container: HTMLDivElement;
let root: Root;

function status(
  provider: string,
  models: Record<string, string>,
  keyConfigured = true,
): InferenceStatus {
  return {
    provider,
    slug: provider === "openai_compatible" ? "byok" : provider,
    baseUrl: provider === "openai_compatible" ? "https://models.example/v1" : "https://openrouter.ai/api/v1",
    models,
    source: "runtime",
    keyConfigured,
    cognition: "harness",
    usageMetering: "perTurn",
    restartRequired: false,
    harnessReachable: true,
    canRebuildInPlace: true,
  };
}

function clientFor(
  inference: InferenceStatus,
  catalog: InferenceModel[] | Error,
): { client: OpenCompanyClient; calls: string[] } {
  const calls: string[] = [];
  const client = {
    scopeFor: () => "/api/v1/companies/acme",
    get: async (path: string) => {
      calls.push(path);
      if (path.endsWith("/inference/models")) {
        if (catalog instanceof Error) throw catalog;
        return catalog;
      }
      return inference;
    },
  } as unknown as OpenCompanyClient;
  return { client, calls };
}

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(InferenceSection, { client, company: "acme", canManage: true }));
  });
  await act(async () => {});
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
});

describe("OpenRouter tier model pickers", () => {
  it("renders one registry-backed select per tier and preserves a custom stored id", async () => {
    const { client } = clientFor(
      status("openrouter", {
        "chat-v1": "operator/custom-chat",
        "reasoning-v1": "openai/reasoning",
        "agentic-v1": "anthropic/agentic",
        "vision-v1": "google/vision",
      }),
      [
        { id: "openai/reasoning", name: "Reasoning" },
        { id: "anthropic/agentic", name: "Agentic" },
        { id: "google/vision", name: "Vision" },
      ],
    );

    await mount(client);

    for (const tier of ["chat-v1", "reasoning-v1", "agentic-v1", "vision-v1"]) {
      expect(container.querySelector(`[data-testid="inference-model-select-${tier}"]`)).not.toBeNull();
      expect(container.querySelector(`input#inference-model-${tier}`)).toBeNull();
    }
    expect(container.querySelector("#inference-model-chat-v1")?.textContent).toContain(
      "operator/custom-chat",
    );
  });

  it("falls back to editable model ids when the registry request fails", async () => {
    const { client } = clientFor(
      status("openrouter", { "chat-v1": "operator/custom-chat" }),
      new Error("offline"),
    );

    await mount(client);

    expect(container.querySelector('[data-testid="inference-model-catalog-fallback"]')).not.toBeNull();
    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty(
      "value",
      "operator/custom-chat",
    );
  });

  it("keeps free-text inputs for a proxied OpenRouter company with no key configured", async () => {
    // No stored key -> the platform's subscription proxy resolves the tier,
    // which only accepts an abstract tier name (or its own disabled-by-default
    // `openrouter/<author>/<model>` passthrough). The registry's raw catalog
    // ids the select would save are exactly what that proxy rejects, so the
    // picker must not offer them here even though the catalog loaded fine.
    const { client } = clientFor(
      status("openrouter", { "chat-v1": "chat-v1" }, false),
      [{ id: "anthropic/claude-sonnet-5", name: "Claude Sonnet" }],
    );

    await mount(client);

    expect(container.querySelector('[data-testid="inference-model-catalog-proxied"]')).not.toBeNull();
    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty("value", "chat-v1");
    expect(container.querySelector('[data-testid="inference-model-select-chat-v1"]')).toBeNull();
  });

  it("clears a catalog-picked id from the tier field once the form would save proxied", async () => {
    // Mirrors what a key-clear leaves behind: a company stores a raw
    // `<author>/<model>` id (exactly what the catalog select writes) while no
    // key is configured. `model_for_tier` honours a tier override verbatim on
    // *both* the direct and proxied paths, so this id would ride straight to
    // the platform proxy, which only resolves an abstract tier name (or its
    // own disabled-by-default `openrouter/<author>/<model>` passthrough) —
    // the proxy rejects it. The free-text field that reappears here must not
    // keep offering that id back to Save; a tier id the operator typed by
    // hand (not present in the catalog) is unaffected.
    const { client } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "reasoning-v1" },
        false,
      ),
      [{ id: "anthropic/claude-sonnet-5", name: "Claude Sonnet" }],
    );

    await mount(client);

    expect(container.querySelector('[data-testid="inference-model-catalog-proxied"]')).not.toBeNull();
    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty("value", "");
    expect(container.querySelector("input#inference-model-reasoning-v1")).toHaveProperty(
      "value",
      "reasoning-v1",
    );
  });

  it("keeps free-text inputs for non-OpenRouter providers without fetching the registry", async () => {
    const { client, calls } = clientFor(
      status("openai_compatible", { "chat-v1": "private/model" }),
      [],
    );

    await mount(client);

    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty(
      "value",
      "private/model",
    );
    expect(calls.some((path) => path.endsWith("/inference/models"))).toBe(false);
  });
});
