import { describe, expect, it } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import { isDetachedChat, type ChatPostResult } from "@/api/types";
import { mergeOpenTurns, openTurnsFromRuns, PendingSyncPosts } from "@/lib/live-reply";
import { reconcileIds, type ChatMessage } from "@/lib/chat";
import type { Transport, TransportRequest, TransportResponse } from "@/api/transport";

/**
 * Posts a chat turn with `detach: true`, so the flag is not buried behind the
 * positional arguments that separate it from the prompt (issue #983).
 */
function postDetached(c: OpenCompanyClient, text: string): Promise<ChatPostResult> {
  return c.chat(text, null, "main", null, undefined, true);
}

/**
 * Detached chat turns (issue #983).
 *
 * The gap this closes is total: the chat POST's error and hang paths had no
 * coverage at all, which is how a route contractually defined to answer with the
 * *finished* turn — an operation of unbounded duration — went to production and
 * 504'd five requests out of five. Nothing here is guarded by an existing test,
 * so each case is written as if it were the only thing standing between the
 * change and the operator.
 */

/** A transport that answers one canned body, and records what it was asked. */
function stubTransport(status: number, body: unknown) {
  const sent: { url: string; body: unknown }[] = [];
  const transport: Transport = {
    request: async ({ url, body: raw }: TransportRequest): Promise<TransportResponse> => {
      sent.push({ url, body: raw ? JSON.parse(raw) : undefined });
      return {
        status,
        statusText: "",
        url,
        text: JSON.stringify(body),
        header: () => null,
      };
    },
    // Never opened here: these cases are about the POST's answer, not the feed.
    subscribe: () => () => {},
  };
  return { transport, sent };
}

function client(status: number, body: unknown) {
  const { transport, sent } = stubTransport(status, body);
  return {
    client: new OpenCompanyClient(
      { baseUrl: "", company: null, operatorToken: null, sessionHeader: null },
      transport,
    ),
    sent,
  };
}

describe("the chat POST discriminates on the response, not on the request", () => {
  it("reads an accepted turn when the body carries `detached: true`", async () => {
    const { client: c } = client(202, {
      turnId: "turn-1",
      messageId: "42",
      detached: true,
    });

    const answer: ChatPostResult = await postDetached(c, "do the long thing");

    expect(isDetachedChat(answer)).toBe(true);
    if (!isDetachedChat(answer)) throw new Error("unreachable");
    expect(answer.turnId).toBe("turn-1");
    // Known at accept time — this is what the optimistic bubble reconciles to,
    // and the reason it no longer has to wait for the whole turn.
    expect(answer.messageId).toBe("42");
  });

  it("reads a settled turn when the body carries no discriminator", async () => {
    const { client: c } = client(200, {
      responses: [{ text: "done", channel: "main" }],
      messageId: "42",
      turnId: "turn-1",
    });

    const answer = await c.chat("hi", null, "main");

    expect(isDetachedChat(answer)).toBe(false);
    if (isDetachedChat(answer)) throw new Error("unreachable");
    expect(answer.responses.map((r) => r.text)).toEqual(["done"]);
    // Additive on the synchronous shape too, so a caller that never detached can
    // still read the turn back afterwards.
    expect(answer.turnId).toBe("turn-1");
  });

  /**
   * The compatibility case that makes shape-discrimination mandatory rather than
   * merely tidy: a host predating the field ignores `detach` (there is no
   * `deny_unknown_fields`) and answers the full synchronous body. A console that
   * decided from what it *asked* would sit waiting for a reply it already held.
   */
  it("reads a settled turn even when detach was requested and silently ignored", async () => {
    const { client: c, sent } = client(200, {
      responses: [{ text: "old host", channel: "main" }],
      messageId: "7",
    });

    const answer = await postDetached(c, "hi");

    expect(sent[0].body).toMatchObject({ detach: true });
    expect(isDetachedChat(answer)).toBe(false);
  });

  it("omits `detach` entirely when it was not asked for", async () => {
    const { client: c, sent } = client(200, { responses: [] });

    await c.chat("hi", null, "main");

    // The body shape an older host has always received, byte for byte.
    expect(sent[0].body).toEqual({ text: "hi", chat: "main" });
    expect(sent[0].body).not.toHaveProperty("detach");
  });
});

/**
 * The live-reply suppression rule — the single highest-risk line in the console
 * half, and the one that fails invisibly in both directions.
 */
