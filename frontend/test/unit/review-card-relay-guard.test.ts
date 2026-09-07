import { describe, expect, it } from "vitest";

import { reviewCardIdForThread } from "@/views/chat/model";
import type { ChatMessage } from "@/lib/chat";
import type { TaskStatus } from "@/api/tasks";

function line(overrides: Partial<ChatMessage> & Pick<ChatMessage, "id" | "from">): ChatMessage {
  return { text: "", at: 0, ...overrides };
}

const inReview: Readonly<Record<string, TaskStatus>> = {
  "t-1": { column: "in_review" },
};

describe("reviewCardIdForThread", () => {
  it("treats the settle pill itself as a review surface", () => {
    const pill = line({ id: "m1", from: "system", taskId: "t-1" });
    expect(reviewCardIdForThread(pill, [pill], inReview)).toBe("t-1");
  });

  it("treats the pill's first company reply as a review surface", () => {
    const pill = line({ id: "m1", from: "system", taskId: "t-1" });
    const relay = line({ id: "m2", from: "company" });
    const messages = [pill, relay];
    expect(reviewCardIdForThread(relay, messages, inReview)).toBe("t-1");
  });

  it("does not treat a later ordinary reply as a review surface", () => {
    const pill = line({ id: "m1", from: "system", taskId: "t-1" });
    const relay = line({ id: "m2", from: "company" });
    const laterReply = line({ id: "m3", from: "company" });
    const messages = [pill, relay, laterReply];
    expect(reviewCardIdForThread(laterReply, messages, inReview)).toBeUndefined();
  });

  it("still ignores a card that has left in_review", () => {
    const pill = line({ id: "m1", from: "system", taskId: "t-1" });
    const relay = line({ id: "m2", from: "company" });
    const messages = [pill, relay];
    const settled: Readonly<Record<string, TaskStatus>> = {
      "t-1": { column: "done" },
    };
    expect(reviewCardIdForThread(relay, messages, settled)).toBeUndefined();
  });
});
