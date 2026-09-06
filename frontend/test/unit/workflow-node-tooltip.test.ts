import { describe, expect, it } from "vitest";

import { runStateTitle } from "@/components/workflow-node";

/**
 * Coderabbit review on #1990: `declined` fell through to the generic
 * "This node finished." tooltip, which reads as an ordinary completion for a
 * step the sufficiency judge decided was not needed at all.
 */
describe("runStateTitle", () => {
  it("names a benign halt instead of claiming an ordinary finish", () => {
    expect(runStateTitle("declined", false)).toBe("This node was not needed.");
  });

  it("still reports a running node as executing", () => {
    expect(runStateTitle("running", false)).toBe("This node is executing now.");
  });

  it("still reports a failed node as failed", () => {
    expect(runStateTitle("error", false)).toBe("This node failed.");
  });

  it("still reports an undelivered report on an otherwise-finished node", () => {
    expect(runStateTitle("ok", true)).toBe(
      "This step ran. Its report did not go out.",
    );
  });

  it("still falls back to the plain finish message for an ordinary ok", () => {
    expect(runStateTitle("ok", false)).toBe("This node finished.");
  });
});
