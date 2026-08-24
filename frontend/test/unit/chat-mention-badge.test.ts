import { describe, expect, it } from "vitest";

import type { NotificationDto } from "@/api/types";
import { mentionCountsByChannel, mentionsToClear } from "@/lib/mention-badge";

/**
 * The mention badge is the durable half of the feature: the SSE feed only
 * reaches an open browser, so a mention that landed overnight is visible here
 * and nowhere else. Getting the counting wrong therefore does not degrade the
 * feature, it removes it — a badge that clears too eagerly loses the summons
 * entirely, with nothing left to notice it by.
 */

const note = (over: Partial<NotificationDto> & Pick<NotificationDto, "id">): NotificationDto => ({
  kind: "mention",
  subjectKind: "message",
  subjectId: "42",
  title: "someone mentioned you",
  createdAt: 1,
  context: "engineering",
  ...over,
});

describe("mentionCountsByChannel", () => {
  it("counts unread mentions per channel", () => {
    expect(
      mentionCountsByChannel([
        note({ id: "a" }),
        note({ id: "b" }),
        note({ id: "c", context: "design" }),
      ]),
    ).toEqual({ engineering: 2, design: 1 });
  });

  it("ignores a mention that has been read", () => {
    expect(
      mentionCountsByChannel([note({ id: "a", readAt: 5 }), note({ id: "b" })]),
    ).toEqual({ engineering: 1 });
  });

  /**
   * `kind`, not `subjectKind`. A later notification about a message that is not
   * a mention — a reply, a reaction — must not silently start badging as one.
   */
  it("counts only rows whose kind is a mention", () => {
    expect(
      mentionCountsByChannel([
        note({ id: "a", kind: "reply" }),
        note({ id: "b" }),
      ]),
    ).toEqual({ engineering: 1 });
  });

  it("drops a row with no channel rather than placing it arbitrarily", () => {
    expect(mentionCountsByChannel([note({ id: "a", context: undefined })])).toEqual({});
  });

  it("maps the legacy main thread onto the rendered main channel", () => {
    expect(mentionCountsByChannel([note({ id: "a", context: "main" })], "general")).toEqual({
      general: 1,
    });
  });


    expect(mentionCountsByChannel([])).toEqual({});
  });

  /**
   * A host answering `GET {scope}/notifications` with something other than the
   * documented shape must not take the console down.
   *
   * This is not hypothetical: a mocked host that returns a bare `[]` for
   * unmatched routes made `feed.notifications` `undefined`, and iterating it
   * threw during render — blanking the entire app and failing every unrelated
   * spec in the file. The badge is the least important thing on the screen and
   * has to fail like it.
   */
  it("survives a caller handing it something that is not a list", () => {
    for (const bad of [undefined, null, "nope", 7, {}]) {
      expect(
        mentionCountsByChannel(bad as unknown as NotificationDto[]),
      ).toEqual({});
    }
  });
});

describe("mentionsToClear", () => {
  const feed = [
    note({ id: "eng-1" }),
    note({ id: "eng-2" }),
    note({ id: "eng-read", readAt: 9 }),
    note({ id: "design-1", context: "design" }),
  ];

  /**
   * The case a bare "mark all read" gets wrong: opening one channel must not
   * clear a summons waiting in another.
   */
  it("clears only the opened channel's unread mentions", () => {
    expect(mentionsToClear(feed, "engineering")).toEqual(["eng-1", "eng-2"]);
    expect(mentionsToClear(feed, "design")).toEqual(["design-1"]);
  });

  it("returns nothing for a channel with no mentions", () => {
    expect(mentionsToClear(feed, "random")).toEqual([]);
  });

  /**
   * An empty list is a real instruction to the host ("mark nothing"), distinct
   * from omitting ids ("mark everything") — so the caller must not send it as
   * though it meant the latter.
   */
  it("returns an empty list rather than undefined when there is nothing to clear", () => {
    expect(mentionsToClear([], "engineering")).toEqual([]);
  });
});
