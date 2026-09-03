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
 * The row used to select a thread through shell state and land on a separate
 * two-pane surface, a selection the address could not carry. Chats are Room
 * now, and the channel is the whole destination.
 */
describe("a card's origin row opens Room", () => {
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

  it("keeps the retired transcript surface out of the shell entirely", () => {
    // The destination has to stay an address. A card's origin row selecting a
    // thread through shell state is what this forbids, and the surface that
    // read that state is gone — so the whole view goes with it.
    expect(shell).not.toContain("setActiveThreadId");
    expect(shell).not.toContain('setView("conversation")');
    expect(shell).not.toContain("<Conversation");
    expect(shell).not.toContain('view === "conversation"');
  });
});
