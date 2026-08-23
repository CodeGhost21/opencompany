// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import {
  absorbHubSetupHandoff,
  arrivedViaHubSetupHandoff,
  arrivedViaSetupHandoff,
  clearSetupHandoff,
  SETUP_HANDOFF_FRAGMENT,
} from "@/setup/state";

/**
 * The one-shot marker a setup hand-off link carries, and how the landing
 * mount consumes it.
 *
 * The marker is a hash-query flag (`#/company?from=setup`) rather than a state
 * write, because it has to survive a full-page navigation: setup's sign-in
 * button sets `window.location.href`, so component state dies at the boundary.
 * What survives is the URL — and `useHashView`'s segment parsing strips the
 * query from the route, so the flag is visible to the shell without ever
 * reaching the router.
 */

const hash = () => window.location.hash;

describe("the setup hand-off marker", () => {
  it("is a fragment whose route the router ignores", () => {
    expect(SETUP_HANDOFF_FRAGMENT).toBe("#/company?from=setup");
    // The view segment is still `company` — the query is the marker, not the
    // route, so `#/company?from=setup` resolves to the same view as `#/company`.
    const [path] = SETUP_HANDOFF_FRAGMENT.replace(/^#/, "").split("?");
    expect(path).toBe("/company");
  });

  it("reads true only when the address carries the marker", () => {
    window.location.hash = "#/company";
    expect(arrivedViaSetupHandoff()).toBe(false);

    window.location.hash = SETUP_HANDOFF_FRAGMENT;
    expect(arrivedViaSetupHandoff()).toBe(true);

    // A different origin value for the same key is not this marker.
    window.location.hash = "#/company?from=elsewhere";
    expect(arrivedViaSetupHandoff()).toBe(false);
  });

  it("consumes the marker without touching the route or other keys", () => {
    window.location.hash = "#/company?from=setup&host=conn-a";
    clearSetupHandoff();
    expect(hash()).toBe("#/company?host=conn-a");
    expect(arrivedViaSetupHandoff()).toBe(false);
  });

  it("leaves an address without the marker alone", () => {
    window.location.hash = "#/overview";
    clearSetupHandoff();
    expect(hash()).toBe("#/overview");
  });
});
