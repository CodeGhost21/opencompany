import { describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { bindRepo, listRepos, revokeRepo, type Repo } from "@/api/repos";

/**
 * Issue #245, console half.
 *
 * Two properties are worth pinning at this layer, and only these two — the
 * behaviour lives on the host, and duplicating its rules here would produce a
 * second set that can disagree with it.
 *
 * 1. **The read never carries a credential.** The card renders whatever this
 *    module returns, so a token that reached here would reach a browser, a
 *    screenshot and a support thread. The shape has a fingerprint field and no
 *    token field, and the mapping must not invent one.
 * 2. **A wrong body is a load error, not a value.** `client.get<T>` casts an
 *    unparsed body straight to `T`, so the declared type is a claim about the
 *    host and never a check on it — the failure #414 documented. A host that
 *    answers something other than the promised shape must fail here rather
 *    than several renders later.
 */

/** A client that records calls and replays a scripted body. */
function fakeClient(body: unknown) {
  const calls: Array<{ method: string; path: string; body?: unknown }> = [];
  const client = {
    scopeFor: (company: string | null) =>
      company ? `/api/v1/companies/${company}` : "/api/v1/company",
    get: async (path: string) => {
      calls.push({ method: "GET", path });
      return body;
    },
    post: async (path: string, payload?: unknown) => {
      calls.push({ method: "POST", path, body: payload });
      return body;
    },
  } as unknown as OpenCompanyClient;
  return { client, calls };
}

const BOUND: Repo = {
  key: "acme-widgets",
  url: "https://github.com/acme/widgets",
  owner: "acme",
  repo: "widgets",
  branches: ["main"],
  tokenFingerprint: "0f1e2d3c4b5a",
  lastFetchedMillis: 1_700_000_000_000,
  sizeBytes: 4096,
  boundAtMillis: 1_700_000_000_000,
};

describe("listRepos", () => {
  it("returns the bindings and the pull-request capability", async () => {
    const { client, calls } = fakeClient({
      repos: [BOUND],
      pullRequestsAvailable: true,
    });
    const list = await listRepos(client, null);
    expect(list.repos).toEqual([BOUND]);
    expect(list.pullRequestsAvailable).toBe(true);
    expect(calls[0]).toEqual({ method: "GET", path: "/api/v1/company/repos" });
  });

  it("carries a fingerprint and never a token", async () => {
    const { client } = fakeClient({ repos: [BOUND], pullRequestsAvailable: false });
    const list = await listRepos(client, null);
    const rendered = JSON.stringify(list);
    expect(rendered).toContain("tokenFingerprint");
    expect(rendered).not.toContain("github_pat_");
    // A field named `token` on a read shape would be a wire-level regression,
    // whatever it happened to hold on the day.
    expect(Object.keys(list.repos[0])).not.toContain("token");
  });

  it("treats a missing capability flag as absent rather than available", async () => {
    // An older host that predates the field must not have the console offering
    // a diff control it cannot serve. Anything but a literal `true` is false.
    const { client } = fakeClient({ repos: [] });
    expect((await listRepos(client, null)).pullRequestsAvailable).toBe(false);
  });

  it("raises when the host's repos field is not a list", async () => {
    const { client } = fakeClient({ repos: { "acme-widgets": BOUND } });
    await expect(listRepos(client, null)).rejects.toThrow(/list/i);
  });

  it("scopes the path to the addressed company on the platform form", async () => {
    const { client, calls } = fakeClient({ repos: [], pullRequestsAvailable: false });
    await listRepos(client, "acme");
    expect(calls[0].path).toBe("/api/v1/companies/acme/repos");
  });
});

describe("bindRepo", () => {
  it("posts the credential exactly once, to the bind route", async () => {
    const { client, calls } = fakeClient({ repo: BOUND, note: "ok" });
    await bindRepo(client, null, {
      url: "https://github.com/acme/widgets",
      token: "github_pat_SENTINEL",
      branches: ["main"],
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("POST");
    expect(calls[0].path).toBe("/api/v1/company/repos");
    expect(calls[0].body).toEqual({
      url: "https://github.com/acme/widgets",
      token: "github_pat_SENTINEL",
      branches: ["main"],
    });
  });
});

describe("revokeRepo", () => {
  it("names the binding in the path and escapes it", async () => {
    const { client, calls } = fakeClient({ note: "gone" });
    await revokeRepo(client, null, "acme/widgets");
    // The host's keys are `[a-z0-9-]`, so this can only arrive from a stale
    // client or a hand-built call — and a path segment assembled by
    // concatenation is exactly where that becomes a traversal.
    expect(calls[0].path).toBe("/api/v1/company/repos/acme%2Fwidgets/revoke");
  });
});
