/**
 * The hive-mind move grammar, client-side.
 *
 * A deliberating desk answers an operator message as a room, and every turn in
 * that room opens its line with a **marker**:
 *
 * ```text
 * !<kind> [#topic] [>target] [^cite ...] free text
 * ```
 *
 * This module is the console's reader for that grammar, plus the two derivations
 * an operator needs to see (`turn_budget`, `quorum`) and the per-seat gating
 * table. It is **pure** — no React, no fetch — because every interesting case
 * here is a string case, and the unit lane runs in a node environment with no
 * React plugin (`vitest.config.ts`).
 *
 * # Why the console parses at all
 *
 * The host journals only the marker line (`docs/spec/runtime/hivemind.md`,
 * "What lands in the journal"), and nothing on the wire says "this reply was a
 * move". Until the episode frames land, reading `AgentReply.text` is the only
 * way the console can tell a deliberation from a conversation. Everything
 * derived this way is marked `derived` by its caller and rendered with lower
 * confidence — see `episode.ts`.
 *
 * # The one rule that is easy to get wrong
 *
 * A marker is recognised **at the start of a line, outside fenced code blocks**.
 * Miss the fence exclusion and a code sample containing `!propose` becomes a
 * proposal the room never made. {@link markerLines} is the only place that knows
 * this, and it has its own test table.
 *
 * Mirrors `src/hivemind/moves.rs` and `vendor/tinyhivemind/crates/
 * tinyhivemind-hive/src/trace/`. Where the two disagree, the host wins and this
 * file is the bug.
 */

/**
 * Every kind a `[group_chat.hive].moves` entry may name.
 *
 * Mirrors `MOVE_KINDS` in `src/hivemind/moves.rs`, in the same order — the host
 * renders a seat's moves in this order, so rendering them in another one here
 * would make the console and the prompt disagree about the same table.
 */
export const MOVE_KINDS = [
  "propose",
  "support",
  "object",
  "refute",
  "evidence",
  "question",
  "defer",
  "commit",
  "pin",
] as const;

export type MoveKind = (typeof MOVE_KINDS)[number];

/**
 * The kinds no `moves` table can take away from a seat.
 *
 * `commit` is bookkeeping rather than authorship: the fold picks the speaker in
 * the Commit phase, so a desk that could bar a seat from recording a decision
 * reaches quorum and then hands the floor to somebody with nothing legal to say.
 * That is not hypothetical — a six-member desk did it for eight turns and then
 * reported itself exhausted on an answer it had already carried.
 *
 * `question` and `defer` are the two honest things a member with nothing to add
 * can say. A seat barred from both has only silence or a guess left.
 */
export const UNGATED_KINDS: readonly MoveKind[] = ["question", "defer", "commit"];

/** The six kinds a `moves` table can actually gate. */
export const GATEABLE_KINDS: readonly MoveKind[] = MOVE_KINDS.filter(
  (kind) => !UNGATED_KINDS.includes(kind),
);

/** Whether `value` names a move kind. */
export function isMoveKind(value: string): value is MoveKind {
  return (MOVE_KINDS as readonly string[]).includes(value);
}

/** One parsed marker line. */
export interface Move {
  kind: MoveKind;
  /** The `#topic` it attaches to, without the `#`. */
  topic?: string;
  /** The `>N` message an objection is aimed at. */
  target?: number;
  /** The `^N` sequences cited as grounds, in authored order. */
  cites: number[];
  /** The text after the marker and its arguments. */
  body: string;
  /** The exact authored line. */
  text: string;
}

/**
 * The lines of `body` that may carry a marker — i.e. every line outside a fenced
 * code block, with its index.
 *
 * Fences are ``` or ~~~ at the start of a line (after optional indentation), and
 * a fence closes only on the same character. That asymmetry matters: a ``` block
 * containing a ~~~ line is still one block, and treating the inner line as a
 * close would expose the rest of the sample to the parser.
 */
