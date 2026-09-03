import { describe, expect, it } from "vitest";

import { isGeneralChannel } from "@/lib/chat";
import { channelForThread, dmThreadId } from "@/views/chat/model";
import type { TeamMember } from "@/lib/team";

/**
 * Which spellings a live turn frame's thread id may be rewritten through
 * before it keys `liveStepsByThread` / `receiptByThread`.
 *
 * `onTurnEvent` normalizes **General aliases only**. The reason it normalizes
 * at all: the host folds the company-wide line under whatever casing the
 * caller addressed, so a client posting to `General` has its frames emitted
 * under that spelling while the console armed these maps at its own General
 * id — rows written where no reader looks (issue #1743).
 *
 * The reason it stops there is this file. These maps are keyed in the
 * **host-thread** namespace: `ChatView` reads them by `dmThreadId(member)` and
 * `onSendStart` arms them under that same id, which for an ordinary teammate
 * is the bare member id. `channelForThread` answers with the DM *channel* id,
 * `dm:<id>` — a different string. Routing every id through the map moves DM
 * live state to a key nothing reads (PR #2068 review).
 */

const ADA: TeamMember = { id: "ada", name: "Ada" } as TeamMember;

/** The shell's thread → channel map, as `channelMap` builds it for one DM. */
const MAP: Record<string, string> = { ada: "dm:ada", "": "general", main: "general" };

/** The rule `onTurnEvent` applies, isolated from the React shell. */
const keyFor = (frameThreadId: string): string =>
  isGeneralChannel(frameThreadId) ? (channelForThread(MAP, frameThreadId) ?? frameThreadId) : frameThreadId;

describe("a live frame's thread key", () => {
  it("leaves a teammate DM in the host-thread namespace the readers use", () => {
    // The identity `ChatView` reads by and `onSendStart` arms under.
    expect(dmThreadId(ADA)).toBe("ada");
    // So the frame must key the same way — not `dm:ada`, which the map answers
    // with and which no reader or receipt lookup ever asks for.
    expect(keyFor("ada")).toBe("ada");
    expect(keyFor("ada")).toBe(dmThreadId(ADA));
    expect(channelForThread(MAP, "ada")).toBe("dm:ada");
  });

  it("resolves a General alias, whatever casing the caller addressed", () => {
    expect(keyFor("main")).toBe("general");
    expect(keyFor("")).toBe("general");
  });

  it("keys a call and its result identically even if the directory loads between them", () => {
    // The map is built from the directory, so it can fill in mid-turn. A DM id
    // is never routed through it, so both frames land in one bucket and the
    // row flips `running → ok` instead of hanging.
    const beforeDirectory = ((id: string) =>
      isGeneralChannel(id) ? (channelForThread({}, id) ?? id) : id)("ada");
    expect(beforeDirectory).toBe(keyFor("ada"));
  });
});
