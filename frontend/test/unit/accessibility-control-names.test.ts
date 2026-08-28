import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("operator control names (issue #1395)", () => {
  it("names the ledger search and status filter independently of their values", () => {
    const ledgers = read("views/LedgersView.tsx");

    expect(ledgers).toContain('aria-label="Search ledger entries"');
    expect(ledgers).toContain('aria-label="Filter by status"');
  });

  it("names the memory search and type filter independently of their placeholders", () => {
    const memory = read("views/MemoryView.tsx");

    expect(memory).toContain('aria-label="Search memory"');
    expect(memory).toContain('aria-label="Filter by memory type"');
  });

  it("names the usage range filter", () => {
    expect(read("views/UsageView.tsx")).toContain('aria-label="Usage date range"');
  });

  it("keeps the composer named after its placeholder disappears", () => {
    const composer = read("views/chat/MessageComposer.tsx");

    expect(composer).toContain("aria-label={placeholder}");
  });
});
