/**
 * Issue #1192 — the publishing row on the Usage view.
 *
 * Publishing was the one capability on this card with no field at all: whether
 * an agent could hand its work over as a deliverable was answerable only by
 * reading the manifest and knowing that `publish_artifact` rides the
 * `files`/`docs` grant. Every other capability there carries a Granted/InBuild
 * shape.
 *
 * Two things are pinned here rather than left to review. The `undefined` rung
 * takes `composioStatus`'s stricter shape — an unanswered host is unknown, never
 * a confident "Not granted" — because the sibling `mediaStatus` collapses those
 * two into one branch and copying it would re-create #886 on a new row. And
 * there is deliberately no credential rung: publishing has no credential and no
 * store toggle, so `granted && inBuild` is the whole verdict.
 */
import { describe, expect, it } from "vitest";

import type { CapabilityStatusDto } from "@/api/types";
import { publishStatus } from "@/views/UsageView";

/** An in-build, granted company — the shape the later rungs are reached from. */
function granted(over: Partial<CapabilityStatusDto> = {}): CapabilityStatusDto {
  return {
    configured: false,
    publishInBuild: true,
    publishGranted: true,
    ...over,
  };
}

describe("publishStatus", () => {
  it("reports a build without the harness before anything else", () => {
    expect(publishStatus(granted({ publishInBuild: false }))).toEqual({
      label: "Not in this build",
      variant: "outline",
    });
  });

  /**
   * The #886-shaped guard. An older host that does not send `publishGranted` has
   * told us nothing; painting that as "Not granted" would state a fact about a
   * company's manifest on the strength of a missing field.
   */
  it("reports an unanswered host as unknown, never as `Not granted`", () => {
    const status = publishStatus(granted({ publishGranted: undefined }));
    expect(status.label).toBe("Couldn't check");
    expect(status.label).not.toBe("Not granted");
    expect(status.variant).not.toBe("destructive");
  });

  it("reports a company with no files/docs grant as ungranted", () => {
    expect(publishStatus(granted({ publishGranted: false }))).toEqual({
      label: "Not granted",
      variant: "secondary",
    });
  });

  it("reports a granted, in-build company as active", () => {
    expect(publishStatus(granted())).toEqual({ label: "Active", variant: "default" });
  });

  /**
   * The absent third rung, asserted so it is not "restored" later. Media,
   * Composio and search each have a credential branch between Granted and
   * Active; publishing must not grow one, because the only flag it could key on
   * would be a hardcoded `true`. A granted company is Active with nothing else
   * on the DTO.
   */
  it("needs no credential or store flag to reach Active", () => {
    expect(publishStatus({ configured: true, publishInBuild: true, publishGranted: true })).toEqual(
      { label: "Active", variant: "default" },
    );
  });

  /**
   * The not-in-build rung outranks the grant, in both directions: a build
   * without the harness wires nothing whatever the manifest says.
   */
  it("lets the build flag outrank the grant", () => {
    expect(publishStatus(granted({ publishInBuild: false, publishGranted: false })).label).toBe(
      "Not in this build",
    );
    expect(publishStatus(granted({ publishInBuild: false, publishGranted: undefined })).label).toBe(
      "Not in this build",
    );
  });
});
