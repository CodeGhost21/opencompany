import { describe, expect, it } from "vitest";

import { ApiError } from "@/api/types";
import {
  buildManifestToml,
  collidesWithArchived,
  describeProvisionError,
  explicitIdProblem,
} from "@/lib/company-manifest";

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

  /**
   * Codex review on #1828 (PR comment 3865689246): TOML's `basic-unescaped`
   * grammar excludes U+007F (DEL) — it falls outside both printable-ASCII
   * ranges (`%x23-5B` / `%x5D-7E`) the spec allows literal — but the old
   * condition only escaped code points below U+0020, so a name containing a
   * pasted DEL byte produced a manifest the host's TOML parser refuses. On a
   * reset that surfaces only after the old company is already archived.
   */
  it("escapes a DEL (U+007F) in the name so the host's TOML parser accepts it", () => {
    const toml = buildManifestToml({ name: "Acme\u007fRobotics" });
    expect(toml).toContain('name = "Acme\\u007fRobotics"');
    // eslint-disable-next-line no-control-regex -- asserting the raw byte is gone
    expect(toml).not.toMatch(/\u007f/);
  });

  it("leaves other C1-range and extended Unicode characters literal, unescaped", () => {
    // U+0080 is the first `non-ascii` code point TOML allows literal in a
    // basic string — confirms the fix didn't widen the escape past DEL.
    const toml = buildManifestToml({ name: "Acme\u0080Robotics" });
    expect(toml).toContain("name = \"Acme\u0080Robotics\"");
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

/**
 * Codex review on #1828 (PR comment 3862711330): a shared-single-DB host
 * namespaces every provisioned id with `<tenant>--`, invisibly to the
 * console. An archived company's `CompanyStatus.id` is therefore the
 * namespaced form (e.g. `tenant-a--acme`), and the dialog's earlier
 * archived-id guard only rejected an exact string match against that full
 * id — so a bare `acme` typed into Advanced sailed through the check and
 * was re-namespaced back to `tenant-a--acme` by the host, recreating the
 * exact collision the guard exists to prevent.
 */
describe("collidesWithArchived (issue #1807)", () => {
  it("catches an exact match against the archived id", () => {
    expect(collidesWithArchived("acme", "acme")).toBe(true);
  });

  it("catches the bare id under a tenant-namespaced archived id", () => {
    expect(collidesWithArchived("acme", "tenant-a--acme")).toBe(true);
  });

  it("does not flag a genuinely distinct id", () => {
    expect(collidesWithArchived("acme-mk2", "acme")).toBe(false);
    expect(collidesWithArchived("acme", "tenant-a--other")).toBe(false);
  });

  it("does not flag the tenant-namespaced form itself as if it were bare", () => {
    // Only the bare tail collides; the full namespaced string typed back in
    // is already caught by the exact-match branch above, not this one.
    expect(collidesWithArchived("tenant-a--acme", "tenant-a--acme")).toBe(true);
    expect(collidesWithArchived("tenant-b--acme", "tenant-a--acme")).toBe(false);
  });
});

/**
 * `explicitIdProblem` only checked an operator-typed id's length and the two
 * reserved dot-segments — anything else was accepted, even though `slug`
 * (`store/paths.rs`) does not pass every character through unmodified.
 * `Bundle::new` derives a company's on-disk directory from `slug(id)`, and
 * `FsCompanyStore::list` reconstructs a company's id FROM that directory
 * name on every subsequent read (never from anything stored inside the
 * bundle) — so an id `slug` would silently change, like `acme corp` →
 * `acme_corp`, provisions and works for the request that created it, then
 * comes back under the changed id after any restart (codex review on
 * #1828, PR comment 3875297936).
 */
describe("explicitIdProblem — slug-stability (issue #1828 comment 3875297936)", () => {
  it("accepts every character slug passes through unmodified", () => {
    expect(explicitIdProblem("acme-corp_2.mk2")).toBeNull();
    expect(explicitIdProblem("ACME123")).toBeNull();
  });

  it("rejects a space, which slug silently folds to _", () => {
    const problem = explicitIdProblem("acme corp");
    expect(problem).not.toBeNull();
    expect(problem).toContain("letters, numbers");
  });

  it("rejects a slash, which slug silently folds to _", () => {
    const problem = explicitIdProblem("acme/ops");
    expect(problem).not.toBeNull();
    expect(problem).toContain("letters, numbers");
  });

  it("still rejects the reserved dot-segments ahead of the charset check", () => {
    // "." and ".." are themselves entirely slug-safe characters, so they
    // need their own, more specific message rather than falling through to
    // the generic charset one.
    expect(explicitIdProblem(".")).toContain("reserved path segment");
    expect(explicitIdProblem("..")).toContain("reserved path segment");
  });

  it("still enforces the length bound ahead of the charset check", () => {
    const tooLong = "a".repeat(129);
    expect(explicitIdProblem(tooLong)).toContain("too long");
  });
});

/**
 * Codex review on #1828 (PR comment 3875745309): `slug` (`store/paths.rs`)
 * allowlists `.` and passes it through unmodified at any position, so
 * `slug("acme.") === "acme."` and the slug-stability check above cannot see
 * a trailing period as a problem. Windows Win32 path handling strips a
 * trailing period from a path component before the directory is ever
 * created — the same hazard already documented and defended against for
 * secret filenames (`percent_encode`, `store/paths.rs`) — so on a
 * Windows-backed host `acme.` is created on disk as `acme`, and `list`
 * reconstructs the id from that directory name on the next read: the
 * bundle is created under `acme.` and comes back as `acme` after a restart.
 */
describe("explicitIdProblem — trailing period (issue #1828 comment 3875745309)", () => {
  it("rejects an id ending in a period, which Windows strips from the folder name", () => {
    const problem = explicitIdProblem("acme.");
    expect(problem).not.toBeNull();
    expect(problem).toContain("end with a period");
  });

  it("still rejects the reserved dot-segments ahead of the trailing-period check", () => {
    // "." and ".." both end in a period too, but they need their own,
    // more specific "reserved path segment" message.
    expect(explicitIdProblem(".")).toContain("reserved path segment");
    expect(explicitIdProblem("..")).toContain("reserved path segment");
  });

  it("accepts an interior period — only a trailing one is a Windows hazard", () => {
    expect(explicitIdProblem("acme.corp")).toBeNull();
  });
});
