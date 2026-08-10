import { describe, expect, it } from "vitest";

import { importSummary, migrationBannerText } from "@/views/WorkspaceView";

/**
 * The migration banner's sentence (issue #507).
 *
 * The banner counted the flat scratchpad list and called every entry a "note",
 * exactly as the receipt did before #500 — so a folder-only scratchpad was
 * offered as "1 note … is not in the company workspace yet" when it held no
 * notes at all. The two surfaces describe the same nodes one click apart, so
 * the cases below check both that each sentence is true on its own and that
 * the pair agrees.
 */

describe("migrationBannerText", () => {
  it("names a files-only scratchpad, unchanged from the wording that was right", () => {
    expect(migrationBannerText(3, 0)).toBe(
      "3 notes from this browser's old scratchpad are not in the company workspace yet.",
    );
  });

  it("names a folders-only scratchpad without inventing notes", () => {
    // The reported defect: this said "1 note … is" for zero notes.
    expect(migrationBannerText(0, 1)).toBe(
      "1 folder from this browser's old scratchpad is not in the company workspace yet.",
    );
    expect(migrationBannerText(0, 1)).not.toContain("note");
  });

  it("names both categories for a mixed scratchpad", () => {
    // The issue's own example: 1 folder + 2 files was offered as "3 notes".
    expect(migrationBannerText(2, 1)).toBe(
      "2 notes and 1 folder from this browser's old scratchpad are not in the company workspace yet.",
    );
  });

  it("agrees with the singular for a lone note", () => {
    expect(migrationBannerText(1, 0)).toBe(
      "1 note from this browser's old scratchpad is not in the company workspace yet.",
    );
  });

  it("takes the plural verb when each kind is singular but there are two nodes", () => {
    // The trap in keying the verb off the summary's leading number: it reads
    // "1", but "1 note and 1 folder" is two things.
    expect(migrationBannerText(1, 1)).toContain("are not in the company workspace yet");
    expect(migrationBannerText(1, 1)).not.toContain(" is not in");
  });

  it("takes the singular verb only for a genuinely single node", () => {
    expect(migrationBannerText(1, 0)).toContain(" is not in");
    expect(migrationBannerText(0, 1)).toContain(" is not in");
    expect(migrationBannerText(2, 0)).toContain(" are not in");
    expect(migrationBannerText(0, 2)).toContain(" are not in");
  });

  it("describes the same nodes the receipt will report", () => {
    // The banner and the toast are one click apart on one scratchpad. Before
    // #507 they disagreed: "3 notes" offered, "2 notes and 1 folder" imported.
    for (const [files, folders] of [
      [3, 0],
      [0, 1],
      [2, 1],
      [1, 1],
      [0, 4],
    ] as const) {
      expect(migrationBannerText(files, folders)).toContain(importSummary(files, folders));
    }
  });
});
