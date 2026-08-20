// The MCP directory's Smithery credential (issue #1287).
//
// The whole point of these tests is that "the directory has nothing to show"
// has two causes and only one of them is the operator's to fix. Every
// assertion here is about keeping those two apart on screen.

import { describe, expect, it } from "vitest";

import { expectCatalogue } from "@/api/mcp-registry";
import type { CapabilityStatusDto } from "@/api/types";
import { directoryEmptyNotice } from "@/lib/mcp-registry";
import { mcpDirectoryStatus } from "@/views/UsageView";

describe("directoryEmptyNotice", () => {
  it("names the missing key when nothing is set", () => {
    const notice = directoryEmptyNotice("none");
    expect(notice).toContain("No Smithery key is set");
    // …and points at the fix, rather than leaving a dead end.
    expect(notice).toContain("Add the company's Smithery key");
  });

  it("reports a real miss when a key IS working", () => {
    for (const tier of ["company", "environment"] as const) {
      const notice = directoryEmptyNotice(tier);
      expect(notice).toContain("No hosted servers matched");
      expect(notice).not.toContain("No Smithery key is set");
    }
  });

  // The shared-host tier is a working directory. Scolding about it on top of
  // every empty result would train an operator to ignore the notice, and the
  // place to act on sharing is the credential card.
  it("does not nag about the shared host key on an empty result", () => {
    expect(directoryEmptyNotice("environment")).not.toContain("shared");
  });
});

describe("expectCatalogue tier narrowing", () => {
  const page = (directoryCredential: unknown) => ({
    servers: [],
    page: 1,
    totalPages: 0,
    directoryCredential,
  });

  it("keeps a recognised tier", () => {
    expect(expectCatalogue(page("company")).directoryCredential).toBe("company");
    expect(expectCatalogue(page("environment")).directoryCredential).toBe("environment");
  });

  // An unknown or missing tier degrades to `none`, which only ever offers MORE
  // explanation than the truth. Guessing `company` would tell an operator their
  // key is working when it may not be.
  it("degrades an unknown or missing tier to none", () => {
    expect(expectCatalogue(page("wat")).directoryCredential).toBe("none");
    expect(expectCatalogue(page(undefined)).directoryCredential).toBe("none");
    expect(expectCatalogue({ servers: [], page: 1, totalPages: 0 }).directoryCredential).toBe(
      "none",
    );
  });
});

describe("mcpDirectoryStatus", () => {
  const caps = (mcpDirectoryCredential?: "company" | "environment" | "none") =>
    ({ configured: true, mcpDirectoryCredential }) as CapabilityStatusDto;

  it("badges the company's own key as active", () => {
    expect(mcpDirectoryStatus(caps("company"))).toEqual({
      label: "Active",
      variant: "default",
    });
  });

  // Working, and shared. Neither "Active" nor "Awaiting credential" is true of
  // it, so it gets its own word rather than being rounded to a neighbour.
  it("gives the shared host key its own word", () => {
    const status = mcpDirectoryStatus(caps("environment"));
    expect(status.label).toBe("Shared host key");
    expect(status.label).not.toBe("Active");
    expect(status.variant).not.toBe("destructive");
  });

  it("badges a missing key as awaiting a credential", () => {
    expect(mcpDirectoryStatus(caps("none"))).toEqual({
      label: "Awaiting credential",
      variant: "destructive",
    });
  });

  // An absent field means the host could not determine the tier. Claiming
  // "Awaiting credential" there would send an admin to paste a key they may
  // already have — the #886 lie in the other direction.
  it("says unknown when the host could not determine the tier", () => {
    expect(mcpDirectoryStatus(caps()).label).toBe("Unknown");
    expect(mcpDirectoryStatus(caps()).variant).not.toBe("destructive");
  });
});
