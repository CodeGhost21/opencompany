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

/**
 * The boundary this *page load* has already settled on, per scope.
 *
 * A module-level map is the whole mechanism, and it is the point rather than an
 * optimisation: module state lives exactly as long as one document, which is
 * exactly the lifetime "since you last opened" is a claim about.
 *
 * ## What advances the boundary, and why it is a page load (issue #1700)
 *
 * The shell mounts this view conditionally, so every trip to Chat and back
 * unmounts and remounts it. Reading *and* writing the boundary on each mount
 * therefore re-anchored it to seconds ago: after one round trip the panel
 * compared against the previous *mount*, printed "No failed attempts were
 * recorded since the previous visit", and buried every failure recorded between
 * the operator's real last visit and now. A panel whose only job is an honest
 * comparison was reassuring people about a window they never asked for.
 *
 * A page load is the boundary an operator actually performs — opening the
 * console, reloading it, restoring the tab. A route change inside the console is
 * not "opening" it, and the heading above the panel does not claim it is.
 *
 * Two alternatives were considered and rejected:
 *
 * - **A time-based reset** ("advance if the previous visit was over N minutes
 *   ago") needs a clock nobody can see. Two visits either side of the threshold
 *   would report different windows for a reason nothing on screen explains,
 *   trading a wrong baseline for an unexplainable one.
 * - **Advancing on unload**, so the next load compares against when you *left*,
 *   is a different claim — "since you last closed" — and it silently swallows
 *   every failure recorded while the tab sat open on another view. The heading
 *   says "opened", and "opened" is also the more honest of the two.
 */
const openedThisLoad = new Map<string, number | null>();

/**
 * Open the overview for `scope`, returning the boundary to compare against.
 *
 * Idempotent for the rest of this page load: the first call takes the stored
 * boundary and replaces it with `atMillis`; every later call for the same scope
 * returns that same boundary and writes nothing. That idempotency is what makes
 * it safe to call from a render — React may double-invoke a render or discard
 * one, and neither can move the boundary.
 *
 * Read and write live in one call on purpose. Splitting them was what #1700
 * actually was: a lazy `useState` read and a `useEffect` write, each correct
 * alone, that between them advanced the boundary on every mount.
 */
export function openOverviewVisit(scope: LocalScope, atMillis: number = Date.now()): number | null {
  const key = keyFor(scope);
  const opened = openedThisLoad.get(key);
  if (opened !== undefined) return opened;
  const previous = readOverviewVisit(scope);
  writeOverviewVisit(scope, atMillis);
  openedThisLoad.set(key, previous);
  return previous;
}
