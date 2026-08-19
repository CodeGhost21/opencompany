// Whose live `agent_reply` frame the shell must drop, and whose it must render.
//
// Extracted from `AppShell` because it is the single highest-risk rule in the
// detached-chat change (issue #983) and it fails in opposite, equally invisible
// ways. Suppress when you should not and the operator's reply never appears at
// all; render when you should not and every reply appears twice. Neither throws,
// neither shows up in a type, and both look like "the chat is behaving oddly".
// As a closure over a ref it could only be exercised by driving the whole shell;
// as a rule with a name it can be asserted transition by transition.

/**
 * The one field {@link PendingSyncPosts.capture} needs off a live `agent_reply`
 * frame. A local shape rather than importing the hook's event type, matching
 * {@link OpenTurnRow} / {@link OpenRunRow} below — this module stays testable
 * without dragging in `hooks/use-events`.
 */
export interface LiveReplyFrame {
  chatId: string;
}

/**
 * The threads with a **synchronous** chat POST in flight.
 *
 * ## The rule, and why it is not "a turn is running"
 *
 * The host journals an `AgentReply` for the operator's own turn too, and pushes
 * it over SSE. When the console is *awaiting* that turn's POST, the awaited
 * response is the authoritative copy (it carries the folded step timeline), and
 * the live frame is a duplicate that would double the bubble — so it is dropped.
 *
 * A **detached** turn inverts that. Its POST answered `202` immediately and is
 * never going to carry a reply, so the live frame is not a duplicate of the
 * answer — it *is* the answer. Suppressing it there means the reply never
 * arrives on screen, which is a strictly worse failure than the double bubble
 * this rule exists to prevent.
 *
 * So membership means "a POST is in flight that will itself deliver the reply",
 * which is why {@link detached} removes a thread the moment the `202` lands even
 * though its turn is still very much running.
 *
 * ## A POST has three outcomes, not two
 *
 * A frame held by {@link capture} is disposed of by whichever outcome the POST
 * reaches, and each outcome disposes of it differently *because the question is
 * always the same one*: did anything else already put this reply on screen?
 *
 * | Outcome | Reached by | Held frames | Because |
 * |---|---|---|---|
 * | **resolved** | {@link ended} | discarded | the awaited body carried the reply and the caller has already rendered it, folded steps and all — a held frame is that same reply a second time |
 * | **detached** | {@link detached} | released | the `202` carried no reply and never will; the stream is the delivery path, so the held frame is the *only* copy of the answer |
 * | **failed** | {@link failed} | released | the POST threw, so nothing was rendered — and the turn is very likely still running on the host, which makes the stream the only delivery path here too |
 *
 * The third row is the one that is easy to miss and the reason it exists at all
 * (issue #1000). A throw is not a settled turn: the request is what died, not
 * the work. The gateway cutting a 120s request, a laptop losing its network, the
 * tenant proxy timing out — the host keeps running the turn and journals its
 * reply, which arrives over SSE like any other. Routing that case through
 * {@link ended} discards a frame that nothing else will ever render, which is
 * the exact silent loss this class was extracted to prevent, merely moved onto
 * the error path — and the error path is the one the detached design exists for.
 *
 * So the split is by *what happened to the POST*, never by widening `ended` to
 * mean "the POST is over". Two of the three outcomes leave a turn running.
 *
 * ## The fourth case: a POST that never settles
 *
 * A request that neither resolves nor throws — no response headers, no error,
 * still pending minutes later — reaches none of the three methods above. Its
 * thread stays suppressed and its held frames stay held, so a reply that
 * arrives in that window is not rendered until something else puts it on screen
 * (the turn's terminal `chat/history` re-read, or the next hydration of the
 * channel).
 *
 * This is written down rather than fixed on purpose. It is **not** a
 * regression: the code before this class dropped such a frame outright, so the
 * outcome on screen is identical and the durable record is unchanged. And the
 * fix that suggests itself — expire the hold after N seconds — is precisely the
 * window-based reasoning this class replaced. A timer cannot tell a hung POST
 * from a slow one, so it would reintroduce, on a timer's schedule, the drop
 * that holding by identity removed. If this case ever needs closing, close it
 * with a fact rather than a duration: the request's own `AbortSignal` firing,
 * which is an outcome and belongs in {@link failed}.
 *
 * It is reachable in production — the hosting manager's proxy buffers a whole
 * upstream body before it will build a response, which is why `/events` hangs
 * past two minutes on a hosted tenant (`opencompany-microservice#23`).
 */
