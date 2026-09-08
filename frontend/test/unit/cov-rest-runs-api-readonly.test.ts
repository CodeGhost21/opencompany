import { describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { getRun, listRuns } from "@/api/runs";

/**
 * `api/runs.ts`'s own doc header: "Read-only on purpose … There is no write
 * here at all." There is therefore no control for this axis to withhold by
 * role — a run is minted, begun and settled by the dispatch path, never by
 * this module. What is worth pinning, given that promise, is that it still
 * holds: both exports resolve through nothing but `client.get`, so neither
 * one could quietly grow a write a future edit forgets to gate.
 */

function readOnlyClient(): OpenCompanyClient {
  // No post/put/patch/del at all — if either export under test reached for
  // one, the call would throw "is not a function" rather than silently pass.
  return {
    scopeFor: () => "/api/v1/companies/acme",
    get: vi.fn(() => Promise.resolve([])),
  } as unknown as OpenCompanyClient;
}

describe("the runs API, read-only end to end", () => {
  it("lists runs through a client that offers no write method at all", async () => {
    const client = readOnlyClient();
    await expect(listRuns(client, "acme")).resolves.toEqual([]);
    expect(client.get).toHaveBeenCalledWith("/api/v1/companies/acme/runs");
  });

  it("reads one run through a client that offers no write method at all", async () => {
    const client = {
      scopeFor: () => "/api/v1/companies/acme",
      get: vi.fn(() => Promise.resolve({ id: "r1" })),
    } as unknown as OpenCompanyClient;
    await expect(getRun(client, "acme", "r1")).resolves.toEqual({ id: "r1" });
    expect(client.get).toHaveBeenCalledWith("/api/v1/companies/acme/runs/r1");
  });
});