describe("live-reply suppression", () => {
  it("suppresses the echo while a synchronous post is in flight", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");

    // The awaited POST is going to deliver this reply itself, so the live frame
    // is a duplicate. Rendering it doubles the bubble.
    expect(pending.suppressesLiveReply("main")).toBe(true);
  });

  it("does NOT suppress once the post detached, though the turn is still running", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.detached("main");

    // The whole point of the conditional. In detached mode this frame IS the
    // answer — suppressing it means the reply never appears at all, which is
    // strictly worse than the double bubble the rule exists to prevent. Note
    // that nothing has ended the turn: `detached` is not `ended`.
    expect(pending.suppressesLiveReply("main")).toBe(false);
  });

  it("stops suppressing when a synchronous post resolves", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.ended("main");

    expect(pending.suppressesLiveReply("main")).toBe(false);
  });

  it("suppresses per thread, so a detached turn does not unmute a synchronous one", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.started("design");
    pending.detached("design");

    // Two conversations in flight at once is the ordinary case on a busy
    // company, and the mode is a property of each POST, not of the console.
    expect(pending.suppressesLiveReply("main")).toBe(true);
    expect(pending.suppressesLiveReply("design")).toBe(false);
  });

  it("never suppresses a thread this console did not post on", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");

    // An inbound Telegram turn, a background desk turn, another operator's
    // message: nothing here is a duplicate of anything, so it all renders.
    expect(pending.suppressesLiveReply("ops")).toBe(false);
  });
});

/**
 * The race issue #1000 closes: a detached turn's own `agent_reply` beating the
 * `202` back to the browser. `onSendStart` arms suppression before the POST's
 * shape is known, and a fast enough turn's frame can land before
 * `onSendDetached` ever fires to lift it. The old boolean dropped that frame
 * outright — a silent, permanent loss of the only reply the operator was going
 * to get. `capture` holds it instead, and `detached`/`ended`/`failed` resolve
 * it for good once the POST's shape is known — dedupe by identity (which
 * thread, what the POST turned out to be), never by how long the frame waited.
 *
 * Three outcomes, not two: `ended` may discard because the awaited body already
 * rendered that reply, while `detached` and `failed` must release because
 * nothing did. See `PendingSyncPosts`' own doc for the table.
 */
describe("live-reply capture — the frame that beats the 202 home", () => {
  it("holds a frame that arrives before the POST's shape is known", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");

    // `capture` returning true means "held — do not render this yet", the
    // caller's signal to skip its own render for this frame.
    expect(pending.capture({ chatId: "main" })).toBe(true);
  });

  it("hands back a held frame, in order, once the turn detaches", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    const first = { chatId: "main", seq: 1 };
    const second = { chatId: "main", seq: 2 };
    pending.capture(first);
    pending.capture(second);

    // Nothing here was ever going to arrive twice: it is the same frame that
    // would have been dropped before, now returned so the caller can render it
    // for the first and only time.
    expect(pending.detached("main")).toEqual([first, second]);
  });

  it("discards held frames once the post turns out to be synchronous", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.capture({ chatId: "main" });

    // The awaited POST is about to render this same reply directly — a held
    // frame here is the echo the suppression exists to drop, not a reply that
    // would otherwise be lost.
    pending.ended("main");
    // Nothing left to leak into a later turn on the same thread.
    pending.started("main");
    expect(pending.detached("main")).toEqual([]);
  });

  /**
   * The outcome the two-way split got wrong, and the one this whole change is
   * for. A POST that *threw* rendered nothing, so a held frame is not a
   * duplicate of anything — and the turn behind it is very likely still
   * running, because the request dying is not the work dying. That is the
   * premise of issue #983 in one sentence, so routing the throw through `ended`
   * put the original silent-loss bug back on the exact path the feature exists
   * to serve.
   */
  it("hands back held frames when the post threw, because nothing rendered them", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    const frame = { chatId: "main" };
    pending.capture(frame);

    // The gateway cut the response at 120s; the host is still running the turn
    // and its reply is on the stream. This frame is the only copy the console
    // will ever be handed.
    expect(pending.failed("main")).toEqual([frame]);
  });

  it("hands back a failed post's held frames in arrival order", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    const first = { chatId: "main", seq: 1 };
    const second = { chatId: "main", seq: 2 };
    pending.capture(first);
    pending.capture(second);

    expect(pending.failed("main")).toEqual([first, second]);
  });

  it("lifts suppression when the post threw, so the stream takes over", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.failed("main");

    // Same reason `detached` lifts it: from here the POST is not going to
    // deliver anything, so a live frame is the answer rather than an echo of
    // one. A thread left suppressed after a throw swallows every later frame
    // of a turn that is still running.
    expect(pending.suppressesLiveReply("main")).toBe(false);
  });

  it("leaves nothing behind for the next post on a thread that failed", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.capture({ chatId: "main" });
    pending.failed("main");

    // Released, not merely handed out: the operator retries, and the previous
    // turn's frame must not surface inside the retry's window.
    pending.started("main");
    expect(pending.detached("main")).toEqual([]);
  });

  it("fails one thread without disturbing a sibling still in flight", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.started("design");
    const mainFrame = { chatId: "main" };
    const designFrame = { chatId: "design" };
    pending.capture(mainFrame);
    pending.capture(designFrame);

    expect(pending.failed("main")).toEqual([mainFrame]);
    expect(pending.suppressesLiveReply("design")).toBe(true);
    expect(pending.detached("design")).toEqual([designFrame]);
  });

  /**
   * The three outcomes side by side, which is the assertion that would have
   * failed on the code this replaces: `ended` and `failed` were the same call.
   */
  it("discards on a resolved post and releases on a failed one, from the same state", () => {
    const resolved = new PendingSyncPosts();
    resolved.started("main");
    resolved.capture({ chatId: "main" });
    resolved.ended("main");
    resolved.started("main");
    expect(resolved.detached("main")).toEqual([]);

    const threw = new PendingSyncPosts();
    threw.started("main");
    const frame = { chatId: "main" };
    threw.capture(frame);
    expect(threw.failed("main")).toEqual([frame]);
  });

  it("captures nothing for a thread with no post in flight", () => {
    const pending = new PendingSyncPosts();

    // Same rule `suppressesLiveReply` already pins for this case: an inbound
    // frame this console did not post for is never this console's to hold.
    expect(pending.capture({ chatId: "ops" })).toBe(false);
  });

  it("keeps each thread's held frames apart from a sibling's", () => {
    const pending = new PendingSyncPosts();
    pending.started("main");
    pending.started("design");
    const mainFrame = { chatId: "main" };
    const designFrame = { chatId: "design" };
    pending.capture(mainFrame);
    pending.capture(designFrame);

    expect(pending.detached("design")).toEqual([designFrame]);
    // `main` is still mid-flight — its own held frame must not have travelled
    // with `design`'s.
    expect(pending.detached("main")).toEqual([mainFrame]);
  });
});

