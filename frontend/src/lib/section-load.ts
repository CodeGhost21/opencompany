// How a connections-section fetch failure should be read (issue #1470).
//
// Five sections on the old Connections page treated ANY fetch failure as "this host
// has no such thing" and unmounted themselves, so a transient 500 or a dropped
// session was indistinguishable from a feature the host genuinely does not have
// — the operator concluded the feature was missing and went looking for a
// rebuild. `CompanyCredentialCard` already draws the right distinction one
// directory over; this is that rule, extracted so every section routes through
// the same decision.

import { ApiError } from "@/api/types";

/**
 * Splits a genuine "not served here" from "the host could not answer".
 *
 * - `"unavailable"` — a 404: the host serves no such route, a fact about the
 *   build. The section may hide.
 * - `"error"` — anything else (5xx, offline, an expired session, a body that
 *   wasn't the shape the route promises): the current state is UNKNOWN, which is
 *   not the same as absent. The section must stay on the page and say so.
 */
export function classifyLoadFailure(err: unknown): "unavailable" | "error" {
  return err instanceof ApiError && err.status === 404 ? "unavailable" : "error";
}
