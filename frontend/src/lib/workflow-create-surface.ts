import { ApiError } from "@/api/types";
import { utf8ByteLength } from "@/views/chat/mentions";

/**
 * Which of the two New-workflow dialogs is on screen.
 *
 * - `describe` — **one box**: a sentence and Create. Name, Workflow ID,
 *   Description, Nodes and Connections are not rendered at all, and neither is
 *   the validation that serves them. This is what *creating* a workflow is now,
 *   on every company and every build. Create drafts if the copilot can, falls
 *   back to the operator's own sentence if it cannot, and either way lands them
 *   on the canvas.
 * - `form` — the manual graph form, byte-for-byte what the dialog has always
 *   been. Two things reach it: an **edit**, which already has a graph, and a
 *   create the host **refused**, which needs controls for the thing it refused.
 */
export type CreateSurface = "describe" | "form";

/**
 * The **one** place the two dialogs are told apart.
 *
 * A pure function, exported and exhaustively tested, because the failure here is
 * silent in one direction: if this answers `form` on a create, nothing breaks —
 * the dialog just looks like it always did, and nobody notices the redesign
 * never shipped. A predicate spelled inline in the component would be provable
 * only by rendering it, and only for the cases somebody thought to render.
 *
 * The copilot's availability is deliberately **not** an input. It used to be,
 * and running it proved that wrong: an operator on a host with no model got the
 * full graph form — the dialog this redesign exists to retire — in exactly the
 * case where they are least likely to want to hand-author a graph. So the box is
 * unconditional, and what changes when the copilot cannot draft is what Create
 * *does*, not what the dialog *is*.
 */
export function createSurface(args: {
  /** Edit mode. An edit already has a graph, so there is nothing to draft. */
  editing: boolean;
  /**
   * Whether a one-box create was **refused** by the host.
   *
   * The one that actually happens: the host mints a draft's id by slugging the
   * name and deduping against the workflows it has *saved*
   * (`safe_workflow_id`, `src/harness/built_in/workflow_build.rs`), but nothing
   * reserves it — so two similar descriptions drafted before either is created
   * mint the same id, and the second Create answers
   * `409 A workflow with id ... already exists. Pick a different id.` A dialog
   * with no id field has no way to obey that, so the refusal hands the operator
   * the full form loaded with the graph that was refused. The one-box dialog
   * never dead-ends.
   */
  writeRefused: boolean;
}): CreateSurface {
  if (args.editing) return "form";
  if (args.writeRefused) return "form";
  return "describe";
}

/**
 * The three ways a build can answer "I cannot draft at all", as opposed to
 * "I drafted nothing useful".
 *
 * These are facts about the deployment, not about the description: `not_wired`
 * (404) is a build with no embedded brain, `inference_required` (409) a company
 * with no provider configured, `restart_required` (409) a provider configured
 * since the process booted. None of them is fixed by rewording the sentence, so
 * each one retires *drafting* for this open — the dialog says so in the host's
 * own words, and Create builds the workflow from the operator's sentence
 * instead. The box stays; only the promise above it changes.
 */
const CAPABILITY_CODES = new Set(["not_wired", "inference_required", "restart_required"]);

/**
 * Classifies a failed draft: the host's message when the copilot is
 * **unavailable**, `null` for every other failure.
 *
 * Keyed on the structured `code`, never the prose — the same rule the run
 * refusal banner follows, and for the same reason: a reworded host message must
 * not silently change which dialog an operator sees.
 *
 * A network blip, a 500 or a 400 answers `null` deliberately. Those say nothing
 * about whether this company can draft — retiring the copilot over a dropped
 * connection would leave the operator building by hand on a host that would have
 * drafted it for them a second later.
 */
export function draftCapabilityGap(err: unknown): string | null {
  if (!(err instanceof ApiError)) return null;
  if (!CAPABILITY_CODES.has(err.code)) return null;
  return err.message;
}

