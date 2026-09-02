import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

/**
 * Where "Open the conversation" sends the operator, pinned by reading the
 * shell's source: a jsdom render of `app-shell` needs the whole client and
 * every hook, the same reason `task-detail-no-op-plumbing` and
 * `chat-task-origin-visibility` check their contracts this way.
 *
 * The row used to select a thread with `setActiveThreadId` and land on
 * `#/conversation` — the legacy two-pane surface with its own thread rail, and
 * a selection the address could not carry. Chats are Room now, and the channel
 * is the whole destination.
 */
describe("a card's origin row opens Room, not the legacy Conversation view", () => {
  const shell = read("components/app-shell.tsx");

  it("routes the origin channel through the address", () => {
    expect(shell).toContain("onOpenChannel={(channelId, threadId) =>");
    expect(shell).toContain('navigate("chat", channelId, { thread: threadId ?? null })');
  });

  it("hands the detail route the map that places the origin thread", () => {
    expect(shell).toContain(
      "chatChannelByThread={chatChannelByThread}\n              onOpenChannel=",
    );
  });

  it("no longer sends a card's origin to the legacy Conversation surface", () => {
    // `#/conversation` stays routable (`ROUTABLE.conversation`) and the view
    // stays mounted for it; what must not come back is a card's origin row
    // selecting a thread in it through state the address knows nothing about.
    expect(shell).not.toContain("setActiveThreadId(threadId);");
    expect(shell).not.toContain('setView("conversation")');
  });
});
