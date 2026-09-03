import { describe, expect, it } from "vitest";

import { deskOfStateKey, turnStateKey } from "@/lib/live-reply";

/**
 * The settle re-read must find a desk for **every** open row, not only the ones
 * folded out of `/runs`.
 *
 * `onSendDetached` mints its entry from the 202 alone — `{ turnId, queued }`,
 * no `chatId` — and that is exactly the row the SSE-unavailable path watches,
 * where the poll is the only delivery route there is. A settle that re-read
 * only when `chatId` was present skipped that row entirely, leaving the live
 * receipt ticking over a reply that had already landed
 * (`chat-detached-sse-unavailable.spec.ts`).
 *
 * So the desk is recovered from the key itself, which every row has.
 */
describe("deskOfStateKey", () => {
  it("returns a plain channel key unchanged", () => {
    expect(deskOfStateKey("engineering")).toBe("engineering");
  });

  it("strips the thread root off a composite key", () => {
    expect(deskOfStateKey("engineering#41")).toBe("engineering");
  });

  it("inverts turnStateKey for both shapes", () => {
    expect(deskOfStateKey(turnStateKey("engineering", 41))).toBe("engineering");
    expect(deskOfStateKey(turnStateKey("engineering", undefined))).toBe("engineering");
    // A root of 0 is a real event seq, not an absent one.
    expect(deskOfStateKey(turnStateKey("engineering", 0))).toBe("engineering");
  });

  it("keeps a desk id that itself contains a hash", () => {
    // Anchored on a numeric suffix rather than the last `#`, so a desk named
    // with one is not silently truncated to a shorter, different desk.
    expect(deskOfStateKey("c#4-eng")).toBe("c#4-eng");
    expect(deskOfStateKey("c#4-eng#41")).toBe("c#4-eng");
  });
});
