import { describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { listWorkflowToolSlugs } from "@/api/workflows";

/**
 * `GET …/workflows/tool-slugs` on the client (issues #783, #874).
 *
 * Since #874 the route answers two lists — the **effective** slugs a proposal
 * may ground on, and the granted-but-unwired ones it must not — and the client's
 * whole job is to hand both on without losing the distinction. The one thing it
 * adds is the back-compat default: a host predating #874 sends no `unwired` key,
 * and "the key is missing" and "nothing is unwired" mean the same thing to every
 * caller, so it normalises to `[]` rather than leaking `undefined` into the
 * copilot context.
 */

/** A fake client that answers one canned body and records the path asked for. */
function readingClient(
  body: unknown,
  sink: { path?: string } = {},
): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      sink.path = path;
      return body as T;
    },
  } as unknown as OpenCompanyClient;
}

describe("listWorkflowToolSlugs", () => {
  it("reads both lists from the scoped route", async () => {
    const sink: { path?: string } = {};
    const result = await listWorkflowToolSlugs(
      readingClient(
        {
          slugs: ["shell", "send_email"],
          unwired: [
            {
              slug: "web_search",
              reason: "searchBackendNotConfigured",
              detail: "granted, but no managed search backend is configured",
            },
          ],
        },
        sink,
      ),
      "acme",
    );
    expect(sink.path).toBe("/api/v1/acme/workflows/tool-slugs");
    expect(result.slugs).toEqual(["shell", "send_email"]);
    expect(result.unwired).toHaveLength(1);
    expect(result.unwired[0].slug).toBe("web_search");
    expect(result.unwired[0].reason).toBe("searchBackendNotConfigured");
  });

  /**
   * The point of the split: a granted-but-unwired tool must never appear in the
   * list a prompt is grounded on. That is issue #874 — `web_search` was offered,
   * the copilot authored a node on it, and the run died at the first node.
   */
  it("keeps an unwired tool out of the groundable set", async () => {
    const result = await listWorkflowToolSlugs(
      readingClient({
        slugs: ["shell"],
        unwired: [{ slug: "web_search", reason: "x", detail: "y" }],
      }),
      null,
    );
    expect(result.slugs).not.toContain("web_search");
    expect(result.unwired.map((t) => t.slug)).toContain("web_search");
  });

  it("defaults `unwired` to an empty list on a host predating issue #874", async () => {
    const result = await listWorkflowToolSlugs(
      readingClient({ slugs: ["shell"] }),
      "acme",
    );
    expect(result.slugs).toEqual(["shell"]);
    expect(result.unwired).toEqual([]);
  });
});
