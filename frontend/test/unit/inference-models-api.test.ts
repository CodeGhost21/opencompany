import { describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { listInferenceModels } from "@/api/inference";

describe("the inference model catalog client", () => {
  it("gets the addressed company's model-list route", async () => {
    const calls: string[] = [];
    const client = {
      scopeFor: (company: string | null) =>
        company ? `/api/v1/companies/${company}` : "/api/v1/company",
      get: async (path: string) => {
        calls.push(path);
        return {
          baseUrl: "https://provider.example/v1",
          models: [{ id: "provider/model", name: "Model" }],
          tierVocabulary: "unknown",
          tierDefaults: {},
        };
      },
    } as unknown as OpenCompanyClient;

    // The catalog travels whole: the route answers with the endpoint that was
    // read and what its vocabulary implies, not just a list of ids. The console
    // needs `baseUrl` to say *whose* list it is showing, and `tierDefaults` to
    // prefill from the configured endpoint rather than from OpenRouter's ids.
    await expect(listInferenceModels(client, "acme")).resolves.toEqual({
      baseUrl: "https://provider.example/v1",
      models: [{ id: "provider/model", name: "Model" }],
      tierVocabulary: "unknown",
      tierDefaults: {},
    });
    expect(calls).toEqual(["/api/v1/companies/acme/inference/models"]);
  });

  it("passes an unreadable catalog's explanation through rather than an empty list alone", async () => {
    // An unreadable catalog is a 200 carrying `error`, so the console can name
    // the endpoint that did not answer instead of rendering an empty picker
    // that reads as "this provider publishes no models". `tierVocabulary` is
    // `null` in that case, which is a different thing from `"unknown"`.
    //
    // The stub sends an explicit `null` because that is what the host sends:
    // `ModelCatalogDto::tier_vocabulary` carries no `skip_serializing_if`, so
    // the key is present and null rather than omitted. Asserting
    // `toBeUndefined()` against a stub that omitted the field passed for a
    // reason nothing real reproduces (CodeRabbit review on #2045).
    const client = {
      scopeFor: () => "/api/v1/company",
      get: async () => ({
        baseUrl: "http://localhost:11434/v1",
        models: [],
        tierVocabulary: null,
        tierDefaults: {},
        error: "Could not list models from http://localhost:11434/v1: connection refused.",
      }),
    } as unknown as OpenCompanyClient;

    const catalog = await listInferenceModels(client, null);
    expect(catalog.models).toEqual([]);
    expect(catalog.tierVocabulary).toBeNull();
    expect(catalog.error).toContain("http://localhost:11434/v1");
  });
});
