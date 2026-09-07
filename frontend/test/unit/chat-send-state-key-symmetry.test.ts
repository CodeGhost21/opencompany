import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The send arms and clears live-turn state under the **same** key.
 *
 * `onSendStart` arms `openTurns` / `liveStepsByThread` / `receiptByThread`, and
 * the three terminal callbacks clear them. When the arm moved to a thread-scoped
 * key, the callbacks were left on `chatId`: a threaded send then armed
 * `engineering#41` and cleared `engineering`, so the armed state was never
 * released and the clear landed on the channel's own (CodeRabbit review on
 * #2042).
 *
 * Asserted against the source rather than by mounting `ChatView`: the failure is
 * "one of four call sites was not updated", which a behavioural test only
 * catches if it happens to exercise that outcome — and there are three, one of
 * which needs a POST to throw.
 */
const here = dirname(fileURLToPath(import.meta.url));
const chatView = readFileSync(resolve(here, "../../src/views/ChatView.tsx"), "utf8");

describe("the send's live-state key is symmetric (CodeRabbit review on #2042)", () => {
  it("arms on stateKey", () => {
    expect(chatView).toMatch(/onSendStart\?\.\(stateKey\)/);
  });

  it("clears on stateKey in all three terminal outcomes", () => {
    expect(chatView).toMatch(/onSendDetached\?\.\(stateKey,/);
    expect(chatView).toMatch(/onSendEnd\?\.\(stateKey,/);
    expect(chatView).toMatch(/onSendFailed\?\.\(stateKey,/);
  });

  it("never passes the bare chatId to a send lifecycle callback", () => {
    // The regression this file exists for, in the shape it actually took.
    for (const cb of ["onSendStart", "onSendDetached", "onSendEnd", "onSendFailed"]) {
      expect(chatView, `${cb} must not be called with the raw chatId`).not.toMatch(
        new RegExp(`${cb}\\?\\.\\(chatId[,)]`),
      );
    }
  });

  it("derives the key from the open thread, not from the reply's parent", () => {
    // A review reply's `parentId` is an anchor *reply*, not the thread root, so
    // keying on it would arm something the panel's own lookup could not match.
    expect(chatView).toMatch(/turnStateKey\(chatId, threadRootOf\(openThreadId \?\? undefined\)\)/);
  });
});
