import { describe, expect, it } from "vitest";

import { ApiError } from "@/api/types";
import { buildManifestToml, describeProvisionError } from "@/lib/company-manifest";

/**
 * The pure half of console company creation (issue #1807).
 *
 * `buildManifestToml` is what stands between an operator's typed name and a body
 * the host will accept, so its whole job is to be a *valid, minimal* manifest —
 * name always, the two optional sections only when the operator gave a value.
 * `describeProvisionError` is what turns a refused provision into a sentence,
 * and the one that matters is `company_exists`: the host names an id the
 * operator never typed, so the console has to re-word it.
 */
describe("buildManifestToml (issue #1807)", () => {
  it("builds a valid minimal manifest from a name alone", () => {
    const toml = buildManifestToml({ name: "Acme Robotics" });
    expect(toml).toBe('[company]\nname = "Acme Robotics"\n');
    // The host injects policy.mode and users.mode for an omitted section, so
    // neither belongs in the minimal body.
    expect(toml).not.toContain("[users]");
    expect(toml).not.toContain("[policy]");
  });

  it("escapes quotes and backslashes in the name so the TOML stays valid", () => {
    const toml = buildManifestToml({ name: 'A "quoted" C:\\orp' });
    expect(toml).toContain('name = "A \\"quoted\\" C:\\\\orp"');
  });

  it("writes [users].admins only when an admin email is given", () => {
    expect(buildManifestToml({ name: "Acme" })).not.toContain("admins");

    const withAdmin = buildManifestToml({ name: "Acme", adminEmail: "ceo@acme.test" });
    expect(withAdmin).toContain("[users]");
    expect(withAdmin).toContain('admins = ["ceo@acme.test"]');
  });

  it("ignores a blank admin email rather than emitting an empty admins list", () => {
    const toml = buildManifestToml({ name: "Acme", adminEmail: "   " });
    expect(toml).not.toContain("[users]");
    expect(toml).not.toContain("admins");
  });

  it("writes [policy].mode only when a tier was chosen", () => {
    expect(buildManifestToml({ name: "Acme" })).not.toContain("[policy]");

    const supervised = buildManifestToml({ name: "Acme", policyMode: "supervised" });
    expect(supervised).toContain("[policy]");
    expect(supervised).toContain('mode = "supervised"');
  });
});

describe("describeProvisionError (issue #1807)", () => {
  it("re-words company_exists in the operator's terms (they typed a name, not an id)", () => {
    const msg = describeProvisionError(
      new ApiError(409, "company_exists", "company already exists: acme", true),
    );
    expect(msg).toContain("already exists");
    expect(msg).toContain("different name");
  });

  it("shows the host's quota message verbatim", () => {
    const host = "tenant company quota of 5 reached";
    expect(
      describeProvisionError(new ApiError(429, "quota_exceeded", host, true)),
    ).toBe(host);
  });

  it("explains a platform-scope refusal in terms of the sign-in", () => {
    const msg = describeProvisionError(new ApiError(401, "unauthorized", "unauthorized", true));
    expect(msg).toContain("platform credential");
  });

  it("falls back to a generic line for a non-ApiError", () => {
    expect(describeProvisionError(new Error("boom"))).toContain("Something went wrong");
  });
});
