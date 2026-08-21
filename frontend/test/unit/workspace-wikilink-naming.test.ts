/**
 * A wiki link resolves against the workspace naming rule, not the literal name.
 *
 * Every name the runtime mints is lowercase and dashed
 * (`docs/spec/runtime/workspace-names.md`), while the prose that links to a note
 * reasonably says `[[Close checklist]]`. Matching literally would have
 * unresolved every existing link in every seeded company the moment the rule
 * landed — the link renders as a dead one, and the host reports no backlink for
 * a note that plainly has one.
 *
 * The host half of the same rule lives in `src/company/workspace_links.rs`; the
 * two must agree, which is why this pins the console side.
 */
import { describe, expect, it } from "vitest";

import type { FsNode } from "@/api/workspace";
import { fileByTitle } from "@/lib/workspace";

function file(id: string, name: string): FsNode {
  return { id, name, kind: "file", parentId: null, updatedAt: 0 } as FsNode;
}

describe("fileByTitle", () => {
  const NOTES: FsNode[] = [
    file("close", "close-checklist.md"),
    file("voice", "channel-voice.md"),
  ];

  it("resolves a link written the way a person says it", () => {
    expect(fileByTitle(NOTES, "Close checklist")?.id).toBe("close");
    expect(fileByTitle(NOTES, "channel voice")?.id).toBe("voice");
    expect(fileByTitle(NOTES, "Channel_Voice")?.id).toBe("voice");
  });

  it("still resolves the stored name verbatim", () => {
    expect(fileByTitle(NOTES, "close-checklist")?.id).toBe("close");
  });

  it("prefers an exact title when a tree carries both spellings", () => {
    const mixed = [...NOTES, file("legacy", "Close checklist.md")];
    expect(fileByTitle(mixed, "Close checklist")?.id).toBe("legacy");
    expect(fileByTitle(mixed, "close-checklist")?.id).toBe("close");
  });

  it("does not resolve a link that normalizes to nothing", () => {
    expect(fileByTitle(NOTES, "🎉")).toBeUndefined();
    expect(fileByTitle(NOTES, "   ")).toBeUndefined();
  });
});