/**
 * Whether a refused write is one the operator can **act on**, and so whether the
 * one-box dialog should hand over the full form.
 *
 * This exists because the obvious spelling of "the write failed" was wrong. The
 * hand-over is a one-way door — it retires the box for the rest of the open —
 * and it used to fire from the first line of the write path's `catch`, so a
 * dropped connection or a 500 collapsed the redesign into the graph form the
 * operator had just been spared. That is the same failure
 * {@link draftCapabilityGap} is written the long way round to avoid, one path
 * over: a transport failure says nothing about what the operator should do next.
 *
 * So it is keyed on what the host actually **asked for**:
 *
 * - the answer came from the host's own `{error, code}` envelope (`fromHost`,
 *   issue #380) — without that, the status is the client's own synthesis of
 *   whatever hop gave up;
 * - a `409` names an id that is taken — "pick a different id" is an instruction,
 *   and the id field is the only way to obey it;
 * - `problems` are per-node complaints (`workflow_invalid`), each of which wants
 *   a control to land on.
 *
 * Everything else — a network blip, a 500, a 400 with no problems, a thrown
 * `TypeError` — leaves the box up and the banner showing. The operator presses
 * Create again.
 *
 * ## Why `fromHost` is checked first
 *
 * `httpError` sets it from whether the body parsed as the host's envelope
 * (`src/api/client.ts`), so a proxy or gateway answering `409` with an HTML
 * page — or with an empty body over HTTP/2, where there is not even a reason
 * phrase — arrives here as a `409` the host never said. Handing over the form
 * for one retires the box over an intermediary's opinion about a request the
 * host may never have seen: the same failure this function exists to prevent
 * for `500`s, one status along.
 *
 * The `problems` branch needs no separate guard — `problems` is populated only
 * off a parsed envelope — but it sits behind the same gate rather than relying
 * on that staying true.
 */
export function writeRefusalHandsOverForm(err: unknown): boolean {
  if (!(err instanceof ApiError)) return false;
  if (!err.fromHost) return false;
  if (err.status === 409) return true;
  return (err.problems?.length ?? 0) > 0;
}

/**
 * What a copilot decline actually was, and what to say about it.
 *
 * `automatable: false` is one flag over two very different events, and the
 * dialog used to render both as advice:
 *
 * - the copilot **judged** the work better done once — a real opinion about the
 *   description, worth showing in the host's own words; and
 * - the copilot **failed** — it ran out of time, errored, exhausted its step
 *   budget, or could not produce a graph its own gates would accept.
 *
 * The second was reaching operators as advice, verbatim, and the verbatim text
 * is the gate diagnostics: *"the described workflow could not be drafted into
 * one that would be accepted: invalid request: a workflow needs exactly one
 * `trigger` node to say what starts it (found 0)."* `trigger` nodes are the
 * exact vocabulary the one-box dialog exists to stop putting in front of people,
 * and calling a crash "better done by hand" is not true.
 *
 * ## Why this reads the prose
 *
 * The host sends no code with a decline — `DraftFromDescriptionResponse`
 * carries `automatable` and a `reason` string, nothing more. So this keys on the
 * four sentence stems `not_automatable_reason`
 * (`src/harness/built_in/workflow_build.rs`) generates, which are literals in
 * that function rather than model output.
 *
 * The failure direction is chosen to be harmless: anything unrecognised is a
 * **judgment**, which is the behaviour that shipped before this and shows the
 * host's own words. A reworded stem therefore degrades to today's dialog rather
 * than to a wrong claim, and nothing about which dialog is on screen depends on
 * it.
 */
export interface DraftDecline {
  /** `judgment` — the copilot's opinion. `failure` — the copilot did not manage it. */
  kind: "judgment" | "failure";
  /** What to render: the host's own words for a judgment, ours for a failure. */
  message: string;
  /** The label on the action beside it — the two mean different things. */
  action: string;
}

/**
 * The stems {@link draftDecline} recognises as a failure, each with the honest
 * sentence to say instead.
 *
 * Mirrors `not_automatable_reason`'s four arms. The stems are prefixes, matched
 * case-insensitively, because each arm appends its own diagnostics.
 */
const DECLINE_FAILURES: ReadonlyArray<readonly [string, string]> = [
  [
    "drafting the workflow ran out of time",
    "The copilot ran out of time before it had a draft.",
  ],
  [
    "drafting the workflow could not complete",
    "The copilot hit an error and did not finish a draft.",
  ],
  [
    "the workflow copilot reached its step budget",
    "The copilot ran out of steps before it had a draft.",
  ],
  [
    "the described workflow could not be drafted into one that would be accepted",
    "The copilot could not turn that into a workflow it would accept.",
  ],
];

/** The tail every failure message carries — what the operator can do next. */
const FAILURE_REMEDY =
  " Reword it and press Create to try again, or start it on the canvas and build it there.";

/**
 * Classifies a decline and words it. See {@link DraftDecline} for why.
 *
 * An empty or missing reason is a judgment with the same default the banner
 * reducer uses, so a host that sends nothing reads as it always did.
 */
export function draftDecline(reason: string | null | undefined): DraftDecline {
  const stated = (reason ?? "").trim();
  const lowered = stated.toLowerCase();
  const failure = DECLINE_FAILURES.find(([stem]) => lowered.startsWith(stem));
  if (failure) {
    return {
      kind: "failure",
      message: `${failure[1]}${FAILURE_REMEDY}`,
      action: "Start it on the canvas",
    };
  }
  return {
    kind: "judgment",
    message: stated || "This is better done once than built into a workflow.",
    action: "Create it anyway",
  };
}

