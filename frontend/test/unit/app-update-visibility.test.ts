import { describe, expect, it } from "vitest";

import {
  type AppUpdatePhase,
  isActionable,
  shouldResurface,
  updateHeadline,
  updateSummary,
} from "@/lib/app-update";

/**
 * The desktop update banner is silent until it has something to ask.
 *
 * This is the whole UX contract of the feature and it is worth pinning here
 * rather than in a browser walk: the states that must show nothing outnumber
 * the ones that show something, and "nothing rendered" is exactly the assertion
 * a render test is worst at making — an empty screen looks the same whether the
 * rule held or the component failed to mount.
 *
 * The rule: checking for an update, finding one, and downloading it are all
 * work the application does without asking. A banner announcing any of them is
 * a banner about something nobody can act on, and the cost is not neutral — it
 * is what teaches people to dismiss the one that matters.
 */

const EVERY_PHASE: AppUpdatePhase[] = [
  "idle",
  "checking",
  "available",
  "downloading",
  "ready",
  "installing",
  "up-to-date",
  "error",
];

describe("when the update banner appears", () => {
  it("says nothing while the shell is checking, finding or downloading", () => {
    for (const phase of ["idle", "checking", "available", "downloading", "up-to-date"] as const) {
      expect(isActionable(phase), `${phase} must be silent`).toBe(false);
    }
  });

  it("appears only once there is a decision, or a failure, to show", () => {
    // Staged bytes and a restart to authorise; the install the operator then
    // started; and the failure of either. Nothing else.
    expect(EVERY_PHASE.filter(isActionable)).toEqual(["ready", "installing", "error"]);
  });

  it("gives every phase it shows a heading", () => {
    for (const phase of EVERY_PHASE.filter(isActionable)) {
      expect(updateHeadline(phase)).not.toBe("Update");
    }
  });

  it("names the version when the manifest carried one, and reads without it", () => {
    expect(updateSummary("0.2.0")).toContain("0.2.0");
    // A build whose manifest named no version still gets a sentence. The
    // alternative — interpolating `null` — is how "Version null is ready"
    // reaches somebody's screen.
    expect(updateSummary(null)).not.toContain("null");
    expect(updateSummary(null).length).toBeGreaterThan(0);
  });
});

describe("after the operator says Later", () => {
  it("comes back for the next release", () => {
    // Dismissed while staged, then a later check finds a newer build and
    // stages that one: a second decision, so a second banner.
    expect(shouldResurface("up-to-date", "ready", null, null)).toBe(true);
  });

  it("comes back for an install the operator started themselves", () => {
    expect(shouldResurface("idle", "installing", null, null)).toBe(true);
  });

  it("stays hidden while the flow is still in the state that was dismissed", () => {
    // Not a transition INTO an actionable phase — nothing new has happened.
    expect(shouldResurface("ready", "installing", null, null)).toBe(false);
    expect(shouldResurface("ready", "ready", null, null)).toBe(false);
  });

  it("stays hidden for a background failure that keeps repeating", () => {
    // THE reason this rule exists. The check runs every fifteen minutes, and a
    // release whose bundle 404s fails identically every time. Re-showing it
    // would put a banner on screen four times an hour saying the same thing.
    const offline = "the update could not be downloaded: connection refused";
    expect(shouldResurface("downloading", "error", offline, offline)).toBe(false);
  });

  it("comes back when the failure is a different one", () => {
    expect(
      shouldResurface("downloading", "error", "signature mismatch", "connection refused"),
    ).toBe(true);
  });

  it("comes back for a failure nothing was dismissed for", () => {
    expect(shouldResurface("downloading", "error", "connection refused", null)).toBe(true);
  });
});
