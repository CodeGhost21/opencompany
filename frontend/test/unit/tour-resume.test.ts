// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import { TOUR } from "@/tour/steps";
import { scopedKey } from "@/connections/types";
import {
  armTourResume,
  clearTourResume,
  markTourResume,
  readTourResume,
  setActiveTourStop,
} from "@/tour/state";

/**
 * The onboarding tour's resume marker (issue #300), and the coupling that
 * actually matters about it.
 *
 * A marker records a stop by its **view**, and the controller later finds the
 * stop again with `TOUR.findIndex((s) => s.view === resumed)`. Those are two
 * halves of one contract held together by a bare string, and when they drift
 * the failure is silent by construction: `findIndex` answers `-1`, the
 * controller's own comment says a retired stop must be dropped, so it drops it
 * and shows nothing. An operator returning from an OAuth redirect simply does
 * not get their tour back, and no error is raised anywhere.
 *
 * That drift has already happened once. Connections became a page of the
 * Settings section — the stop is `{ view: "settings", sub: "connections" }` —
 * and `oauth-onboarding-resume.spec.ts` went on seeding the retired
 * `"connections"`, so the Playwright spec was red for a reason that had nothing
 * to do with the product. Because nothing ran that suite, the red went
 * unreported and was later filed as a product bug (#428) that does not exist.
 *
 * So the assertion that earns its place here is not "a marker round-trips". It
 * is "**every** view a stop can publish is a view the lookup can find" — the
 * mechanism is *reached*, not merely functional.
 */

const COMPANY = "acme";
/** One connection's view of `COMPANY`. */
const SCOPE = { connection: "conn-a", company: COMPANY };
/** A *different host* that happens to serve a company of the same name. */
const SAME_NAME_ELSEWHERE = { connection: "conn-b", company: COMPANY };

beforeEach(() => {
  window.localStorage.clear();
  setActiveTourStop(null);
});

/** The controller's own lookup, so a drift fails here rather than in a browser. */
function stopIndexFor(view: string | null): number {
  return view === null ? -1 : TOUR.findIndex((s) => s.view === view);
}

describe("the stop-view ⇄ resume-marker contract", () => {
  /** The one stop that hands the browser to a third party, so the only one that arms. */
  const connections = TOUR.find((s) => s.sub === "apps");

  it("still has an accounts stop at all", () => {
    expect(connections, "the tour should still have an Apps stop").toBeDefined();
  });

  it("pins the view the accounts stop publishes", () => {
    // NOT a tautology, and the distinction is the whole reason this test is
    // here. Reading `stop.view` and then looking it up in `TOUR` would pass by
    // construction and prove nothing — the vacuous shape that let the e2e spec
    // rot unnoticed in the first place. This pins the *literal*, because a
    // literal is exactly what drifted: `oauth-onboarding-resume.spec.ts` seeds
    // this string into localStorage by hand, and cannot see it change.
    //
    // If this fails, the tour was reorganised: update the e2e spec's seed in
    // the same commit.
    //
    // It has now failed once and been updated exactly that way. The accounts
    // page left the Settings rail for the Connections section, so the stop is
    // `{ view: "connections", sub: "apps" }` and the marker it publishes is
    // `"connections"` — which is what `oauth-onboarding-resume.spec.ts` seeds.
    // The literal below and that seed are the coupling; this test is the fast
    // half of it.
    expect(connections!.view).toBe("connections");
  });

  it("leaves the lookup unambiguous, so a resume lands on the stop that armed it", () => {
    // The controller resolves a marker with `findIndex` — FIRST match wins.
    // Views are not unique across the tour (there are two `overview` stops and
    // two `chat` stops), so this is only safe while the arming stop is the sole
    // holder of its view. Add a second Connections stop ahead of this one and
    // the operator silently resumes on the wrong one; nothing else would catch
    // that, because the tour still runs and still shows *a* stop.
    const sharingItsView = TOUR.filter((s) => s.view === connections!.view);
    expect(sharingItsView).toHaveLength(1);
    expect(stopIndexFor(connections!.view)).toBe(TOUR.indexOf(connections!));
  });

  it("round-trips an armed marker through storage unchanged", () => {
    setActiveTourStop(connections!.view);
    armTourResume(SCOPE);
    expect(readTourResume(SCOPE)).toBe(connections!.view);
  });
});

describe("armTourResume", () => {
  it("writes nothing when no tour is running", () => {
    // What lets ConnectionsView arm unconditionally without first asking
    // whether it is inside onboarding.
    armTourResume(SCOPE);
    expect(readTourResume(SCOPE)).toBeNull();
  });

  it("keeps each company's marker separate", () => {
    setActiveTourStop("settings");
    armTourResume(SCOPE);
    expect(readTourResume({ connection: "conn-a", company: "other-co" })).toBeNull();
    expect(readTourResume(SCOPE)).toBe("settings");
  });

  it("keeps two hosts serving the same company name apart", () => {
    // The buzz regression, in one assertion. Keyed on company alone — which is
    // what this was before connections existed, and what block/buzz still does
    // — these two are the same key, and one operator's tour progress silently
    // becomes the other's.
    setActiveTourStop("settings");
    armTourResume(SCOPE);
    expect(readTourResume(SAME_NAME_ELSEWHERE)).toBeNull();
    expect(readTourResume(SCOPE)).toBe("settings");
  });
});

describe("readTourResume", () => {
  it("treats a marker past its TTL as absent", () => {
    markTourResume(SCOPE, "settings");
    const key = scopedKey("oc-tour", SCOPE);
    const stored = JSON.parse(window.localStorage.getItem(key)!);
    stored.pendingResume.at = Date.now() - 16 * 60 * 1000;
    window.localStorage.setItem(key, JSON.stringify(stored));

    expect(readTourResume(SCOPE)).toBeNull();
  });

  it("honours a marker inside the TTL", () => {
    // The other half of the boundary — without this, a TTL of zero would pass
    // the test above and break the feature.
    markTourResume(SCOPE, "settings");
    expect(readTourResume(SCOPE)).toBe("settings");
  });

  it("is null once the marker is consumed, so it cannot fire on a later visit", () => {
    markTourResume(SCOPE, "settings");
    clearTourResume(SCOPE);
    expect(readTourResume(SCOPE)).toBeNull();
  });

  it("keeps the completed/skipped flags when the marker is cleared", () => {
    window.localStorage.setItem(scopedKey("oc-tour", SCOPE), JSON.stringify({ skipped: true }));
    markTourResume(SCOPE, "settings");
    clearTourResume(SCOPE);

    const stored = JSON.parse(window.localStorage.getItem(scopedKey("oc-tour", SCOPE))!);
    expect(stored.skipped).toBe(true);
    expect(stored.pendingResume).toBeUndefined();
  });
});
