import { describe, expect, it } from "vitest";
import { buildMemoryGraph } from "@/views/overview/kg/adapter";

describe("repro: buildMemoryGraph with legacy array shape", () => {
  it("throws when entries is undefined (as when GET /memory returns [] but code reads .items)", () => {
    // Simulate the Overview flow: value is [] (old shape), code reads .items
    const value: unknown = [];
    const items = (value as { items?: unknown }).items;
    expect(() => buildMemoryGraph(items as never)).toThrow();
  });
});
