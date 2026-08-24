// Where a run came from (issue #1573).
//
// A teammate's run history is a list of attempts, and an attempt on its own
// says almost nothing: "succeeded, 41s, 12 steps" is not an answer to "what was
// it doing". What makes the list readable is the *provenance* — this one worked
// a card, that one answered a message in #front-desk, that one was a step in
// the nightly digest workflow.
//
// # None of that is on the run record, and that is deliberate
//
// A `RunSummary` carries a `taskId` **or** a `chatId` and nothing else, because
// those are the only two things the dispatch path knows at the moment it mints
// the row. Everything richer — the card's title, the workflow that opened the
// card, the channel's display name — belongs to surfaces that outlive, and are
// edited independently of, the attempt.
//
// So the source is *resolved*, from lists the console already reads, rather
// than denormalised onto the run at write time. That has one property worth
// stating plainly: **it degrades**. A card that has been deleted, a workflow
// that has been retired, a host too old to send `originWorkflowId` — each of
// those lands here as a run whose source is known only by id. This module says
// so in those cases instead of guessing, because a confident wrong provenance
// ("Card: Draft the Q3 memo") on an attempt that was actually something else is
// worse than an honest `card-8f2a`.
//
// # Why a workflow beats a card
//
// A card opened by a workflow run is both a card and a workflow step. The
// workflow wins the headline because it is the thing that *decided* the work —
// the card is the mechanism it used, and it is still named, on the second line.
// The reverse ordering would file every scheduled run under a card title nobody
// wrote and hide the schedule that is actually generating them.

import type { RunSummary } from "@/api/runs";
import type { Task } from "@/api/tasks";
import { cardHref, workflowHref } from "@/lib/task-output";

/**
 * What kind of thing set an attempt going.
 *
 * `unknown` is a real, reachable case rather than a defensive default: a run
 * row carries neither handle when the dispatch that minted it died before it
 * could name one. Such a run is still an honest record of an attempt, so it is
 * listed — labelled as unattributed rather than hidden.
 */
export type RunSourceKind = "workflow" | "card" | "chat" | "unknown";

/** A run's provenance, resolved for display. */
export interface RunSource {
  kind: RunSourceKind;
  /** The headline — a workflow name, a card title, a channel name, or an id. */
  label: string;
  /** The second line, when the headline is not the whole story. */
  detail?: string;
  /** Where to go to see it, as a console hash address. */
  href?: string;
  /**
   * Whether `label` is a real name or a bare identifier standing in for one.
   *
   * The render leans on this: an id is set in a mono face and never dressed up
   * as a title, so "this is the card `card-8f2a`, whose title I could not find"
   * never reads as "this is the card *named* card-8f2a".
   */
  resolved: boolean;
}

/** The lists a resolution reads. All optional — a missing one degrades. */
export interface RunSourceIndex {
  /** The board, by card id. */
  tasks?: ReadonlyMap<string, Task>;
  /** Workflow display names, by workflow id. */
  workflows?: ReadonlyMap<string, string>;
  /** Channel/desk display names, by chat id. */
  chats?: ReadonlyMap<string, string>;
}

/** `#/chat/<chatId>` — the thread a turn was raised in. */
export function chatHref(chatId: string, directMessage = false): string {
  return `#/chat/${directMessage ? "dm:" : ""}${encodeURIComponent(chatId)}`;
}

/**
 * Resolve one run's provenance against the lists in `index`.
 *
 * Pure, and total: every run gets a source, including one whose card, workflow
 * and channel are all unknown.
 */
export function runSource(run: RunSummary, index: RunSourceIndex = {}): RunSource {
  if (run.taskId !== undefined) {
    const card = index.tasks?.get(run.taskId);
    const workflowId = card?.originWorkflowId;
    if (card && workflowId !== undefined) {
      const name = index.workflows?.get(workflowId);
      return {
        kind: "workflow",
        label: name ?? workflowId,
        // The card is named on the second line rather than dropped: it is where
        // the run's output actually landed, and an operator following up wants
        // it. `card.title` is present by construction here — `card` is what
        // carried `originWorkflowId`.
        detail: card.title,
        // Deep-linked to the run that opened the card where the host recorded
        // one, so the canvas shows what ran rather than only what the graph
        // says now.
        href: workflowHref(workflowId, card.originRunId),
        resolved: name !== undefined,
      };
    }
    if (card) {
      return {
        kind: "card",
        label: card.title,
        detail: card.originChatId ? "Opened from a conversation" : undefined,
        href: cardHref(run.taskId),
        resolved: true,
      };
    }
    return {
      kind: "card",
      label: run.taskId,
      // Two different absences, and the wording covers both without claiming
      // either: the card may have been deleted, or the board read may simply
      // not have landed. Neither is a reason to withhold the attempt.
      detail: "This card is no longer on the board.",
      href: cardHref(run.taskId),
      resolved: false,
    };
  }

  if (run.chatId !== undefined) {
    const name = index.chats?.get(run.chatId);
    return {
      kind: "chat",
      label: name ?? run.chatId,
      // A desk's channel id *is* its thread id, so a run in a desk channel
      // carries `chatId == agentId == desk` and `index.chats` names it; a DM's
      // thread id is the roster member's id, which no desk claims. So the
      // "is it a DM" answer is "not a known desk", read from the index rather
      // than guessed from the record — `agentId === chatId` is true for a
      // desk-channel run too, and stamping `dm:` on one would link
      // `#/chat/dm:engineering`, which ChatView treats as an unknown channel
      // (issue #1671).
      href: chatHref(run.chatId, name === undefined),
      resolved: name !== undefined,
    };
  }

  return {
    kind: "unknown",
    label: "No source recorded",
    detail:
      "The dispatch that opened this attempt never named a card or a conversation.",
    resolved: false,
  };
}

/** The word for a source kind, as the chip renders it. */
export const RUN_SOURCE_LABEL: Record<RunSourceKind, string> = {
  workflow: "Workflow",
  card: "Card",
  chat: "Chat",
  unknown: "Unattributed",
};