export class PendingSyncPosts<F extends LiveReplyFrame = LiveReplyFrame> {
  private readonly threads = new Set<string>();
  /**
   * Frames {@link capture} held back because their thread's POST shape was
   * still unknown when they arrived, keyed by thread and kept in arrival
   * order. Resolved for good — never left to expire — by whichever of
   * {@link ended} / {@link detached} / {@link failed} the POST turns out to
   * reach.
   */
  private readonly held = new Map<string, F[]>();

  /**
   * A chat POST has gone out on this thread.
   *
   * Suppression starts here, before the shape of the answer is known, and that
   * is the safe default in both directions: a synchronous turn's echo arrives
   * mid-await and must be dropped, while a detached turn's `202` comes back in
   * milliseconds and lifts the suppression long before its reply is written.
   */
  started(threadId: string): void {
    this.threads.add(threadId);
  }

  /**
   * The host answered `202`: accepted, not answered (issue #983).
   *
   * The turn is still running — this is emphatically not `ended` — but this POST
   * has stopped being the delivery path, so the stream takes over.
   *
   * Returns whatever {@link capture} held for this thread, oldest first, so the
   * caller can render it now. This is the fix for the race the boolean alone
   * could not close: `onSendStart` arms suppression synchronously, but nothing
   * makes the browser learn the `202`'s shape before a fast turn's SSE frame
   * already arrived — a detached echo brain can answer in milliseconds, well
   * inside the round trip. A frame landing in that window used to be dropped
   * outright, which is a silent, permanent loss of the operator's only reply.
   * Holding it and handing it back here — identity by thread, not a timer —
   * closes the window instead of narrowing it: no frame captured while a
   * thread's shape was unknown is ever thrown away, no matter when it lands
   * relative to the `202`.
   */
  detached(threadId: string): F[] {
    return this.release(threadId);
  }

  /**
   * The chat POST **threw**: no response body, nothing rendered (issue #1000).
   *
   * Not {@link ended}, and the distinction is the whole point of this method.
   * `ended` may discard held frames because the awaited body already put that
   * reply on screen; a throw put nothing on screen, so there is no copy for a
   * held frame to duplicate. Discarding here loses the reply outright.
   *
   * And the turn is very likely still running. That is the premise of the whole
   * detached design (issue #983): the work outlives the request, so a gateway
   * cutting the connection at 120s kills the response and nothing else — the
   * host finishes the turn and journals the reply, which arrives over SSE like
   * any other. From the moment the request dies the stream is the only delivery
   * path this console has, exactly as it is after a `202`.
   *
   * So this behaves like {@link detached}: hand back whatever {@link capture}
   * held for this thread, oldest first, and lift the suppression. The caller
   * still tells the operator the request failed — a reply that lands later does
   * not make the failure untrue, and the two facts are not in competition.
   */
  failed(threadId: string): F[] {
    return this.release(threadId);
  }

  /**
   * The synchronous POST resolved with a body; its reply is already rendered.
   *
   * Whatever {@link capture} held for this thread was, by the same reasoning,
   * a live echo of that same reply — the awaited response is authoritative, so
   * the held frames are discarded rather than replayed.
   *
   * Scoped to a POST that *resolved*. A POST that threw resolved nothing and
   * belongs in {@link failed}; sending it here discards a reply the console is
   * never going to be handed again.
   */
  ended(threadId: string): void {
    this.threads.delete(threadId);
    this.held.delete(threadId);
  }

