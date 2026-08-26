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
): { client: OpenCompanyClient; calls: string[]; puts: unknown[] } {
  const calls: string[] = [];
  const puts: unknown[] = [];
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
    put: async (_path: string, body: unknown) => {
      puts.push(body);
      return { status: inference, note: "" };
    },
  } as unknown as OpenCompanyClient;
  return { client, calls, puts };
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

  it("strips a catalog-picked id from the stored config when Remove Key is clicked", async () => {
    // A keyed company saved a raw catalog id straight to OpenRouter (allowed
    // while keyed — this is not the proxied path). Remove Key clears the key
    // and, per `wouldSaveProxied`, immediately switches the company onto the
    // platform proxy. `model_for_tier` honours the stored override verbatim on
    // both paths, so the id it just carried over is exactly what the proxy
    // rejects unless Remove Key strips it before saving. The hand-typed tier
    // id on the other tier (not present in the catalog) must survive — Remove
    // Key only clears what the catalog select itself wrote.
    const { client, puts } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "reasoning-v1" },
        true,
      ),
      [{ id: "anthropic/claude-sonnet-5", name: "Claude Sonnet" }],
    );

    await mount(client);

    const button = container.querySelector('[data-testid="inference-remove-key"]') as HTMLButtonElement;
    expect(button).not.toBeNull();
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { key?: string; models?: Record<string, string> };
    expect(body.key).toBe("");
    expect(body.models).toEqual({ "reasoning-v1": "reasoning-v1" });
  });
});

describe("raw OpenRouter registry ids vs the proxy's own passthrough shape (issue #1838 follow-up, fifth instance)", () => {
  it("strips a two-segment OpenRouter-registry id (e.g. openrouter/auto) when Remove Key switches to the proxy", async () => {
    // OpenRouter's own catalog has ids under the `openrouter/` author too —
    // `openrouter/auto` is a real two-segment registry id, not the proxy's
    // three-segment `openrouter/<author>/<slug>` passthrough form. A prefix
    // check that only tests `startsWith("openrouter/")` mistakes the former
    // for the latter and leaves it in place; `model_for_tier` then forwards
    // it to the proxy verbatim, which rejects it.
    const { client, puts } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "openrouter/auto", "reasoning-v1": "reasoning-v1" },
        true,
      ),
      [{ id: "openrouter/auto", name: "Auto" }],
    );

    await mount(client);

    const button = container.querySelector('[data-testid="inference-remove-key"]') as HTMLButtonElement;
    expect(button).not.toBeNull();
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { key?: string; models?: Record<string, string> };
    expect(body.key).toBe("");
    expect(body.models).toEqual({ "reasoning-v1": "reasoning-v1" });
  });

  it("keeps the proxy's genuine three-segment openrouter/<author>/<slug> passthrough id", async () => {
    // The exemption exists for this shape specifically — an operator who
    // enabled proxy passthrough and saved its own `openrouter/<author>/<slug>`
    // form must not have it stripped out from under them by the same fix.
    const { client, puts } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "openrouter/anthropic/claude-3-opus", "reasoning-v1": "reasoning-v1" },
        true,
      ),
      [{ id: "openrouter/anthropic/claude-3-opus", name: "Claude 3 Opus (passthrough)" }],
    );

    await mount(client);

    const button = container.querySelector('[data-testid="inference-remove-key"]') as HTMLButtonElement;
    expect(button).not.toBeNull();
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { key?: string; models?: Record<string, string> };
    expect(body.models).toEqual({
      "chat-v1": "openrouter/anthropic/claude-3-opus",
      "reasoning-v1": "reasoning-v1",
    });
  });
});

describe("clearing a tier override back to the tier default (issue #1838 follow-up)", () => {
  it("offers a 'Use the tier default' item in the catalog select for a tier with a saved override", async () => {
    // A keyed OpenRouter company with a saved override and a loaded catalog
    // used to only ever offer concrete models — no way to remove the one
    // mapping and let `model_for_tier` fall back to its own default for that
    // tier. Opening the select must show an explicit way out.
    const { client } = clientFor(
      status("openrouter", { "chat-v1": "anthropic/claude-sonnet-5" }, true),
      [{ id: "anthropic/claude-sonnet-5", name: "Claude Sonnet" }],
    );

    await mount(client);

    const trigger = container.querySelector(
      '[data-testid="inference-model-select-chat-v1"]',
    ) as HTMLButtonElement;
    expect(trigger).not.toBeNull();
    await act(async () => {
      trigger.click();
    });
    await act(async () => {});

    expect(document.body.querySelector('[data-testid="inference-model-clear-chat-v1"]')).not.toBeNull();
  });

  it("clears the tier override when 'Use the tier default' is picked, and Save drops it from the wire", async () => {
    const { client, puts } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "anthropic/agentic" },
        true,
      ),
      [
        { id: "anthropic/claude-sonnet-5", name: "Claude Sonnet" },
        { id: "anthropic/agentic", name: "Agentic" },
      ],
    );

    await mount(client);

    const trigger = container.querySelector(
      '[data-testid="inference-model-select-chat-v1"]',
    ) as HTMLButtonElement;
    await act(async () => {
      trigger.click();
    });
    await act(async () => {});

    const clearItem = document.body.querySelector(
      '[data-testid="inference-model-clear-chat-v1"]',
    ) as HTMLElement;
    expect(clearItem).not.toBeNull();
    await act(async () => {
      clearItem.click();
    });
    await act(async () => {});

    // Cleared tier reverts to the placeholder select (no stored value); the
    // untouched tier keeps its override and stays a select, not free text.
    expect(container.querySelector("#inference-model-chat-v1")?.textContent).not.toContain(
      "anthropic/claude-sonnet-5",
    );
    expect(container.querySelector("#inference-model-reasoning-v1")?.textContent).toContain(
      "Agentic",
    );

    const save = container.querySelector('[data-testid="inference-save"]') as HTMLButtonElement;
    await act(async () => {
      save.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { models?: Record<string, string> };
    expect(body.models).toEqual({ "reasoning-v1": "anthropic/agentic" });
  });
});

