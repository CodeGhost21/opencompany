import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The channel's indicator reports a **running** turn over a queued one
 * (Codex review on #2042).
 *
 * `openTurn` aggregates every open turn in the channel except the open
 * thread's. Taking the first match let map order decide the wording, and map
 * order follows `/runs` — which is newest-first. So the ordinary serialized
 * case, an older turn working while a newer one waits on the company lock, put
 * the queued thread first and the channel rendered "Queued…" over live work.
 *
 * Asserted against the source because the value is computed inline in
 * `ChatView`'s render from three pieces of shell state; the alternative is
 * mounting the view with a hand-built `openTurns` map, which tests React's
 * wiring rather than the precedence rule.
 */
const here = dirname(fileURLToPath(import.meta.url));
const chatView = readFileSync(resolve(here, "../../src/views/ChatView.tsx"), "utf8");

describe("the channel's working row prefers a running turn", () => {
  it("selects a non-queued head before falling back to a queued one", () => {
    expect(chatView).toMatch(/heads\.find\(\(t\) => !t\.queued\) \?\? heads\[0\]/);
  });

  it("no longer takes whichever entry the map happens to yield first", () => {
    // The pre-fix shape: `.find(...)?.[1][0]` over `Object.entries`, which is
    // "first matching key" and says nothing about whether it is running.
    expect(chatView).not.toMatch(/Object\.entries\(openTurns \?\? \{\}\)\.find\(/);
  });

  it("still excludes the open thread's own turn from the channel's row", () => {
    // The exclusion is what stops the channel and the panel both claiming the
    // same turn; the precedence change must not have dropped it.
    expect(chatView).toMatch(/key !== threadTurnKey/);
  });
});
