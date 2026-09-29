import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The host's `replying` frame (an agent started writing its reply) is wired
 * through three files that only meet at runtime, so this pins the seams in the
 * source-contract idiom of `chat-rail-focus.test.ts`:
 *
 *  - `use-events` routes it to `onTurnEvent`, not the `default` arm that warns;
 *  - the shell's `onTurnEvent` accepts it but never folds it into a row, or the
 *    live and folded step counts would disagree;
 *  - the shell gives it no timer (a person's `typing` frame expires after 8s, an
 *    agent's reply can stream longer).
 */

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("the replying frame", () => {
  const events = read("hooks/use-events.ts");
  const shell = read("components/app-shell.tsx");

  it("is a member of the frame union and routed with the other turn frames", () => {
    expect(events).toContain('type: "replying";');
    expect(events).toMatch(/case "thinking":\s*case "replying":\s*onTurnEvent\?\.\(event\);/);
  });

  it("is accepted by the shell but kept out of the folded rows", () => {
    expect(shell).toContain('event.type !== "replying"');
    expect(shell).toContain('if (event.type !== "replying") {');
    const guard = shell.indexOf('if (event.type !== "replying") {');
    const fold = shell.indexOf("foldLiveFrame(prev[rowKey]", guard);
    expect(fold).toBeGreaterThan(guard);
  });

  it("has no expiry of its own", () => {
    const start = shell.indexOf("const isReplying");
    const body = shell.slice(start, start + 400);
    expect(body).not.toMatch(/setTimeout|expire|TTL/i);
  });
});
