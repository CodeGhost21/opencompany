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
export class PendingSyncPosts {
  private readonly threads = new Set<string>();

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
   */
  detached(threadId: string): void {
    this.threads.delete(threadId);
  }

  /** The synchronous POST resolved (or threw); its reply is already rendered. */
  ended(threadId: string): void {
    this.threads.delete(threadId);
  }

  /** Whether a live `agent_reply` for this thread would be a duplicate. */
  suppressesLiveReply(threadId: string): boolean {
    return this.threads.has(threadId);
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
