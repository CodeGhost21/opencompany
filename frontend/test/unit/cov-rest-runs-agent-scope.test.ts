// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { listRuns } from "@/api/runs";

/**
 * `api/runs.ts` documents that `?agent=` is sent to the host rather than
 * applied client-side, and that a caller must not assume every returned row
 * is that desk's own (a host predating the selector answers unfiltered).
 * `agent-runs.test.ts` already proves the *consuming* component drops a
 * misattributed row; this pins the api module itself, at the one place a
 * future edit could silently stop sending the selector at all.
 */

describe("listRuns sends the agent selector the host is asked to narrow by", () => {
  it("puts ?agent= on the wire rather than filtering a fetched page", async () => {
    const get = vi.fn((_path: string) => Promise.resolve([]));
    const client = { scopeFor: () => "/api/v1/company/acme", get } as unknown as OpenCompanyClient;

    await listRuns(client, "acme", { agent: "designer" });

    expect(get).toHaveBeenCalledTimes(1);
    const [path] = get.mock.calls[0];
    expect(path).toContain("agent=designer");
  });
});
