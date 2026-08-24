// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { ChatHistoryMessageDto } from "@/api/types";
import { fromHistory, mergeHistoryInOrder, type ChatMessage } from "@/lib/chat";

const here = dirname(fileURLToPath(import.meta.url));
const appShell = readFileSync(resolve(here, "../../src/components/app-shell.tsx"), "utf8");

describe("chat channel history polling", () => {
  it("wires each resolved channel fan-out to a disposable 5s visible-tab poll", () => {
    expect(appShell.match(/startVisiblePolling\(rehydrateAll, 5000\)/g)).toHaveLength(2);
    expect(appShell).toContain("disposeRehydratePolling?.();");
  });

  // The polling merge is `mergeHistoryInOrder` — the same reconstruction rule
  // the cold mount and every 5s tick share, which is what the source-wiring
  // test above arms but cannot itself observe. These exercise the real
  // function against the endpoint's oldest-first contract (issue #1690).

  const dto = (id: string, text: string, mine = false): ChatHistoryMessageDto => ({
    id,
    channel: "engineering",
    author: mine ? "operator" : "workflow",
    text,
    atMillis: 1_700_000_000_000 + Number(id),
    mine,
  });

  it("folds a first history read in and never duplicates it on a later tick", () => {
    const hydrated = fromHistory([dto("1686", "The workflow finished.")]);
    const first = mergeHistoryInOrder([], hydrated);
    expect(first.map((message) => message.text)).toEqual(["The workflow finished."]);

    // Same durable row on the next tick is id-seen: the identical array
    // reference comes back, so a caller can skip the state write (and React
    // the re-render).
    expect(mergeHistoryInOrder(first, hydrated)).toBe(first);
    expect(first).toHaveLength(1);
  });

  it("places a message recovered by a later tick after the transcript it follows", () => {
    const earlier = fromHistory([dto("1", "Kicking off the deploy.", true)])[0];
    // Oldest-first, matching the real endpoint's ordering.
    const recovered = fromHistory([dto("1", "Kicking off the deploy.", true), dto("2", "Deploy finished.")]);

    const merged = mergeHistoryInOrder([earlier], recovered);
    expect(merged.map((message) => message.text)).toEqual([
      "Kicking off the deploy.",
      "Deploy finished.",
    ]);
    // The transcript's own copy of the already-seen row survives the re-fetch
    // (reactions and other local decoration are kept), only the new row is new.
    expect(merged[0]).toBe(earlier);
  });

  it("fills a gap the live path left at the host's own position, not the tail", () => {
    // The SSE frame for 2 was missed while 1 and 3 landed: a plain append or
    // prepend rule would merge to `[1, 3, 2]` or `[2, 1, 3]`. The persisted
    // rows must take the history's order — `[1, 2, 3]`.
    const one = fromHistory([dto("1", "First", true)])[0];
    const three = fromHistory([dto("3", "Third")])[0];
    const hydrated = fromHistory([dto("1", "First", true), dto("2", "Second"), dto("3", "Third")]);

    const merged = mergeHistoryInOrder([one, three], hydrated);
    expect(merged.map((message) => message.text)).toEqual(["First", "Second", "Third"]);
  });

  it("keeps rows the host has not persisted yet at the tail", () => {
    // The operator's optimistic bubble is minted with a browser-local `m<seq>`
    // id the host does not know, so history does not name it. It must survive
    // the fold as the newest line, after every persisted row.
    const persisted = fromHistory([dto("1", "Kicking off the deploy.", true)])[0];
    const optimistic: ChatMessage = {
      id: "m42",
      from: "you",
      text: "unacked local bubble",
      at: 1_700_000_010_000,
    };

    const merged = mergeHistoryInOrder([persisted, optimistic], fromHistory([dto("1", "Kicking off the deploy.", true)]));
    expect(merged).toEqual([persisted, optimistic]);
    expect(merged[1]).toBe(optimistic);
  });
});
