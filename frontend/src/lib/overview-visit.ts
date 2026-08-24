// The operator overview's last-read boundary (issue #1321).
//
// The host persists runs, but it does not yet persist an operator's read
// cursor for the company event log. This is therefore deliberately scoped to
// this browser and this connection/company pair. Keeping the boundary here,
// rather than pretending an SSE mount time is durable, lets the page say
// exactly what its "since" claim means.

import { type LocalScope, scopedKeyAdoptingLegacy } from "@/connections/types";

function keyFor(scope: LocalScope): string {
  return scopedKeyAdoptingLegacy("oc.overview.last-visit", scope, `oc.overview.last-visit.${scope.company ?? "__default__"}`);
}

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The previous time this browser opened this company's operator overview. */
export function readOverviewVisit(scope: LocalScope): number | null {
  try {
    const raw = storage()?.getItem(keyFor(scope));
    if (!raw) return null;
    const value = Number(raw);
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null;
  }
}

/** Record that this browser has opened this company's operator overview. */
export function writeOverviewVisit(scope: LocalScope, atMillis: number): void {
  try {
    storage()?.setItem(keyFor(scope), String(atMillis));
  } catch {
    // The page remains useful when storage is unavailable; it simply cannot
    // make a since-last-visit comparison after a reload.
  }
}
