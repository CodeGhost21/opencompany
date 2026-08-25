// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LocalScope } from "@/connections/types";

const ACME: LocalScope = { connection: "local-a", company: "acme" };
const OTHER_CONNECTION: LocalScope = { connection: "local-b", company: "acme" };
const OTHER_COMPANY: LocalScope = { connection: "local-a", company: "globex" };

/**
 * A fresh module registry is a fresh page load.
 *
 * The since-visit boundary settles once per page load (issue #1700) and the
 * mechanism holding it is module state, so "reload the browser" and "reset the
 * module registry" are the same event. Every test that needs a second visit
 * asks for a second load here rather than reaching into the module.
 */
async function load() {
  vi.resetModules();
  return await import("@/lib/overview-visit");
}

describe("the operator overview visit boundary (#1321)", () => {
  beforeEach(() => window.localStorage.clear());

  it("survives a reload for the same connection and company", async () => {
    const { readOverviewVisit, writeOverviewVisit } = await load();
    writeOverviewVisit(ACME, 1_700_000_000_000);

    expect(readOverviewVisit(ACME)).toBe(1_700_000_000_000);
  });

  it("does not share a browser-local boundary between connections", async () => {
    const { readOverviewVisit, writeOverviewVisit } = await load();
    writeOverviewVisit(ACME, 1_700_000_000_000);

    expect(readOverviewVisit(OTHER_CONNECTION)).toBeNull();
  });

  it("ignores malformed stored values rather than inventing a boundary", async () => {
    const { readOverviewVisit } = await load();
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "yesterday");

    expect(readOverviewVisit(ACME)).toBeNull();
  });
});

describe("opening the overview settles the boundary for one page load (#1700)", () => {
  beforeEach(() => window.localStorage.clear());

  it("hands back the previous open and records this one", async () => {
    const { openOverviewVisit, readOverviewVisit } = await load();
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "1700000000000");

    expect(openOverviewVisit(ACME, 1_700_000_500_000)).toBe(1_700_000_000_000);
    expect(readOverviewVisit(ACME)).toBe(1_700_000_500_000);
  });

  it("keeps answering with the same boundary for the rest of the load", async () => {
    // This is the whole of #1700. Every navigation back to the overview opens
    // it again; when each open advanced the boundary, the panel compared
    // against a moment ago and reported that nothing had failed since.
    const { openOverviewVisit, readOverviewVisit } = await load();
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "1700000000000");

    openOverviewVisit(ACME, 1_700_000_500_000);

    expect(openOverviewVisit(ACME, 1_700_000_900_000)).toBe(1_700_000_000_000);
    expect(openOverviewVisit(ACME, 1_700_001_300_000)).toBe(1_700_000_000_000);
    // …and the recorded visit stays where the first open put it, so the *next*
    // page load compares against when this one began.
    expect(readOverviewVisit(ACME)).toBe(1_700_000_500_000);
  });

  it("advances on the next page load, which is the event the heading names", async () => {
    const first = await load();
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "1700000000000");
    first.openOverviewVisit(ACME, 1_700_000_500_000);

    const second = await load();

    expect(second.openOverviewVisit(ACME, 1_700_000_900_000)).toBe(1_700_000_500_000);
    expect(second.readOverviewVisit(ACME)).toBe(1_700_000_900_000);
  });

  it("settles each scope separately, so a company switch gets its own boundary", async () => {
    const { openOverviewVisit } = await load();
    window.localStorage.setItem("oc.overview.last-visit:local-a::acme", "1700000000000");
    window.localStorage.setItem("oc.overview.last-visit:local-a::globex", "1600000000000");

    expect(openOverviewVisit(ACME, 1_700_000_500_000)).toBe(1_700_000_000_000);
    expect(openOverviewVisit(OTHER_COMPANY, 1_700_000_500_000)).toBe(1_600_000_000_000);
    // Switching back does not re-open the first: within one load each scope is
    // opened exactly once.
    expect(openOverviewVisit(ACME, 1_700_000_900_000)).toBe(1_700_000_000_000);
  });

  it("reports no earlier visit the first time a browser opens a company", async () => {
    const { openOverviewVisit, readOverviewVisit } = await load();

    expect(openOverviewVisit(ACME, 1_700_000_500_000)).toBeNull();
    // A settled `null` must still count as settled rather than as never opened.
    expect(openOverviewVisit(ACME, 1_700_000_900_000)).toBeNull();
    expect(readOverviewVisit(ACME)).toBe(1_700_000_500_000);
  });
});