describe("proxy-incompatible overrides survive an unready catalog (issue #1838 follow-up)", () => {
  it("strips a raw catalog id from the draft while the registry is still loading, before Save is clicked", async () => {
    // Third instance of the #1838 class: the earlier fix only stripped a
    // tier value once `modelCatalog.kind === "ready"`, so a keyless company
    // that already has a raw `<author>/<model>` override stored (from an
    // earlier keyed session, or from switching onto OpenRouter's own preset)
    // kept offering it back to Save for as long as the registry request was
    // still in flight — which, on a slow network, can be indefinitely.
    const calls: string[] = [];
    const puts: unknown[] = [];
    const inference = status(
      "openrouter",
      { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "reasoning-v1" },
      false,
    );
    const client = {
      scopeFor: () => "/api/v1/companies/acme",
      get: async (path: string) => {
        calls.push(path);
        if (path.endsWith("/inference/models")) {
          // Never resolves — the registry request is permanently in flight
          // for the duration of this test.
          return new Promise(() => {});
        }
        return inference;
      },
      put: async (_path: string, body: unknown) => {
        puts.push(body);
        return { status: inference, note: "" };
      },
    } as unknown as OpenCompanyClient;

    await mount(client);

    // Still loading, never reached "ready".
    expect(container.querySelector('[data-testid="inference-model-select-chat-v1"]')).toBeNull();
    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty("value", "");
    expect(container.querySelector("input#inference-model-reasoning-v1")).toHaveProperty(
      "value",
      "reasoning-v1",
    );

    const button = container.querySelector('[data-testid="inference-save"]') as HTMLButtonElement;
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { models?: Record<string, string> };
    expect(body.models).toEqual({ "reasoning-v1": "reasoning-v1" });
  });

  it("does not let Save persist a raw catalog id when the registry failed to load", async () => {
    // Same class, the "or has failed" half: a failed registry fetch also
    // never reaches `kind === "ready"`.
    const { client, puts } = clientFor(
      status(
        "openrouter",
        { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "reasoning-v1" },
        false,
      ),
      new Error("registry unreachable"),
    );

    await mount(client);

    expect(container.querySelector('[data-testid="inference-model-catalog-fallback"]')).not.toBeNull();
    expect(container.querySelector("input#inference-model-chat-v1")).toHaveProperty("value", "");

    const button = container.querySelector('[data-testid="inference-save"]') as HTMLButtonElement;
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { models?: Record<string, string> };
    expect(body.models).toEqual({ "reasoning-v1": "reasoning-v1" });
  });

  it("strips a raw catalog id from Remove Key's carried models when the registry fetch fails", async () => {
    // Fourth instance: Remove Key used to fetch the registry itself to
    // decide what to carry over, and fell back to sending the stored
    // overrides completely unfiltered when that fetch failed — the one
    // outcome guaranteed to break every proxied tier, on exactly the
    // condition (registry unreachable) that triggers it.
    const inference = status(
      "openrouter",
      { "chat-v1": "anthropic/claude-sonnet-5", "reasoning-v1": "reasoning-v1" },
      true,
    );
    const puts: unknown[] = [];
    const client = {
      scopeFor: () => "/api/v1/companies/acme",
      get: async (path: string) => {
        if (path.endsWith("/inference/models")) throw new Error("registry unreachable");
        return inference;
      },
      put: async (_path: string, body: unknown) => {
        puts.push(body);
        return { status: inference, note: "" };
      },
    } as unknown as OpenCompanyClient;

    await mount(client);

    const button = container.querySelector('[data-testid="inference-remove-key"]') as HTMLButtonElement;
    expect(button).not.toBeNull();
    await act(async () => {
      button.click();
    });
    await act(async () => {});

    expect(puts).toHaveLength(1);
    const body = puts[0] as { key?: string; models?: Record<string, string> };
    expect(body.key).toBe("");
    expect(body.models).toEqual({ "reasoning-v1": "reasoning-v1" });
  });
});
