import { describe, expect, it } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import { foldEpisodes } from "@/lib/hive/episode";
import { buildTimeline, buildTimelineItems, type Channel } from "@/views/room/model";

const CHANNEL: Channel = {
  id: "solvers",
  name: "solvers",
  voice: "Solvers desk",
  kind: "channel",
  purpose: "",
};

/**
 * Collapsing a room's turns into one timeline item.
 *
 * The additive-parameter shape is the point: `buildTimelineItems` grew a fourth
 * argument, so every call site and every spec written before deliberation keeps
 * its exact behaviour. The first test here is the one that matters — with no
 * episodes the function must return precisely what it always did, because that
 * is what lets ~36 existing model specs go on being the regression net for this
 * whole change rather than casualties of it.
 */

let seq = 0;
function op(text: string): ChatMessage {
  return { id: `h${++seq}`, from: "you", text, at: seq * 1000, byPerson: true };
}
function turn(agentId: string, text: string): ChatMessage {
  return { id: `h${++seq}`, from: "company", channel: agentId, text, at: seq * 1000 };
}
function report(text: string): ChatMessage {
  return { id: `h${++seq}`, from: "company", channel: "hive-report", text, at: seq * 1000 };
}

function transcript(messages: ChatMessage[]) {
  return buildTimeline(messages, CHANNEL, []);
}

describe("a channel with no room", () => {
  it("returns exactly what it did before episodes existed", () => {
    seq = 0;
    const messages = [op("hello"), turn("analyst", "Hi.")];
    const entries = transcript(messages);
    const before = buildTimelineItems(entries, [], {});
    const after = buildTimelineItems(entries, [], {}, []);
    expect(after).toEqual(before);
    expect(after.every((item) => item.kind === "message")).toBe(true);
  });

  it("leaves a plain transcript alone even when the fold is offered it", () => {
    seq = 0;
    const messages = [op("hello"), turn("analyst", "Hi.")];
    const entries = transcript(messages);
    const episodes = foldEpisodes(messages);
    expect(episodes).toEqual([]);
    expect(buildTimelineItems(entries, [], {}, episodes)).toHaveLength(2);
  });
});

describe("a channel with a room", () => {
  const messages = () => {
    seq = 0;
    return [
      op("decide the rollout"),
      turn("planner", "!propose #stage ship to staging first"),
      turn("critic", "!evidence #stage ^1 the last rollout took checkout down"),
      turn("critic", "!support #stage ^3 staging first"),
      report("The desk settled on #stage after 3 turns (backed by planner, critic)."),
    ];
  };

  it("collapses the room's rows into one item and leaves the question outside it", () => {
    // The question is the operator's and the answer is the room's. Nesting the
    // former inside the latter reads as though the desk asked itself.
    const rows = messages();
    const items = buildTimelineItems(
      transcript(rows),
      [],
      {},
      foldEpisodes(rows, { members: 3 }),
    );
    expect(items.map((i) => i.kind)).toEqual(["message", "episode"]);
    const block = items[1];
    if (block.kind !== "episode") throw new Error("expected an episode block");
    // Three turns plus the closing report.
    expect(block.items).toHaveLength(4);
  });

  it("puts the block where the room started, not where it ended", () => {
    const rows = messages();
    const items = buildTimelineItems(
      transcript(rows),
      [],
      {},
      foldEpisodes(rows, { members: 3 }),
    );
    expect(items[1].at).toBe(2000);
  });

  it("carries each row's folded turn, so a renderer needs no second parse", () => {
    const rows = messages();
    const items = buildTimelineItems(
      transcript(rows),
      [],
      {},
      foldEpisodes(rows, { members: 3 }),
    );
    const block = items[1];
    if (block.kind !== "episode") throw new Error("expected an episode block");
    expect(block.turnByMessageId["h2"]?.move?.kind).toBe("propose");
    expect(block.turnByMessageId["h4"]?.move?.cites).toEqual([3]);
  });

  it("does not wrap a later ordinary exchange on the same desk in an episode block", () => {
    // The desk deliberated once, then answered a plain follow-up question
    // ordinarily. `hasRoom` is true for the whole channel, but the second
    // exchange must still render as two bare messages, not a second
    // "still deliberating" block around an ordinary reply (regression for
    // the false positive Codex flagged on foldEpisodes' segmentation).
    seq = 0;
    const rows = [
      ...(() => {
        const opened = [
          op("decide the rollout"),
          turn("planner", "!propose #stage ship to staging first"),
          report("The desk settled on #stage after 1 turn (backed by planner)."),
        ];
        return opened;
      })(),
      op("what time did that ship?"),
      turn("planner", "Around 9am."),
    ];
    const items = buildTimelineItems(
      transcript(rows),
      [],
      {},
      foldEpisodes(rows, { members: 1, quorum: 1 }),
    );
    expect(items.map((i) => i.kind)).toEqual([
      "message",
      "episode",
      "message",
      "message",
    ]);
  });

  it("keeps two rooms in two blocks", () => {
    seq = 0;
    const rows = [
      op("first"),
      turn("planner", "!propose #a one"),
      report("The desk spent its 1-turn budget without reaching a decision."),
      op("second"),
      turn("planner", "!propose #b two"),
      report("The desk spent its 1-turn budget without reaching a decision."),
    ];
    const items = buildTimelineItems(
      transcript(rows),
      [],
      {},
      foldEpisodes(rows, { members: 3 }),
    );
    expect(items.map((i) => i.kind)).toEqual([
      "message",
      "episode",
      "message",
      "episode",
    ]);
  });
});
