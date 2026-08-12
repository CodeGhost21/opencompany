import { describe, expect, it } from "vitest";

import type { Repo } from "@/api/repos";
import { approvalAction } from "@/lib/language";
import { repoCoverage } from "@/lib/repos";
import type { ApprovalSummary } from "@/api/types";

/**
 * The console half of issue #245's agent tier.
 *
 * Two derivations, both of which fail *quietly* when they are wrong — which is
 * the whole reason they are unit-tested rather than left to the browser suite.
 *
 * 1. **The approval labels.** `repo_checkout` and `repo_pr` park per call, so
 *    each one reaches an operator on an approval card and again in the Standing
 *    permissions list. An unlabelled tool falls through to "Use one of its
 *    tools", and two permissions that both read that way are indistinguishable
 *    — the exact failure #372 and #374 were filed about.
 * 2. **The coverage notice.** Binding a repository and granting `repo` happen
 *    on two different surfaces, so three of the four combinations look
 *    identical on screen: nothing happens. Picking the wrong branch tells an
 *    operator their setup is fine when no agent can read a line of code.
 */

function approval(kind: string): ApprovalSummary {
  return {
    id: "a1",
    kind,
    amount_usd: null,
    at_millis: 1_700_000_000_000,
    agent: "ceo",
  };
}

const BOUND: Repo = {
  key: "acme-widgets-000000000000",
  url: "https://github.com/acme/widgets",
  owner: "acme",
  repo: "widgets",
  branches: ["main"],
  tokenFingerprint: "0f1e2d3c4b5a",
  sizeBytes: 4096,
  boundAtMillis: 1_700_000_000_000,
};

describe("what a repository approval says", () => {
  it("names both tools in plain language rather than falling back", () => {
    expect(approvalAction(approval("repo_checkout"))).toBe(
      "Check out one of the company's repositories",
    );
    expect(approvalAction(approval("repo_pr"))).toBe(
      "Fetch a pull request from one of the company's repositories",
    );
  });

  it("distinguishes the two, which the generic fallback cannot", () => {
    // The regression this guards: an unlabelled tool reads "Use one of its
    // tools", so a checkout and a diff fetch would be one indistinguishable
    // row in the Standing permissions list.
    expect(approvalAction(approval("repo_checkout"))).not.toBe(
      approvalAction(approval("repo_pr")),
    );
    expect(approvalAction(approval("repo_checkout"))).not.toBe("Use one of its tools");
  });

  it("never shows the raw tool identifier", () => {
    for (const kind of ["repo_checkout", "repo_pr"]) {
      expect(approvalAction(approval(kind))).not.toContain("repo_");
    }
  });
});

describe("which half of the repository setup is missing", () => {
  it("says nothing when bound repositories have a reader", () => {
    expect(repoCoverage({ repos: [BOUND], grantedAgents: ["ceo"], repoGranted: true })).toEqual({
      notice: null,
      readableBy: ["ceo"],
    });
  });

  it("names the grant when repositories are bound and nobody holds `repo`", () => {
    const coverage = repoCoverage({ repos: [BOUND], grantedAgents: [], repoGranted: false });
    expect(coverage.notice).toBe("bound-ungranted");
    expect(coverage.readableBy).toEqual([]);
  });

  it("names the binding when the grant exists and nothing is bound", () => {
    expect(repoCoverage({ repos: [], grantedAgents: [], repoGranted: true }).notice).toBe(
      "granted-unbound",
    );
  });

  it("says nothing at all when neither half is configured", () => {
    // Not a misconfiguration — a company that does not use this feature must
    // not be nagged about it on every visit to the connections page.
    expect(repoCoverage({ repos: [], grantedAgents: [], repoGranted: false }).notice).toBeNull();
  });

  it("treats an unknown grant as unknown, not as absent", () => {
    // An older host, or a `/capabilities` read that failed. Telling an operator
    // "you have not granted this" on the strength of a failed request sends
    // them to edit a manifest that is already correct.
    expect(repoCoverage({ repos: [], grantedAgents: [], repoGranted: undefined }).notice).toBeNull();
  });

  it("still names the grant gap when the grant flag is unknown but nothing can read", () => {
    // The bound-but-unreadable state is decided by the roster answer, which the
    // host resolves through the same walk the harness builds agents with — so
    // it holds whether or not the manifest flag came back.
    expect(
      repoCoverage({ repos: [BOUND], grantedAgents: [], repoGranted: undefined }).notice,
    ).toBe("bound-ungranted");
  });
});
