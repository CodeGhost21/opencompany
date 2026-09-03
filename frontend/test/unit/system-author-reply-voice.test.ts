import { describe, expect, it } from "vitest";

import { fromHistory, replyVoice, SYSTEM_AUTHOR } from "@/lib/chat";
import type { ChatHistoryEntry } from "@/api/types";

/**
 * One settled turn must read the same whether you watched it or loaded it.
 *
 * A capped turn posts two lines: the agent's partial write-up, then the host's
 * pause notice. The notice is authored `SYSTEM_AUTHOR`, so `fromHistory` gives
 * it `from: "system"` — a centred pill, not an agent bubble.
 *
 * The live renderers used to build a `company` message unconditionally and pass
 * the author through only as the channel. Because `mergeHistoryInOrder` keeps
 * the existing live object for a matching durable id, that first impression was
 * never corrected: whoever watched the turn kept an agent-style System bubble
 * forever, while whoever opened the transcript later saw the intended system
 * row. Two readings of one turn — the same "who actually said this" confusion
 * the pause-attribution fix set out to remove (Codex review on #2068).
 */

const entry = (author: string): ChatHistoryEntry =>
  ({
    id: "7",
    author,
    text: "The reply above is a pause, not a finished answer",
    atMillis: 1,
    mine: false,
  }) as ChatHistoryEntry;

describe("a host-authored reply", () => {
  it("is a system row live, exactly as it is after a reload", () => {
    // What the reload has always done.
    expect(fromHistory([entry(SYSTEM_AUTHOR)])[0].from).toBe("system");
    // What the live renderers now do with the same author.
    expect(replyVoice(SYSTEM_AUTHOR)).toBe("system");
    expect(replyVoice(SYSTEM_AUTHOR)).toBe(fromHistory([entry(SYSTEM_AUTHOR)])[0].from);
  });

  it("leaves an agent's own reply in the company voice", () => {
    expect(replyVoice("product_manager")).toBe("company");
    expect(fromHistory([entry("product_manager")])[0].from).toBe("company");
  });

  it("treats an unattributed reply as the company, not the host", () => {
    // The frame omits `agentId` for a turn that named no responder. That is an
    // agent bubble with no name on it, never a host line — reading absence as
    // `system` would turn every unattributed reply into a centred pill.
    expect(replyVoice(undefined)).toBe("company");
    expect(replyVoice(null)).toBe("company");
    expect(replyVoice("")).toBe("company");
  });
});