  /**
   * Lift this thread's suppression and hand back what was held for it, oldest
   * first — the shared half of {@link detached} and {@link failed}.
   *
   * The two stay separate methods rather than one `postOver` because they are
   * separate facts about the turn, and a caller that cannot say which one it
   * means is a caller that has not decided. `detached` has a turn id and a row
   * to poll; `failed` has neither, and a working row armed from it would be a
   * spinner with nothing to take it down.
   */
  private release(threadId: string): F[] {
    this.threads.delete(threadId);
    const frames = this.held.get(threadId) ?? [];
    this.held.delete(threadId);
    return frames;
  }

  /** Whether a live `agent_reply` for this thread would be a duplicate. */
  suppressesLiveReply(threadId: string): boolean {
    return this.threads.has(threadId);
  }

  /**
   * Route one live `agent_reply` frame: render it now, or hold it because this
   * thread's POST has not yet told the console what it delivers.
   *
   * Returns `true` when the frame was held — the caller must not render it,
   * whichever of {@link ended} / {@link detached} / {@link failed} the POST
   * reaches will dispose of it — and `false` when
   * there is nothing pending on this thread and the frame is the caller's to
   * render immediately, same as before this thread ever posted.
   *
   * This is the only place a frame's fate is decided, and it decides by
   * identity — is this thread's POST still unresolved — never by how long it
   * has been unresolved. See {@link detached} for why that distinction is the
   * whole fix.
   */
  capture(frame: F): boolean {
    if (!this.suppressesLiveReply(frame.chatId)) return false;
    const queue = this.held.get(frame.chatId);
    if (queue) queue.push(frame);
    else this.held.set(frame.chatId, [frame]);
    return true;
  }
}

/**
 * One chat turn that has been accepted but has not settled (issue #983).
 *
 * The shell holds these **per thread, as an ordered list** — a thread can have
 * a running turn and a queued one behind it at once (the per-company serial
 * lock admits exactly that), and every one of them has a reply the operator is
 * eventually shown. The first entry is the one the working indicator reads
 * where it renders; the poll watches the whole list and drains oldest-first.
 * Declared here rather than in the shell so the fold below and the views can
 * share it without a type-cycle.
 */
export type OpenTurn = {
  /**
   * The durable row to poll. Absent when the host could not mint one — the turn
   * still ran, and the indicator still shows, it just cannot be watched.
   */
  turnId?: string;
  /**
   * The row is still `pending`: accepted, but waiting on the per-company serial
   * lock rather than working. Drives the indicator's wording.
   */
  queued: boolean;
};

/** The per-thread rows the fold produces — the same shape as {@link OpenTurn}. */
export type OpenTurnRow = OpenTurn;

/** The shape {@link openTurnsFromRuns} reads — the run rows' relevant fields. */
export interface OpenRunRow {
  id: string;
  chatId?: string;
  status: string;
}

/**
 * Folds the open run rows into the per-thread turns the working indicator reads
 * (issue #983).
 *
 * This is the reload leg, and it is the thing that was impossible before the
 * turn became durable: a console that never saw the POST asks which turns are
 * open and re-arms the indicator from the answer, instead of showing a
 * settled-looking transcript with an answer still on its way.
 *
 * Two rules earn their own assertions. A run at a **card** is a dispatch, not a
 * chat turn, and owns no thread's indicator — so a row with no conversation is
 * skipped rather than defaulted somewhere. And `pending` versus `running` is
 * carried through rather than flattened, because "queued behind other turns" and
 * "working" are different things to tell an operator, and the serial lock makes
 * the first one common.
 */
export function openTurnsFromRuns(runs: readonly OpenRunRow[]): Record<string, OpenTurnRow> {
  const open: Record<string, OpenTurnRow> = {};
  for (const run of runs) {
    if (!run.chatId) continue;
    const queued = run.status === "pending";
    // A thread can hold a running turn and a queued one behind it. The running
    // one is what the operator is waiting on, so it names the row — otherwise
    // the answer would depend on the order the store listed the rows in.
    if (open[run.chatId] && !open[run.chatId].queued) continue;
    open[run.chatId] = { turnId: run.id, queued };
  }
  return open;
}
