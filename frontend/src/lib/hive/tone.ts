/**
 * How the deliberation vocabulary is rendered.
 *
 * Pure — a projection from a move or an ending to the label, the icon name and
 * the class strings that draw it — so every visual decision here is unit-testable
 * in the node lane with no React tree.
 *
 * # Three rules this file exists to keep
 *
 * **A move is told apart by its shape, never by its colour.** Roughly one in
 * twelve men cannot separate a red/green pair, and nine move kinds is far past
 * what any palette separates honestly anyway. So every move chip takes the same
 * neutral treatment and differs by icon and label. That also means this file
 * introduces **no new colour token**, which keeps `docs/design-system/color.md`'s
 * cleared-debt grep green.
 *
 * **An ending borrows the closed status vocabulary and never extends it.** The
 * five states — idle, running, blocked, done, failed — carry measured contrast
 * ratios; a sixth hue invented for "deadlocked" would carry none.
 *
 * **A topic is a third form.** Identity is a tile with initials, status is a pill
 * with a dot, and neither may be reused for "which option is this" — the design
 * system's own rule is that identity and status never take the same shape, and a
 * topic is a third kind of thing again. It gets a `#`-prefixed monospace chip.
 *
 * Class strings are written out in full and never assembled from a template,
 * because Tailwind finds classes by scanning source text — a computed
 * `bg-status-${key}` is silently never generated.
 */

import type { MoveKind } from "@/lib/hive/grammar";

/** How a move is drawn: a lucide icon name, a label, and one line of help. */
export interface MoveMark {
  /** A key into the renderer's icon map — never a component, so this stays pure. */
  icon: string;
  label: string;
  /** What the move does, in the room's own terms. */
  hint: string;
}

export const MOVE_MARKS: Record<MoveKind, MoveMark> = {
  propose: {
    icon: "propose",
    label: "Proposes",
    hint: "Puts a new option on the floor. Counts as its own author's support.",
  },
  support: {
    icon: "support",
    label: "Supports",
    hint: "Backs an option, citing the message that grounds it.",
  },
  object: {
    icon: "object",
    label: "Objects",
    hint: "Silences one advocate. Local to the message it targets.",
  },
  refute: {
    icon: "refute",
    label: "Refutes",
    hint: "Argues a cited fact against the option itself, for the whole room.",
  },
  evidence: {
    icon: "evidence",
    label: "Evidence",
    hint: "Supplies grounds without taking a side.",
  },
  question: {
    icon: "question",
    label: "Asks",
    hint: "Asks for something the room has not established.",
  },
  defer: {
    icon: "defer",
    label: "Defers",
    hint: "Stands aside. Costs the turn, adds no support.",
  },
  commit: {
    icon: "commit",
    label: "Commits",
    hint: "Records the decision the room reached. Bookkeeping, not a fresh judgement.",
  },
  pin: { icon: "pin", label: "Pins", hint: "Folds a line into the desk's pinboard." },
};

/**
 * The neutral chip every move takes.
 *
 * One treatment, deliberately. See the header: the icon is what separates them.
 */
export const MOVE_CHIP_CLASS =
  "inline-flex items-center gap-1 rounded-md border border-border bg-muted/60 px-1.5 py-0.5 font-mono text-2xs leading-none text-muted-foreground";

/** The chip a `#topic` takes — the third form, shared by the rail and the rows. */
export const TOPIC_CHIP_CLASS =
  "inline-flex items-center gap-1 rounded-md border border-dashed border-border bg-background px-1.5 py-0.5 font-mono text-2xs leading-none text-foreground";

/** The tone a carried topic takes, from the closed status vocabulary. */
export const TOPIC_CARRIED_CLASS =
  "inline-flex items-center gap-1 rounded-md border border-status-done/40 bg-status-done-soft px-1.5 py-0.5 font-mono text-2xs leading-none text-status-done-text";

/** The five closed status words. Nothing here may invent a sixth. */
export type StatusTone = "idle" | "running" | "blocked" | "done" | "failed";

/** How an episode ending is drawn. */
export interface EndingMark {
  tone: StatusTone;
  label: string;
  icon: string;
  hint: string;
}

/**
 * An ending, mapped into the status vocabulary.
 *
 * `deadlocked` takes **blocked** rather than a failure tone: a room that argued
 * two options to a standstill has not gone wrong, it is waiting on somebody. And
 * `exhausted` takes **idle** rather than failed for the same reason a by-design
 * decline does — a clean terminal outcome that did not succeed is not a fault.
 */
export const ENDING_MARKS: Record<string, EndingMark> = {
  converged: {
    tone: "done",
    label: "Settled",
    icon: "commit",
    hint: "One option carried and the room recorded it.",
  },
  deadlocked: {
    tone: "blocked",
    label: "Deadlocked",
    icon: "scale",
    hint: "Two or more options carried together and nobody broke the tie.",
  },
  exhausted: {
    tone: "idle",
    label: "Budget spent",
    icon: "defer",
    hint: "The turn budget ran out before the room reached a decision.",
  },
  idle: {
    tone: "idle",
    label: "Did not open",
    icon: "defer",
    hint: "Nobody on the desk had anything to add.",
  },
};

/** Full class strings per status tone. Never assembled from a template. */
export const TONE_CLASSES: Record<StatusTone, { dot: string; text: string; soft: string }> = {
  idle: {
    dot: "bg-status-idle",
    text: "text-status-idle-text",
    soft: "bg-status-idle-soft",
  },
  running: {
    dot: "bg-status-running",
    text: "text-status-running-text",
    soft: "bg-status-running-soft",
  },
  blocked: {
    dot: "bg-status-blocked",
    text: "text-status-blocked-text",
    soft: "bg-status-blocked-soft",
  },
  done: {
    dot: "bg-status-done",
    text: "text-status-done-text",
    soft: "bg-status-done-soft",
  },
  failed: {
    dot: "bg-status-failed",
    text: "text-status-failed-text",
    soft: "bg-status-failed-soft",
  },
};

/**
 * The one-line summary of a topic's standing.
 *
 * Words rather than a bare count, because "2 of 3" is ambiguous about which
 * number is the threshold — and the threshold is the whole question.
 */
export function standingSummary(
  supporters: number,
  quorum: number,
  carried: boolean,
): string {
  if (carried) return `Carried — ${supporters} of ${quorum} needed`;
  if (supporters === 0) return `No grounded support yet — ${quorum} needed`;
  return `${supporters} of ${quorum} needed`;
}
