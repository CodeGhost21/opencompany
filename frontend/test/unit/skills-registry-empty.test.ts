import { describe, expect, it } from "vitest";

import { registryEmptyLabel } from "@/lib/skills";

/**
 * The registry tab's empty-state line (issue #1467).
 *
 * The bug this pins: a failed registry read left the list empty, so the tab
 * rendered a destructive "couldn't reach the registry" alert AND, directly
 * below it, "this host serves no shared skill registry" — a fact about the host
 * asserted from the very failure the alert reports. The two claims contradict.
 * A read error must win the label.
 */
describe("registryEmptyLabel", () => {
  it("says only that the read failed when there was an error", () => {
    const label = registryEmptyLabel(true, true);
    expect(label).toMatch(/couldn't reach the registry/i);
    // Must NOT assert a fact about the host derived from the same failure.
    expect(label).not.toMatch(/serves no/i);
  });

  it("an error wins even if some rows had loaded before it", () => {
    expect(registryEmptyLabel(true, false)).toMatch(/couldn't reach the registry/i);
  });

  it("claims 'no registry' only for a read that succeeded and came back empty", () => {
    expect(registryEmptyLabel(false, true)).toMatch(/serves no shared skill registry/i);
  });

  it("reads a non-empty registry filtered to nothing as a search miss", () => {
    expect(registryEmptyLabel(false, false)).toMatch(/no skills match/i);
  });
});
