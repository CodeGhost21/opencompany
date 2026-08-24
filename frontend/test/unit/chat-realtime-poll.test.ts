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
    // Mirrors `hydrateChannel`'s actual merge rule: the mount-time call's
    // `fresh` rows are the whole history and belong in front of anything
    // already shown, but once a channel has hydrated once, a later tick's
    // `fresh` rows are new tail rows `chat/history` just grew and belong
    // after everything already shown (issue #1690 — the polling path used
    // to always prepend, which put a recovered reply *before* the
    // transcript it followed).
    let hydratedOnce = false;
    const rehydrateAll = async () => {
      const isPoll = hydratedOnce;
      hydratedOnce = true;
      const hydrated = fromHistory(await client.getChatHistory());
      const known = new Set(transcript.map((message) => message.id));
      const fresh = hydrated.filter((message) => !known.has(message.id));
      if (fresh.length > 0) {
        transcript = isPoll ? [...transcript, ...fresh] : [...fresh, ...transcript];
      }
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

  it("appends a message recovered by a later poll tick after the transcript it follows", async () => {
    vi.useFakeTimers();

    const earlier: ChatHistoryMessageDto = {
      id: "1",
      channel: "engineering",
      author: "operator",
      text: "Kicking off the deploy.",
      atMillis: 1_700_000_000_000,
      mine: true,
    };
    const recovered: ChatHistoryMessageDto = {
      id: "2",
      channel: "engineering",
      author: "workflow",
      text: "Deploy finished.",
      atMillis: 1_700_000_005_000,
      mine: false,
    };
    let history: ChatHistoryMessageDto[] = [earlier];
    const client = {
      getChatHistory: vi.fn(async () => history),
    };
    let transcript: ChatMessage[] = [];
    let hydratedOnce = false;
    const rehydrateAll = async () => {
      const isPoll = hydratedOnce;
      hydratedOnce = true;
      const hydrated = fromHistory(await client.getChatHistory());
      const known = new Set(transcript.map((message) => message.id));
      const fresh = hydrated.filter((message) => !known.has(message.id));
      if (fresh.length > 0) {
        transcript = isPoll ? [...transcript, ...fresh] : [...fresh, ...transcript];
      }
    };

    // Mount hydrate lands the earlier message first.
    await rehydrateAll();
    expect(transcript.map((message) => message.text)).toEqual(["Kicking off the deploy."]);

    const dispose = startVisiblePolling(() => void rehydrateAll(), 5000);

    // `chat/history` now includes the reply the live SSE frame missed.
    // Oldest-first, matching the real endpoint's ordering.
    history = [earlier, recovered];
    await vi.advanceTimersByTimeAsync(5000);

    // The recovered reply must land after the message it replies to, not
    // ahead of the whole transcript.
    expect(transcript.map((message) => message.text)).toEqual([
      "Kicking off the deploy.",
      "Deploy finished.",
    ]);

    dispose();
  });
});
