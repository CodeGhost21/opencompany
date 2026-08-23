// The Brain page's engine and drop logic — pure functions, tested over the
// whole domain because they carry facts the pod log used to keep to itself:
// which engine is bound, whether an operator may change it, and — the loudest
// case — whether the bound engine is the null one, silently discarding every
// write.
//
// Replaces the `/spec`-shaped panel tests (issue #914). The panel became a
// picker when the engine stopped being boot-only, and the state it renders now
// comes from `GET …/memory/engine` rather than the unauthenticated handshake:
// an operator can change the binding live, and the engine endpoint refreshes
// the provider's health for every read.
import { describe, expect, it } from "vitest";

import { documentSlug, type MemoryEngineState, type MemoryStats } from "@/api/memory";
import { teammateMemoryCount } from "@/views/MemoryView";

function engine(overrides: Partial<MemoryEngineState>): MemoryEngineState {
  return {
    active: "store",
    capabilities: [],
    selected: "store",
    apiKeySet: false,
    layer: "default",
    editable: true,
    configPath: "/data/config.toml",
    options: [],
    ...overrides,
  };
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

// The one engine state the *writing* half of the page must respect: the null
// engine takes every write and throws it away, so the page disables both the
// "New memory" button and the drop zone against it.
describe("the discarding engine", () => {
  it("is the null engine, whatever the saved selection says", () => {
    expect(engine({ active: "null", selected: "mem0" }).active === "null").toBe(true);
  });

  it("is not any engine that actually retains", () => {
    for (const active of ["store", "embedded", "namespace", "supermemory", "mem0", "cognee"]) {
      expect(engine({ active }).active === "null").toBe(false);
    }
  });
});

// The refusal the console must render rather than work around: a deployment
// that injects `OPENCOMPANY_MEMORY` owns the choice, and a picker that
// accepted an edit there would write a file that changes nothing.
describe("who owns the engine choice", () => {
  it("is the environment on a hosted tenant, and the console says so", () => {
    const state = engine({ layer: "env", editable: false });
    expect(state.editable).toBe(false);
  });

  it("is the console on a self-hosted host with nothing injected", () => {
    expect(engine({ layer: "default", editable: true }).editable).toBe(true);
    expect(engine({ layer: "config.toml", editable: true }).editable).toBe(true);
  });
});

// `documentSlug` is a copy of the host's `ingest::label_for`, and the forget
// route addresses documents by that slug — so a drift here is a delete that
// 404s on exactly the documents whose names needed slugging.
describe("documentSlug", () => {
  it("matches the host's slug for a folder-dropped path", () => {
    expect(documentSlug("Contracts/2026/acme msa.pdf")).toBe("contracts-2026-acme-msa.pdf");
  });

  it("keeps a plain file name as it is, lower-cased", () => {
    expect(documentSlug("Handbook.md")).toBe("handbook.md");
  });

  it("never yields an empty slug for a name that is all separators", () => {
    expect(documentSlug("///")).toBe("document");
  });

  it("keeps the identifying tail of a very long path", () => {
    const slug = documentSlug(`${"a/".repeat(80)}report.pdf`);
    expect(Array.from(slug).length).toBe(96);
    expect(slug.endsWith("report.pdf")).toBe(true);
  });
});

// Issue #1402. `/memory/stats` hands back a superset (`agentChunks`, every
// context chunk) and one of its own slices (`taskOutcomes`, the ones carrying
// the outcome label prefix). The health strip renders them as neighbouring
// tiles, so the tile has to subtract or it counts the same rows twice.
describe("teammateMemoryCount", () => {
  it("is zero when every chunk is a task outcome — the shape that shipped the bug", () => {
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
