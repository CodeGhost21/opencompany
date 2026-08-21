import { describe, expect, it } from "vitest";

import { ApiError } from "@/api/types";
import { classifyLoadFailure } from "@/lib/section-load";

/**
 * How a connections-section fetch failure is read (issue #1470).
 *
 * The bug this pins: five sections treated ANY failure as "this host has no such
 * thing" and unmounted, so a transient 500 was indistinguishable from a feature
 * the host never had. Only a 404 is genuinely "not served here"; everything else
 * is "the host could not answer", which the section must show rather than vanish.
 */
function apiError(status: number): ApiError {
  return new ApiError(status, "err", `http ${status}`);
}

describe("classifyLoadFailure", () => {
  it("reads a 404 as the surface being genuinely absent", () => {
    expect(classifyLoadFailure(apiError(404))).toBe("unavailable");
  });

  it("reads any other status as the host failing to answer", () => {
    expect(classifyLoadFailure(apiError(500))).toBe("error");
    expect(classifyLoadFailure(apiError(401))).toBe("error");
    expect(classifyLoadFailure(apiError(503))).toBe("error");
  });

  it("reads a non-ApiError (offline, a thrown string) as an error, not absence", () => {
    // A dropped connection or a malformed body is unknown state, never a
    // confident "this host has no such feature".
    expect(classifyLoadFailure(new TypeError("fetch failed"))).toBe("error");
    expect(classifyLoadFailure("boom")).toBe("error");
    expect(classifyLoadFailure(undefined)).toBe("error");
  });
});
