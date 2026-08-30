import { describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { listWorkflowRuns } from "@/api/workflows";

/**
 * `beforeSeq: 0` is a real cursor and must reach the wire (issue #1012
 * follow-up).
 *
 * The host issues the page's lowest `seq` as its pagination boundary, and a
 * journal's first row is `seq` 0 — so 0 is exactly the cursor a company whose
 * history reaches the beginning gets handed. A truthy check drops it, the
 * request goes out with no `before_seq` at all, and the host answers the
 * NEWEST page: the drawer appends the rows it already has, "Load older" stays
 * enabled, and every further click repeats the same page forever.
 */
describe("listWorkflowRuns cursor", () => {
  function spy(): { calls: string[]; client: OpenCompanyClient } {
    const calls: string[] = [];
    const client = {
      scopeFor: () => "/api/v1/company",
      get: async (path: string) => {
        calls.push(path);
        return { runs: [], hasMore: false };
      },
    } as unknown as OpenCompanyClient;
    return { calls, client };
  }

  it("sends before_seq=0 rather than dropping it as falsy", async () => {
    const { calls, client } = spy();
    await listWorkflowRuns(client, null, { workflow: "wf", limit: 50, beforeSeq: 0 });
    expect(calls).toHaveLength(1);
    expect(calls[0]).toContain("before_seq=0");
  });

  it("still omits before_seq when no cursor was given", async () => {
    const { calls, client } = spy();
    await listWorkflowRuns(client, null, { workflow: "wf", limit: 50 });
    expect(calls[0]).not.toContain("before_seq");
  });
});