/** Cap on a derived name, so a rambling sentence cannot become a 400-character title. */
const NAME_CAP = 60;

/**
 * The host's own ceiling on a workflow name, in **UTF-8 bytes**.
 *
 * `MAX_WORKFLOW_NAME_LEN` (`src/company/workflow_create.rs:183`), checked there
 * as `draft.name.trim().len() > MAX_WORKFLOW_NAME_LEN` — and Rust's `str::len`
 * counts bytes, however the message words it ("at most 200 characters").
 *
 * {@link NAME_CAP} alone does not imply this. Sixty code points of emoji is 240
 * bytes, and the refusal it earns is a graph-level `400` with no `problems`, so
 * it neither hands over the form nor changes on a second press: the description
 * is simply uncreatable, on the one path that has no copilot to name the
 * workflow instead.
 */
const NAME_BYTE_CAP = 200;

/**
 * A workflow name from the sentence the operator typed — the fallback for the
 * one path that has no copilot draft to take a name from.
 *
 * The copilot names its own drafts, so this is used only by "Create it anyway",
 * where the operator has overruled a decline and there is nothing but their
 * sentence to go on. It takes the **first clause** — up to the first sentence or
 * clause break — because that is where an English description says what the
 * thing is, and the rest says how.
 *
 * `"Every Monday, draft the digest and email it."` → `"Every Monday"`. Crude,
 * and deliberately so: the name is renameable on the canvas a second later, and
 * a name that reads like the operator's own words beats one invented for them.
 *
 * Returns `""` when the sentence has nothing usable — no letter and no digit in
 * the first clause, so `"🎉🎉"` and `"---"` both derive nothing. The caller must
 * treat that as "no name derived" and ask for one, never write an empty name: an
 * empty name also derives an empty id, and an empty id is the permanent join key
 * nothing can fix afterwards.
 *
 * The emptiness rule is stated here rather than left to the caller's
 * `isSafeId` check because two callers now read it, and "the name is empty" is a
 * fact about the sentence rather than about the id that happens to follow.
 */
export function nameFromDescription(description: string): string {
  const firstClause = description.split(/[.;\n,!?]/, 1)[0] ?? "";
  const collapsed = firstClause.replace(/\s+/g, " ").trim();
  // Not merely non-empty: a clause of punctuation or emoji reads as a name on
  // screen and slugs to nothing, which is the empty permanent id this contract
  // exists to refuse.
  if (!/[\p{L}\p{N}]/u.test(collapsed)) return "";
  // Elided on **code points**, not UTF-16 units — the same rule
  // `titleFromMessage` follows in `lib/chat.ts`, and here it is a correctness
  // bug rather than a cosmetic one. Slicing UTF-16 units cuts between the
  // halves of an astral character whenever the 60th unit is a high surrogate
  // (59 ASCII characters then an emoji), and the lone surrogate that leaves is
  // not representable: `JSON.stringify` emits it as a bare `\ud83d`, and the
  // host answers `400 Failed to parse the request body as JSON: name:
  // unexpected end of hex escape` — verified against a running host.
  //
  // Code points rather than grapheme clusters: splitting a ZWJ sequence or
  // orphaning a combining mark leaves a slightly odd-looking name, which is
  // cosmetic on a title the operator renames on the canvas. Splitting a
  // surrogate pair leaves something the wire cannot carry at all.
  const points = Array.from(collapsed);
  // `charAt(0).toUpperCase()` is safe on an astral first character: it is the
  // lone high surrogate and `slice(1)` begins with its low half, so the two
  // concatenate back into the pair, and `toUpperCase()` leaves a lone
  // surrogate alone. It CAN change the byte length ("ß" uppercases to "SS"),
  // which is why the byte check below measures the finished name.
  const build = (kept: number): string => {
    const body = points.slice(0, kept).join("").trimEnd();
    const elided = kept < points.length ? `${body}…` : body;
    return elided.charAt(0).toUpperCase() + elided.slice(1);
  };
  let kept = Math.min(points.length, NAME_CAP);
  let name = build(kept);
  // …and then down to the host's byte ceiling, a whole code point at a time so
  // the surrogate rule above is never undone by the one below. Measured on the
  // finished, trimmed name because that is the string the host measures.
  while (kept > 1 && utf8ByteLength(name.trim()) > NAME_BYTE_CAP) {
    kept -= 1;
    name = build(kept);
  }
  return name;
}
