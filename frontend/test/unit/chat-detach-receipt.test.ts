import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * Source-wiring coverage for issue #2021 — the live receipt (issue #1934) riding
 * a detached turn past its 202 into the queued/working window, on every send
 * surface, instead of being dropped for a bare open-turn row.
 *
 * The behaviour lives in three large hosts that the repo already declines to
 * mount for this kind of wiring assertion (`chat-receipt-scope-reset.test.ts`
 * settles `AppShell` the same way): `AppShell`'s send-outcome callbacks and poll
 * settle, `MessageTimeline`'s render gate, and `Transcript`'s working row. The
 * *semantics* of what the receipt shows in each state are proven directly by
 * `chat-live-receipt.test.ts` (`ChatLiveReceipt` + `receiptStateLine`); what
 * this file locks down is that each host is actually wired into them.
 */

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../", rel), "utf8");

const appShell = read("src/components/app-shell.tsx");
const messageTimeline = read("src/views/chat/MessageTimeline.tsx");
const transcript = read("src/views/conversation/Transcript.tsx");
const conversation = read("src/views/Conversation.tsx");

/** Slice a `const <name> = useCallback(` body up to its dependency array. */
function callbackBody(source: string, name: string, depsMarker: string): string {
  const start = source.indexOf(`const ${name} = useCallback(`);
  expect(start, `${name} must be present`).toBeGreaterThan(-1);
  const end = source.indexOf(depsMarker, start);
  expect(end, `${name}'s dependency array (${depsMarker})`).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe("onSendDetached keeps the receipt alive across the 202 handoff", () => {
  it("does not clear the receipt — the turn carries it into the open-turn window", () => {
    const body = callbackBody(appShell, "onSendDetached", "[renderAgentReply],");
    // The 202 still arms the open-turn row…
    expect(body).toMatch(/setOpenTurns\(\(prev\) => \{/);
    // …but no longer tears the receipt down: the poll's settle does that later.
    expect(body).not.toContain("clearReceipt(");
  });
});

describe("onSendFailed defers the receipt clear to the durable-turn answer", () => {
  it("does not clear before it knows whether the host kept the turn", () => {
    const body = callbackBody(appShell, "onSendFailed", "[client, company, renderAgentReply, clearReceipt],");
    const listRunsAt = body.indexOf("listRuns(");
    expect(listRunsAt, "onSendFailed must query the run store").toBeGreaterThan(-1);
    // The pre-#2021 shape cleared the receipt unconditionally up front, before
    // the listRuns query — which is exactly what dropped the affordances for a
    // throw whose turn the host actually kept running.
    expect(body.slice(0, listRunsAt)).not.toContain("clearReceipt(");
  });

  it("clears only when no durable turn survived (the else) or the query failed (the catch)", () => {
    const body = callbackBody(appShell, "onSendFailed", "[client, company, renderAgentReply, clearReceipt],");
    // A durable turn is kept alive (settle clears it); no durable turn drops it.
    expect(body).toMatch(/else clearReceipt\(threadId, gen\);/);
    // Offline / host without /runs — nothing will ever settle it, so drop it.
    expect(body).toMatch(/\.catch\(\(\) => \{[\s\S]*clearReceipt\(threadId, gen\);[\s\S]*\}\);/);
  });
});

describe("the poll's terminal settle clears the receipt under the open-turns guard", () => {
  it("clears the receipt beside the live rows, inside the hasOtherOpenTurns guard", () => {
    // The guard block in reReadSettledThread that already clears the live tool
    // rows only when no sibling turn is still running — the receipt clear must
    // sit inside the SAME guard so a queued sibling keeps it, and after the
    // company-scope checks above it so a late cross-company settle cannot delete
    // a newer company's receipt.
    const guardAt = appShell.indexOf(
      "if (!hasOtherOpenTurns(openTurnsRef.current, threadId, settledTurnId)) {",
    );
    expect(guardAt, "the settle guard must be present").toBeGreaterThan(-1);
    const block = appShell.slice(guardAt, guardAt + 900);
    expect(block).toContain("setLiveStepsByThread(");
    expect(block).toContain("setReceiptByThread(");
    expect(block).toMatch(/delete next\[threadId\];/);
  });
});

describe("MessageTimeline renders the receipt in the queued state too", () => {
  it("gates the receipt on its presence alone, not on !queued", () => {
    // Pre-#2021 the gate was `receipt && !queued ?`, which handed a queued
    // detached turn back to the bare "Queued…" row. It is now `receipt ?`.
    expect(messageTimeline).not.toMatch(/\{receipt && !queued \?/);
    expect(messageTimeline).toMatch(/\{receipt \? \(/);
  });

  it("forwards queued into ChatLiveReceipt so its base line stays honest", () => {
    const at = messageTimeline.indexOf("<ChatLiveReceipt");
    expect(at, "MessageTimeline must render ChatLiveReceipt").toBeGreaterThan(-1);
    expect(messageTimeline.slice(at, at + 300)).toContain("queued={queued}");
  });
});

describe("the Conversation surface renders the receipt too (both send surfaces)", () => {
  it("AppShell feeds Conversation the receipt map and the name map", () => {
    const at = appShell.indexOf("<Conversation");
    expect(at, "AppShell must render Conversation").toBeGreaterThan(-1);
    const block = appShell.slice(at, at + 600);
    expect(block).toContain("receiptByThread={receiptByThread}");
    expect(block).toContain("agentNames={agentNames}");
  });

  it("Conversation threads the resolved receipt down to the transcript", () => {
    expect(conversation).toContain("receipt={receiptByThread?.[active.id]}");
    expect(conversation).toMatch(/receipt=\{receipt\}/);
  });

  it("Transcript shows ChatLiveReceipt in place of the bare typing row when one is armed", () => {
    expect(transcript).toContain("<ChatLiveReceipt");
    expect(transcript).toMatch(/working &&\s*\n?\s*\(receipt \?/);
    // And keeps the queued wording honest here as well.
    const at = transcript.indexOf("<ChatLiveReceipt");
    expect(transcript.slice(at, at + 300)).toContain("queued={openTurn?.queued}");
  });
});