/**
 * Re-arming the working indicator after a reload — the leg that was impossible
 * before the turn had a durable row, and the one that proves the whole design.
 */
describe("open turns read back from the run store", () => {
  it("re-arms the indicator on the thread that raised the turn", () => {
    const open = openTurnsFromRuns([
      { id: "turn-1", chatId: "main", status: "running" },
    ]);

    // `chatId` rides alongside since #2042's follow-up: the map key can be a
    // composite (`main#41`), and the settle poll needs the real desk to re-read.
    expect(open).toEqual({ main: [{ turnId: "turn-1", queued: false, chatId: "main" }] });
  });

  it("calls a turn that has not taken the lock queued, not working", () => {
    const open = openTurnsFromRuns([
      { id: "turn-1", chatId: "main", status: "pending" },
    ]);

    // The serial train is real — five concurrent messages queue behind one
    // another — and a spinner implying progress while a turn waits its turn is
    // the console saying something untrue.
    expect(open.main[0].queued).toBe(true);
  });

  it("ignores a dispatch at a card, which owns no conversation's indicator", () => {
    const open = openTurnsFromRuns([
      { id: "run-9", status: "running" },
      { id: "turn-1", chatId: "main", status: "running" },
    ]);

    expect(Object.keys(open)).toEqual(["main"]);
  });

  it("re-arms each conversation independently", () => {
    const open = openTurnsFromRuns([
      { id: "turn-1", chatId: "main", status: "running" },
      { id: "turn-2", chatId: "design", status: "pending" },
    ]);

    expect(open.main[0].queued).toBe(false);
    expect(open.design[0].queued).toBe(true);
  });

  it("orders a thread's rows running-first yet keeps the queued sibling", () => {
    // The per-company serial lock lets one thread hold a running turn and a
    // queued one at once. The running row is what the operator is waiting on,
    // so it must head the list whichever order the store listed them in — but
    // the queued sibling is NOT dropped, because its reply is still coming and
    // the poll watches it too (issue #1000).
    const runningFirst = openTurnsFromRuns([
      { id: "turn-1", chatId: "main", status: "running" },
      { id: "turn-2", chatId: "main", status: "pending" },
    ]);
    const pendingFirst = openTurnsFromRuns([
      { id: "turn-2", chatId: "main", status: "pending" },
      { id: "turn-1", chatId: "main", status: "running" },
    ]);

    const expected = {
      main: [
        { turnId: "turn-1", queued: false, chatId: "main" },
        { turnId: "turn-2", queued: true, chatId: "main" },
      ],
    };
    expect(runningFirst).toEqual(expected);
    expect(pendingFirst).toEqual(expected);
  });

  it("keeps two queued rows in store order", () => {
    // No running turn yet — two sends queued behind a long first turn. Both
    // are still awaited, so both stay, in the order the store listed them.
    const open = openTurnsFromRuns([
      { id: "turn-1", chatId: "main", status: "pending" },
      { id: "turn-2", chatId: "main", status: "pending" },
    ]);

    expect(open.main.map((t) => t.turnId)).toEqual(["turn-1", "turn-2"]);
    expect(open.main.every((t) => t.queued)).toBe(true);
  });
});

