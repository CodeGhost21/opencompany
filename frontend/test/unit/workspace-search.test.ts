/**
 * Workspace search on the console side (issue #607).
 *
 * Two things are worth pinning here and neither needs a browser: the request
 * this client builds (an empty `q` is a 400 on the host, so the caller must
 * never send one, and the scope/limit params have to land where the route reads
 * them), and the highlighter, which is the one piece of real logic in the hit
 * list.
 */
import { describe, expect, it } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import { highlightRuns, searchWorkspace } from "@/api/workspace";

function client(handler: (req: { method: string; url: string }) => unknown) {
  const transport = {
    request: async (req: { method: string; url: string; body?: string }) => ({
      status: 200,
      statusText: "OK",
      url: req.url,
      text: JSON.stringify(handler(req)),
      header: () => null,
    }),
    subscribe: () => () => {},
  };
  return new OpenCompanyClient(
    { baseUrl: "", company: "acme", operatorToken: "t0ken" },
    transport as never,
  );
}

describe("searchWorkspace", () => {
  it("sends the query, the scope and the limit where the route reads them", async () => {
    const seen: string[] = [];
    const c = client((req) => {
      seen.push(req.url);
      return { hits: [], total: 0 };
    });

    await searchWorkspace(c, "acme", "refund policy");
    await searchWorkspace(c, "acme", "refund", { prefix: "Standards", limit: 5 });

    expect(seen[0]).toContain("/workspace/search?q=refund+policy");
    expect(seen[1]).toContain("q=refund");
    expect(seen[1]).toContain("prefix=Standards");
    expect(seen[1]).toContain("limit=5");
  });

  it("normalizes a hit's node half exactly as the tree read does", async () => {
    const c = client(() => ({
      total: 3,
      hits: [
        {
          // No `parentId`, no origins — the wire shape at the workspace root,
          // against a host that predates issue #326.
          id: "n-1",
          name: "Support.md",
          kind: "file",
          updatedAt: 42,
          path: "Support.md",
          matched: "content",
          excerpt: "…a REFUND request…",
        },
      ],
    }));

    const results = await searchWorkspace(c, "acme", "refund");
    expect(results.total).toBe(3);
    expect(results.hits).toHaveLength(1);
    const hit = results.hits[0];
    // An absent `parentId` becomes an explicit null once, here, exactly as
    // `fetchTree` does — every tree query in the view keys off `=== null`.
    expect(hit.parentId).toBeNull();
    expect(hit.createdBy).toEqual({ kind: "operator" });
    expect(hit.updatedBy).toEqual({ kind: "operator" });
    // …and the search-only fields survive the normalization.
    expect(hit.path).toBe("Support.md");
    expect(hit.matched).toBe("content");
    expect(hit.excerpt).toBe("…a REFUND request…");
  });

  it("keeps the total separate from the page, so a capped answer says so", async () => {
    const c = client(() => ({
      total: 137,
      hits: [{ id: "n-1", name: "a.md", kind: "file", updatedAt: 1, path: "a.md", matched: "name" }],
    }));
    const results = await searchWorkspace(c, "acme", "a");
    expect(results.hits).toHaveLength(1);
    expect(results.total).toBe(137);
  });
});

describe("highlightRuns", () => {
  it("marks every occurrence and returns the original casing", () => {
    const runs = highlightRuns("A REFUND and a refund", "refund");
    expect(runs.map((r) => r.text).join("")).toBe("A REFUND and a refund");
    // Both occurrences are marked, and each keeps the case it was written in —
    // highlighting must never rewrite the operator's prose.
    expect(runs.filter((r) => r.hit).map((r) => r.text)).toEqual(["REFUND", "refund"]);
  });

  it("marks nothing for an empty or whitespace query", () => {
    for (const query of ["", "   "]) {
      const runs = highlightRuns("a refund", query);
      expect(runs).toEqual([{ text: "a refund", hit: false }]);
    }
  });

  it("handles a match at either end without emitting empty runs", () => {
    expect(highlightRuns("refund policy", "refund")).toEqual([
      { text: "refund", hit: true },
      { text: " policy", hit: false },
    ]);
    expect(highlightRuns("a refund", "refund")).toEqual([
      { text: "a ", hit: false },
      { text: "refund", hit: true },
    ]);
    expect(highlightRuns("refund", "refund")).toEqual([{ text: "refund", hit: true }]);
  });

  it("does not loop forever on back-to-back matches", () => {
    expect(highlightRuns("aaaa", "aa")).toEqual([
      { text: "aa", hit: true },
      { text: "aa", hit: true },
    ]);
  });

  it("leaves text alone when the query does not occur", () => {
    expect(highlightRuns("a refund", "dividend")).toEqual([{ text: "a refund", hit: false }]);
  });
});
