import { describe, expect, it } from "vitest";

import { stripEnginePrefixes } from "@/views/workflows/run-error-message";

describe("stripEnginePrefixes", () => {
  it("shows the actionable leaf of a nested harness error", () => {
    expect(
      stripEnginePrefixes(
        "harness error: capability error: agent: the writer agent has no model",
      ),
    ).toBe("the writer agent has no model");
  });

  it("also strips a workflow kind when it is the only engine prefix", () => {
    expect(stripEnginePrefixes("http_request: the remote service refused the request")).toBe(
      "the remote service refused the request",
    );
  });

  it("keeps an unfamiliar prefix verbatim", () => {
    const raw = "provider error: quota exhausted";
    expect(stripEnginePrefixes(raw)).toBe(raw);
  });

  it("keeps prefix-only diagnostics verbatim", () => {
    const raw = "harness error: capability error: ";
    expect(stripEnginePrefixes(raw)).toBe(raw);
  });
});
