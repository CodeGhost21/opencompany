import { describe, expect, it } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import { mergeReadFloors, unreadCount } from "@/lib/unread";

/**
 * How the host's read markers meet the console's own floor (issue #755).
 *
 * Unread used to be measured from a floor stamped when the tab mounted, so a
 * reload marked every channel read. The host now remembers a per-person floor,
 * but it arrives *asynchronously* — the console is usable while that request is
 * in flight, and a channel opened in that window already has a fresher floor
 * than anything stored.
 *
 * These import the shipped rules from `@/lib/unread`, the same module the shell
 * calls. An earlier version of this file re-implemented them locally, which
 * would have passed whether or not the shipped code still agreed with the copy.
 */

describe("merging the host's floor with the browser's", () => {
  it("adopts a stored floor for a channel this tab has not opened", () => {
    const merged = mergeReadFloors({}, [{ channelId: "engineering", lastReadAt: 5_000 }]);
    expect(merged.engineering).toBe(5_000);
  });

  it("keeps a fresher local floor when a channel was opened mid-flight", () => {
    // The operator opened the channel while the read-state request was still
    // in the air. Overwriting here would re-raise a badge they just cleared.
    const merged = mergeReadFloors({ engineering: 9_000 }, [
      { channelId: "engineering", lastReadAt: 5_000 },
    ]);
    expect(merged.engineering).toBe(9_000);
  });

  it("leaves channels the host said nothing about untouched", () => {
    const merged = mergeReadFloors({ "dm:pm": 3_000 }, [
      { channelId: "engineering", lastReadAt: 5_000 },
    ]);
    expect(merged["dm:pm"]).toBe(3_000);
    expect(merged.engineering).toBe(5_000);
  });

  it("treats an empty marker list as leaving the browser floor alone", () => {
    const merged = mergeReadFloors({ engineering: 1_234 }, []);
    expect(merged).toEqual({ engineering: 1_234 });
  });
});

describe("what the merged floor means for a badge", () => {
  // `from` is a union on `ChatMessage`; "company" is the not-you case.
  const transcript: Array<Pick<ChatMessage, "from" | "at">> = [
    { from: "company", at: 1_000 },
    { from: "you", at: 2_000 },
    { from: "company", at: 3_000 },
    { from: "company", at: 4_000 },
  ];

  it("counts what arrived after the stored floor, across a reload", () => {
    // The regression this issue is about: on load the browser floor is `now`,
    // which reads the whole transcript as caught up. The stored floor restores
    // the two messages that arrived after the operator last looked.
    const floor = mergeReadFloors({}, [{ channelId: "c", lastReadAt: 2_500 }]).c;
    expect(unreadCount(transcript, floor)).toBe(2);
  });

  it("never counts your own lines, whatever the floor says", () => {
    expect(unreadCount(transcript, 1_500)).toBe(2);
  });

  it("reports nothing once the floor is past the newest line", () => {
    expect(unreadCount(transcript, 4_000)).toBe(0);
  });
});
