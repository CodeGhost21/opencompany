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
 */
/**
 * The one field {@link PendingSyncPosts.capture} needs off a live `agent_reply`
 * frame. A local shape rather than importing the hook's event type, matching
 * {@link OpenTurnRow} / {@link OpenRunRow} below — this module stays testable
 * without dragging in `hooks/use-events`.
 */
export interface LiveReplyFrame {
  chatId: string;
}

export class PendingSyncPosts {
  private readonly threads = new Set<string>();
  /**
   * Frames {@link capture} held back because their thread's POST shape was
   * still unknown when they arrived, keyed by thread and kept in arrival
   * order. Resolved for good — never left to expire — by whichever of
   * {@link ended} / {@link detached} the POST turns out to reach.
   */
  private readonly held = new Map<string, LiveReplyFrame[]>();

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
  detached(threadId: string): LiveReplyFrame[] {
    this.threads.delete(threadId);
    const frames = this.held.get(threadId) ?? [];
    this.held.delete(threadId);
    return frames;
  }

  /**
   * The synchronous POST resolved (or threw); its reply is already rendered.
   *
   * Whatever {@link capture} held for this thread was, by the same reasoning,
   * a live echo of that same reply — the awaited response is authoritative, so
   * the held frames are discarded rather than replayed.
   */
  ended(threadId: string): void {
    this.threads.delete(threadId);
    this.held.delete(threadId);
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
   * {@link detached} or {@link ended} will dispose of it — and `false` when
   * there is nothing pending on this thread and the frame is the caller's to
   * render immediately, same as before this thread ever posted.
   *
   * This is the only place a frame's fate is decided, and it decides by
   * identity — is this thread's POST still unresolved — never by how long it
   * has been unresolved. See {@link detached} for why that distinction is the
   * whole fix.
   */
  capture(frame: LiveReplyFrame): boolean {
    if (!this.suppressesLiveReply(frame.chatId)) return false;
    const queue = this.held.get(frame.chatId);
    if (queue) queue.push(frame);
    else this.held.set(frame.chatId, [frame]);
    return true;
  }
}

/**
 * A chat turn accepted but not settled, keyed by the thread it belongs to.
 *
 * Mirrors `OpenTurn` in the shell, declared here so the fold below can be tested
 * without standing up a React tree.
 */
export interface OpenTurnRow {
  turnId?: string;
  queued: boolean;
}

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
    open[run.chatId] = { turnId: run.id, queued: run.status === "pending" };
  }
  return open;
}