export function markerLines(body: string): { line: string; index: number }[] {
  const out: { line: string; index: number }[] = [];
  let fence: { character: string; length: number } | null = null;
  body.split("\n").forEach((line, index) => {
    const opener = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      // A closer must use the opener's character, be at least as long, and
      // contain nothing after the fence other than whitespace. This permits a
      // four-backtick sample to show a three-backtick example without ending
      // the outer block.
      if (
        opener &&
        opener[1][0] === fence.character &&
        opener[1].length >= fence.length &&
        /^\s{0,3}(?:`{3,}|~{3,})\s*$/.test(line)
      )
        fence = null;
      return;
    }
    if (opener) {
      fence = { character: opener[1][0], length: opener[1].length };
      return;
    }
    // `parseMove` deliberately accepts leading whitespace for ordinary prose,
    // but four spaces (or a tab) introduce Markdown's indented code block.
    if (/^(?: {4}|\t)/.test(line)) return;
    out.push({ line, index });
  });
  return out;
}

/**
 * Read one line as a move, or `null` when it is not one.
 *
 * Returns `null` for a line with no leading `!`, for a marker this host does not
 * police, and for the two markers whose arguments are mandatory:
 *
 * - `!refute` requires **both** a `#topic` and at least one `^cite`, so every
 *   refutation is grounded by construction — it caps a topic for the whole room
 *   and must never rest on nothing.
 * - `!defer` requires a `#topic`, so it abstains from something in particular.
 *
 * `!propose` with no `#topic` names nothing and is discarded, which is the host's
 * behaviour too (`!propose canary …` deposits no trace).
 */
export function parseMove(line: string): Move | null {
  const trimmed = line.trimStart();
  if (!trimmed.startsWith("!")) return null;
  const word = trimmed.slice(1).split(/\s/, 1)[0];
  // `!unpin` asks for the `pin` permission: both write the same desk pinboard,
  // and a member entitled to put something on the board may take it off again.
  const kind = word === "unpin" ? "pin" : word;
  if (!isMoveKind(kind)) return null;

  const rest = trimmed.slice(1 + word.length);
  const topic = /(?:^|\s)#([A-Za-z0-9][A-Za-z0-9_-]*)/.exec(rest)?.[1];
  const target = /(?:^|\s)>(\d+)/.exec(rest)?.[1];
  const cites = [...rest.matchAll(/(?:^|\s)\^(\d+)/g)].map((m) => Number(m[1]));

  if (kind === "refute" && (topic === undefined || cites.length === 0)) return null;
  if (kind === "defer" && topic === undefined) return null;
  if (kind === "propose" && topic === undefined) return null;

  // Whatever is left once the arguments are struck out.
  const body = rest
    .replace(/(?:^|\s)#[A-Za-z0-9][A-Za-z0-9_-]*/, " ")
    .replace(/(?:^|\s)>\d+/, " ")
    .replace(/(?:^|\s)\^\d+/g, " ")
    .trim();

  return {
    kind,
    ...(topic === undefined ? {} : { topic }),
    ...(target === undefined ? {} : { target: Number(target) }),
    cites,
    body,
    text: trimmed,
  };
}

/**
 * The first move in a message body, or `null` when it carries none.
 *
 * The host journals only the marker line, so in practice this reads the whole
 * body. It still scans rather than assuming line 0, because a turn that
 * deposited no trace falls through to the first thing it actually said, and a
 * reply that opens with a greeting is the shape that produces.
 */
export function moveOf(body: string): Move | null {
  for (const { line } of markerLines(body)) {
    const move = parseMove(line);
    if (move) return move;
  }
  return null;
}

/**
 * Whether a line looks like a move whose `!` was stripped.
 *
 * The host **demotes** a barred move by journaling the line with its leading `!`
 * removed, so it folds to no trace at all. That is deliberately invisible to the
 * fold and deliberately visible to a reader: a demoted line still says what its
 * author wanted to say, and the console showing it as "this seat tried a move it
 * does not hold" is the difference between a desk whose grammar is wrong and a
 * desk whose members are unhelpful.
 *
 * A heuristic, and only ever used to annotate — never to count.
 */
export function looksDemoted(body: string): MoveKind | null {
  const first = markerLines(body)[0]?.line.trimStart();
  if (!first || first.startsWith("!")) return null;
  const word = first.split(/\s/, 1)[0];
  const kind = word === "unpin" ? "pin" : word;
  return isMoveKind(kind) ? kind : null;
}

/**
 * The moves `member` may open a line with, in {@link MOVE_KINDS} order.
 *
 * Mirrors `HiveConfig::moves_for`. Two readings are deliberately identical:
 *
 * - a member the table does **not name** keeps every move, so an omitted table is
 *   a no-op for every manifest written before it existed;
 * - a member named with an **empty list** also keeps every move, because an empty
 *   list is a table somebody started and never filled in far more often than it
 *   is a vow of silence, and the other reading hands a seat the floor with
 *   nothing legal to say.
 *
 * {@link UNGATED_KINDS} are always unioned in.
 */
export function movesFor(
  moves: Record<string, string[]> | undefined,
  member: string,
): MoveKind[] {
  const named = moves?.[member];
  if (!named || named.length === 0) return [...MOVE_KINDS];
  const held = new Set<MoveKind>(UNGATED_KINDS);
  for (const kind of named) if (isMoveKind(kind)) held.add(kind);
  return MOVE_KINDS.filter((kind) => held.has(kind));
}

/**
 * Whether the table governs this seat at all — i.e. whether a grammar is
 * installed on it.
 *
 * Invisible from {@link movesFor} alone, because "named with every kind" and
 * "not named" produce the same list, and only one of them is a decision somebody
 * made.
 */
export function isGoverned(
  moves: Record<string, string[]> | undefined,
  member: string,
): boolean {
  const named = moves?.[member];
  return !!named && named.length > 0;
}

/** Whether `member` may open a line with `kind`. */
export function may(
  moves: Record<string, string[]> | undefined,
  member: string,
  kind: MoveKind,
): boolean {
  return movesFor(moves, member).includes(kind);
}

/**
 * The turn budget a desk of `members` runs on when the manifest names none.
 *
 * Three turns per member: an opening position, a reply to the room, and a commit
 * — the shortest sequence that can actually reach a quorum. Conformity in a room
 * of language models rises with interaction time, so a bigger budget buys
 * correlated error rather than a better answer.
 */
export function derivedTurnBudget(members: number): number {
  return Math.max(2, members) * 3;
}

/**
 * The quorum a desk of `members` runs on when the manifest names none.
 *
 * A simple majority that still leaves somebody outside it, so a decision is never
 * contingent on the whole room agreeing. For a pair that is exactly one
 * supporter, which is the only number available.
 */
export function derivedQuorum(members: number): number {
  const n = Math.max(2, members);
  return Math.max(1, Math.min(Math.floor(n / 2) + 1, n - 1));
}

/** Clamp an operator's quorum the way the host does on read. */
export function effectiveQuorum(declared: number | undefined, members: number): number {
  const n = Math.max(2, members);
  if (declared === undefined) return derivedQuorum(members);
  return Math.min(Math.max(declared, 1), n);
}

/** The turn budget in force, honouring an operator's number except zero. */
export function effectiveTurnBudget(
  declared: number | undefined,
  members: number,
): number {
  if (declared === undefined) return derivedTurnBudget(members);
  return Math.max(declared, 1);
}

/**
 * How many seats may deposit a distinct supporter.
 *
 * `!propose` counts, and that is not a rounding error: in `tinyhivemind` a
 * proposal already counts as its own author's support, so a seat that may propose
 * can carry a topic without ever being allowed to `!support` one.
 */
export function eligibleSupporters(
  moves: Record<string, string[]> | undefined,
  members: string[],
): number {
  return members.filter(
    (id) => may(moves, id, "support") || may(moves, id, "propose"),
  ).length;
}

/**
 * Every problem with a hive config on a desk of `members`, in the console's own
 * words.
 *
 * Mirrors the refusals in `src/company/manifest.rs`. The host is the authority —
 * this exists so the grammar editor can put an error next to the control that
 * caused it *before* a round trip, and the server's own sentences are rendered
 * verbatim when they come back.
 */
export interface GrammarProblem {
  /** The control to attach this to, e.g. `"quorum"` or `"moves.scout"`. */
  field: string;
  message: string;
}

export function grammarProblems(
  config: {
    quorum?: number;
    turnBudget?: number;
    dominanceCap?: number;
    repetitionCap?: number;
    refutationCap?: number;
    moves?: Record<string, string[]>;
    enabled?: boolean;
  },
  members: string[],
): GrammarProblem[] {
  const problems: GrammarProblem[] = [];

  // Every zero is refused rather than clamped: an operator who wrote a number
  // meant it, and silently substituting a different one is how a desk ends up
  // behaving in a way its manifest does not describe.
  if (config.quorum === 0)
    problems.push({
      field: "quorum",
      message:
        "A quorum of 0 would settle every topic the moment it was proposed. A topic needs at least one grounded supporter to carry.",
    });
  if (config.turnBudget === 0)
    problems.push({
      field: "turnBudget",
      message:
        "A turn budget of 0 opens a room that is exhausted before anybody speaks. Turn the room off instead if that is what you want.",
    });
  for (const [field, value] of [
    ["dominanceCap", config.dominanceCap],
    ["repetitionCap", config.repetitionCap],
    ["refutationCap", config.refutationCap],
  ] as const) {
    if (value === 0)
      problems.push({
        field,
        message: `A cap of 0 fires before anybody has done anything. Leave ${field} unset to keep its default.`,
      });
  }

  for (const [member, kinds] of Object.entries(config.moves ?? {})) {
    if (!members.includes(member))
      problems.push({
        field: `moves.${member}`,
        message: `\`${member}\` is not on this desk. List the desk's own member ids.`,
      });
    for (const kind of kinds) {
      if (!isMoveKind(kind))
        problems.push({
          field: `moves.${member}`,
          message: `\`${kind}\` is not a move. Valid moves are ${MOVE_KINDS.join(", ")}.`,
        });
    }
  }

  // The load-bearing one. A desk whose table lets fewer seats deposit a
  // supporter than its quorum needs can never decide anything, however much the
  // room agrees — the barred seats simply cannot say so.
  if (config.enabled !== false && members.length >= 2) {
    const quorum = effectiveQuorum(config.quorum, members.length);
    const eligible = eligibleSupporters(config.moves, members);
    if (eligible < quorum)
      problems.push({
        field: "quorum",
        message: `Only ${eligible} of ${members.length} seats may support or propose, but quorum needs ${quorum} distinct supporters — a topic here can never carry. Widen the table, or lower quorum to ${eligible}.`,
      });
  }

  return problems;
}
