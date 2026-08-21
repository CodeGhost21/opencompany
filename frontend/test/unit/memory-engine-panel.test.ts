// The engine panel's decision logic (issue #914 console surfacing) — pure
// functions, tested over the whole domain because the panel is the operator's
// only view of a fact the pod log used to keep to itself: which engine is
// bound, whether the boot probe reached it, and — the loudest case — whether
// it is the null engine silently discarding every write.
import { describe, expect, it } from "vitest";

import type { MemoryStats } from "@/api/memory";
import type { MemorySpec } from "@/api/types";
import {
  enginePanelMode,
  isMandatoryOnly,
  MANDATORY_FAMILIES,
  probeLabel,
  teammateMemoryCount,
} from "@/views/MemoryView";

function spec(overrides: Partial<MemorySpec>): MemorySpec {
  return { backend: "remote", driver_id: "supermemory", capabilities: [], ...overrides };
}

function stats(overrides: Partial<MemoryStats>): MemoryStats {
  return {
    facts: 0,
    factsUpdatedAtMillis: 0,
    lastUpdatedAtMillis: 0,
    agentChunks: 0,
    taskOutcomes: 0,
    ...overrides,
  };
}

describe("enginePanelMode", () => {
  it("hides on an old host that never sent the field", () => {
    expect(enginePanelMode(undefined)).toBe("hidden");
  });

  it("hides on the base store backend — memory served by the host's own stores", () => {
    expect(enginePanelMode(spec({ backend: "store", driver_id: "" }))).toBe("hidden");
  });

  it("warns on the null engine: bound and probed, but discarding every write", () => {
    expect(enginePanelMode(spec({ backend: "null", driver_id: "null" }))).toBe("discard");
  });

  it("renders embedded and remote engines normally", () => {
    expect(enginePanelMode(spec({ backend: "embedded", driver_id: "namespace" }))).toBe("normal");
    expect(enginePanelMode(spec({ backend: "remote" }))).toBe("normal");
  });
});

describe("probeLabel", () => {
  it("maps the three states an operator must tell apart", () => {
    expect(probeLabel(true)).toBe("reachable at boot");
    expect(probeLabel(false)).toContain("unreachable at boot");
    expect(probeLabel(undefined)).toBe("not probed");
  });

  it("is total: a literal null (absent-on-the-wire) reads as not probed, never blank", () => {
    expect(probeLabel(null)).toBe("not probed");
  });
});

describe("isMandatoryOnly", () => {
  it("matches exactly the mandatory floor, in any order", () => {
    expect(isMandatoryOnly([...MANDATORY_FAMILIES])).toBe(true);
    expect(isMandatoryOnly([...MANDATORY_FAMILIES].reverse())).toBe(true);
  });

  it("rejects subsets, supersets, and the empty (not-negotiated) case", () => {
    expect(isMandatoryOnly(["core", "recall"])).toBe(false);
    expect(isMandatoryOnly([...MANDATORY_FAMILIES, "graph"])).toBe(false);
    expect(isMandatoryOnly([])).toBe(false);
  });

  it("rejects a same-length set that swaps a family — length alone is not the test", () => {
    expect(isMandatoryOnly(["core", "recall", "graph"])).toBe(false);
  });
});

// Issue #1402. `/memory/stats` hands back a superset (`agentChunks`, every
// context chunk) and one of its own slices (`taskOutcomes`, the ones carrying
// the outcome label prefix). The health strip renders them as neighbouring
// tiles, so the tile has to subtract or it counts the same rows twice.
describe("teammateMemoryCount", () => {
  it("is zero when every chunk is a task outcome — the shape that shipped the bug", () => {
    // The seeded company: 13 chunks, all of them outcomes. The strip read
    // "Teammate memory 13" beside "Task outcomes 13" and "Total items 13".
    expect(teammateMemoryCount(stats({ agentChunks: 13, taskOutcomes: 13 }))).toBe(0);
  });

  it("counts only the chunks left over once outcomes are carved out", () => {
    expect(teammateMemoryCount(stats({ agentChunks: 13, taskOutcomes: 5 }))).toBe(8);
  });

  it("reads zero before the first stats response, not NaN", () => {
    expect(teammateMemoryCount(null)).toBe(0);
  });

  it("never goes negative when two live reads cross under a concurrent write", () => {
    expect(teammateMemoryCount(stats({ agentChunks: 2, taskOutcomes: 7 }))).toBe(0);
  });
});