/**
 * The merge that arms the map without evicting a turn another leg registered.
 * `onSendDetached` appends from a POST's answer while a reload (or a failed
 * POST's re-query) arms from `listRuns`; the two collide on the same turn.
 */
describe("merging armed turn lists", () => {
  it("appends a thread's turns beside an existing row", () => {
    const before = { main: [{ turnId: "turn-1", queued: false, chatId: "main" }] };
    const merged = mergeOpenTurns(before, {
      main: [{ turnId: "turn-2", queued: true, chatId: "main" }],
    });

    expect(merged.main.map((t) => t.turnId)).toEqual(["turn-1", "turn-2"]);
    // A copy, never a mutation of the previous state.
    expect(before.main).toEqual([{ turnId: "turn-1", queued: false, chatId: "main" }]);
  });

  it("collapses the same turn onto one entry, fresh reading wins", () => {
    // A reload mid-POST arms the same row the 202 just registered. One entry
    // survives, and the store's later answer is the honest `queued` reading.
    const merged = mergeOpenTurns(
      { main: [{ turnId: "turn-1", queued: true, chatId: "main" }] },
      { main: [{ turnId: "turn-1", queued: false, chatId: "main" }] },
    );

    expect(merged.main).toEqual([{ turnId: "turn-1", queued: false, chatId: "main" }]);
  });

  it("keeps id-less turns as separate entries", () => {
    const merged = mergeOpenTurns(
      { main: [{ queued: true, chatId: "main" }] },
      { main: [{ queued: false, chatId: "main" }, { queued: true, chatId: "main" }] },
    );

    expect(merged.main).toEqual([
      { queued: true, chatId: "main" },
      { queued: false, chatId: "main" },
      { queued: true, chatId: "main" },
    ]);
  });

  it("never evicts an existing row a re-arm does not repeat", () => {
    // The re-arm came back listing only the newer turn; the running row the
    // POST leg registered must survive the merge, not be replaced by it.
    const merged = mergeOpenTurns(
      { main: [{ turnId: "turn-1", queued: false, chatId: "main" }] },
      { main: [{ turnId: "turn-2", queued: true, chatId: "main" }] },
    );

    expect(merged.main.map((t) => t.turnId)).toEqual(["turn-1", "turn-2"]);
    expect(merged.design).toBeUndefined();
  });

  it("leaves untouched threads alone", () => {
    const merged = mergeOpenTurns(
      { design: [{ turnId: "turn-x", queued: true, chatId: "main" }] },
      { main: [{ turnId: "turn-1", queued: false, chatId: "main" }] },
    );

    expect(merged.design).toEqual([{ turnId: "turn-x", queued: true, chatId: "main" }]);
  });
});

/**
 * The id reconciliation at the 202 call site.
 *
 * Strictly better than the synchronous path it replaces: since the message is
 * journaled at accept time, the durable id is a fact within milliseconds rather
 * than after the whole turn — so the operator's own bubble becomes replyable and
 * reactable immediately instead of at settle.
 */
describe("reconciling the optimistic id from a 202", () => {
  it("swaps the local id for the durable one the accept already minted", async () => {
    const { client: c } = client(202, {
      turnId: "turn-1",
      messageId: "42",
      detached: true,
    });

    const answer = await postDetached(c, "do the long thing");
    if (!isDetachedChat(answer)) throw new Error("expected the accepted shape");

    // What `ChatView.send` does with it, before it branches on the shape at all.
    const before = [{ id: "m1", from: "you", text: "do the long thing", at: 1_000 } as ChatMessage];
    const after = reconcileIds(before, "m1", answer.messageId);

    expect(after[0].id).toBe("h42");
  });

  it("carries a reply typed against the optimistic bubble across the swap", async () => {
    const { client: c } = client(202, { turnId: "t", messageId: "42", detached: true });
    const answer = await postDetached(c, "the plan");
    if (!isDetachedChat(answer)) throw new Error("expected the accepted shape");

    const before = [
      { id: "m1", from: "you", text: "the plan", at: 1_000 } as ChatMessage,
      { id: "m2", from: "you", text: "and this", at: 1_001, parentId: "m1" } as ChatMessage,
    ];
    const after = reconcileIds(before, "m1", answer.messageId);

    // The race the immediate reconciliation makes shorter, not longer: the
    // operator opened a thread on their own bubble while the turn ran.
    expect(after[1].parentId).toBe("h42");
  });
});
