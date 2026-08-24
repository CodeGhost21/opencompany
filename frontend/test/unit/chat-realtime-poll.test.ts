// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it, vi } from "vitest";

import type { ChatHistoryMessageDto } from "@/api/types";
import { fromHistory, type ChatMessage } from "@/lib/chat";
import { startVisiblePolling } from "@/lib/visible-poll";

const here = dirname(fileURLToPath(import.meta.url));
const appShell = readFileSync(resolve(here, "../../src/components/app-shell.tsx"), "utf8");

afterEach(() => {
  vi.useRealTimers();
});

describe("chat channel history polling", () => {
  it("wires each resolved channel fan-out to a disposable 5s visible-tab poll", () => {
    expect(appShell.match(/startVisiblePolling\(rehydrateAll, 5000\)/g)).toHaveLength(2);
    expect(appShell).toContain("disposeRehydratePolling?.();");
  });

  it("adds a persisted message without a remount and does not duplicate it", async () => {
    vi.useFakeTimers();

    const entry: ChatHistoryMessageDto = {
      id: "1686",
      channel: "engineering",
      author: "workflow",
      text: "The workflow finished.",
      atMillis: 1_700_000_000_000,
      mine: false,
    };
    let history: ChatHistoryMessageDto[] = [];
    const client = {
      getChatHistory: vi.fn(async () => history),
    };
    let transcript: ChatMessage[] = [];
    const rehydrateAll = async () => {
      const hydrated = fromHistory(await client.getChatHistory());
      const known = new Set(transcript.map((message) => message.id));
      const fresh = hydrated.filter((message) => !known.has(message.id));
      if (fresh.length > 0) transcript = [...fresh, ...transcript];
    };

    // The existing mount hydrate runs before polling is armed.
    await rehydrateAll();
    const dispose = startVisiblePolling(() => void rehydrateAll(), 5000);
    expect(transcript).toEqual([]);

    // A workflow writes while the shell remains mounted and its SSE frame is
    // missed. The next visible tick recovers it from durable history.
    history = [entry];
    await vi.advanceTimersByTimeAsync(5000);
    expect(transcript.map((message) => message.text)).toEqual(["The workflow finished."]);

    // The same durable row on the next tick is id-seen and changes nothing.
    await vi.advanceTimersByTimeAsync(5000);
    expect(transcript).toHaveLength(1);
    expect(client.getChatHistory).toHaveBeenCalledTimes(3);

    dispose();
  });
});
