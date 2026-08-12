import { describe, expect, it } from "vitest";

import { DESTINATION_KINDS, destinationLabel } from "@/api/workflows";

// #813 defect 8: the collapsed destination Select rendered the raw `__none__`
// sentinel because base-ui prints the stored value when given no text. These pin
// the value→label mapping the collapsed control now uses.
describe("destinationLabel", () => {
  it("maps the __none__ sentinel to a human label, never the raw value", () => {
    expect(destinationLabel("__none__")).toBe("Nowhere (run result only)");
    expect(destinationLabel("__none__")).not.toContain("__none__");
  });

  it("treats an empty stored value as no destination", () => {
    expect(destinationLabel("")).toBe("Nowhere (run result only)");
  });

  it("maps every destination kind to its picker label", () => {
    for (const kind of DESTINATION_KINDS) {
      expect(destinationLabel(kind.value)).toBe(kind.label);
    }
  });

  it("falls back to the raw value for an unrecognized kind", () => {
    expect(destinationLabel("webhook")).toBe("webhook");
  });
});
