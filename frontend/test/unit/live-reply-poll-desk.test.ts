import { describe, expect, it } from "vitest";

import { openTurnsFromRuns, turnStateKey, type OpenRunRow } from "@/lib/live-reply";

/**
 * An open turn carries the **desk** it belongs to, not just the key it is filed
 * under (Codex review on #2042).
 *
 * The map is keyed per thread, so a threaded turn is filed under a composite
 * like `engineering#41`. The settle poll used to read that key back as a host
 * thread id and hand it to `getChatHistory`, asking for a desk called
 * `engineering#41` — which no host has. The row settled, the durable reply was
 * never rehydrated, and the loss only showed on the poll path, i.e. exactly
 * when SSE was unavailable and the poll was the sole delivery route.
 */
describe("an open turn keeps its desk alongside its state key", () => {
  const run = (over: Partial<OpenRunRow> & { id: string }): OpenRunRow => ({
    chatId: "engineering",
    status: "running",
    ...over,
  });

  it("carries the plain chatId on a threaded turn, whose key is composite", () => {
    const open = openTurnsFromRuns([run({ id: "t1", threadRoot: 41 })]);
    const key = turnStateKey("engineering", 41);

    expect(Object.keys(open)).toEqual([key]);
    expect(key).toBe("engineering#41");
    expect(open[key][0].chatId, "the desk the poll must re-read").toBe("engineering");
  });

  it("carries it on an unrooted turn too, where key and desk coincide", () => {
    const open = openTurnsFromRuns([run({ id: "t1" })]);
    expect(open["engineering"][0].chatId).toBe("engineering");
  });

  it("keeps each thread's desk when one channel holds several", () => {
    const open = openTurnsFromRuns([
      run({ id: "t1", threadRoot: 41 }),
      run({ id: "t2", threadRoot: 77 }),
    ]);
    expect(open["engineering#41"][0].chatId).toBe("engineering");
    expect(open["engineering#77"][0].chatId).toBe("engineering");
  });
});
