// The Add-teammate dialog's two shapes, and the derivations the reduced one
// needs (issue #1989).
//
// ## Where the whole-teammate draft route came from
//
// `WorkflowCreateDialog`'s one box works because the host has a
// draft-the-whole-thing route: `draftWorkflowFromDescription` turns a sentence
// into a named, id'd, fully-wired graph before anything is created, so the
// dialog can ask for one thing and still write a complete record.
//
// There was no such route for a teammate, and the first version of this module
// treated that as fixed. The two that existed — `draftAgentField`
// (`/team/<id>/draft`) and `draftNewAgentField` (`/team/draft`) — draft ONE
// field, and only `description` or `instructions`; `DraftableField` excludes
// `name` and `role` on purpose, and `AgentFields.tsx` gives the reason: "a role
// is what delegation grounds on, so a drafted one would change who the company
// routes work to."
//
// **Read that reason against the case it is refusing.** It is about *editing a
// teammate that exists*: work is already routed to it, and a model re-pointing
// that without the operator choosing to is the harm. At creation there is
// nothing to re-route — the teammate does not exist, nothing is addressed to
// it, no orchestrator has seen it. So the exclusion protects a property that
// the create path does not have, and inheriting it here bought nothing and cost
// everything: with no route to ask, this module cut the operator's sentence at
// sixty characters and stored the front half as a permanent job title.
//
// So the route exists now, creation-only: `POST {scope}/team/design`
// (`designTeammate`) takes a name and a sentence and answers with a role, a
// mandate and a persona, in one pass, before anything is written.
// `/team/<id>/draft` still refuses anything but the two prose fields, and takes
// an agent id — which is exactly why the new one takes none.
//
// The redirect to `#/team/<id>?edit` stays, and it is worth being precise about
// what it now does. It is **not** where the drafting happens — that claim was
// made here before and it was false: landing on that page enabled a copilot
// button and nothing else, so a teammate sat with an empty persona until the
// operator noticed and prompted it. The drafting happens on Create. The
// redirect is what puts the three designed fields in front of the operator, in
// editable boxes, so a role a model wrote is read before it can matter.
//
// ## What the reduced dialog therefore asks for
//
// A name and a sentence. Not a sentence alone: with no model in the loop, a
// name could only be derived by splitting the sentence, and a teammate's name
// is not a phrase. `nameFromDescription` in the workflow module yields "Every
// Monday" for a workflow, which reads fine on a canvas; the same split yields
// "Runs paid acquisition" for a teammate, which then renders as a person's name
// on every roster card, in every chat member list and beside every message they
// send. The workflow module can afford that because it is the fallback for one
// rare path ("Create it anyway"); here it would be the only path.
//
// ## Where the role, the mandate and the persona come from
//
// From the model, in one pass, before the write — `POST {scope}/team/design`
// (`designTeammate`). Not from a split of the sentence. This module used to
// carry a `roleFromDescription` that took the first clause and cut it at sixty
// characters with an ellipsis, and every one of its failures was a **stored**
// record: "Runs wholesale outreach to boutique retailers and keeps the…" as a
// permanent job title, "Every Monday" for a sentence that opened with a
// frequency, a 60-character cut for any language whose punctuation the class
// did not name. A split cannot tell a job from an adverbial and no tuning makes
// it able to.
//
// The host's design route is creation-only and takes no agent id, which is what
// keeps `DraftableField`'s exclusion of `role` intact where it means something:
// that rule protects an *existing* teammate's delegation grounding from being
// re-pointed by a model, and a teammate that does not exist has none. See
// `designTeammate` in `api/agent-copilot.ts` and `design_teammate` on the host.
//
// When the pass cannot run — no model, provider down, unreadable answer, token
// ceiling reached — nothing is written and the operator gets the full form
// carrying what they typed, with the host's own reason. That is the same answer
// an `echo` company gets, and it is the only honest one: there is nothing to
// derive a job title from but the sentence, and cutting the sentence up is what
// this replaced.

import type { TeammateDesign } from "@/api/agent-copilot";
import type { CognitionPath } from "@/api/inference";

/**
 * Which of the two Add-teammate dialogs is on screen.
 *
 * - `describe` — a name and one box, then Create. Role, What they do,
 *   Instructions, Daily budget and the inbox toggle are not rendered at all.
 *   Create writes the teammate and lands the operator on its detail page with
 *   the edit form open, where the copilot drafts the rest.
 * - `form` — today's full form, unchanged. What a company whose copilot cannot
 *   draft still gets.
 */
export type AddTeammateSurface = "describe" | "form";

/**
 * The **one** place the two dialogs are told apart.
 *
 * A pure function, exported and exhaustively tested, because the failure here
 * is silent in one direction: if the copilot is reachable and this answers
 * `form`, nothing breaks — the dialog just looks like it always did, and nobody
 * reports that the redesign never shipped. A predicate spelled inline in a
 * component is provable only by rendering it, and only for the cases somebody
 * thought to render.
 *
 * ## Why an unsettled cognition read means `describe`
 *
 * `cognition` is `null` both while `/inference` is in flight and on a host with
 * no such route — issue #753 leaves the copilot ENABLED in that case rather
 * than refusing to draft because we could not confirm, and both dialogs that
 * already read it (`AddMemberDialog`, `AgentDetailView`) follow that rule.
 * This follows it too, and the two wrong answers are not symmetrical:
 *
 * - Guessing `describe` on a company that turns out not to draft costs the
 *   operator a teammate whose description they wrote themselves and whose
 *   persona the detail page's copilot then declines to draft, saying so in the
 *   host's own words. Everything they typed is kept, and the teammate is real.
 * - Guessing `form` on a company that CAN draft is silent: it looks exactly
 *   like the dialog did before this change, so nothing reports it.
 *
 * The loud wrong answer is the one to risk.
 *
 * ## Why there is no duplicate-id input, unlike the workflow dialog
 *
 * `WorkflowCreateDialog` needs a `writeRefused` input because the host mints a
 * workflow id by slugging the name without reserving it, so a second create can
 * land a `409` that a dialog with no id field cannot obey. The teammate write
 * has no such dead end: `add_member` mints the agent id through
 * `record.mint_agent_id(&body.name)` (`src/ports/types.rs`), which sweeps
 * `<slug>_2`, `<slug>_3` … until it finds a free one, so two teammates named
 * the same thing both create. The only hand-over this dialog needs is
 * `designRefused`.
 */
export function addTeammateSurface(args: {
  /** The company's cognition path; `null` while unread or on a host without the route. */
  cognition: CognitionPath | null;
  /**
   * Whether the host says this company can run a design pass at all
   * (`designsProfiles` on `/inference`), or `null`/`undefined` when it did not
   * say — an older host, or a check still in flight.
   *
   * The capability, asked of the host, instead of guessed from `cognition`.
   * The guess was `cognition !== "echo"`, and it is wrong for three of the six
   * paths: `profile_drafter()` is built from `workflow_harness_deps`, assigned
   * in exactly one place — the embedded harness arm of `RuntimeBuilder::build`
   * — so `hosted`, `sidecar` and `custom` companies have no drafter either.
   * Every create through the reduced dialog on one of them was a sentence
   * typed, a Create pressed, a model call waited on that could only answer
   * `no_model`, and then the full form to fill in by hand.
   *
   * `echo` keeps its own line below rather than folding into this one, because
   * the two say different things and only one survives an old host: `echo` is
   * "there is no model on this path at all" and is true from the cognition
   * label alone.
   */
  designsProfiles?: boolean | null;
  /**
   * Whether a Create was already attempted and the design pass could not
   * produce a teammate.
   *
   * The one dead end the reduced dialog can reach, and the reduced dialog never
   * dead-ends — the same promise `WorkflowCreateDialog` makes with its
   * `writeRefused` input. All four host refusals arrive here (`no_model`,
   * `model_unreachable`, `unreadable`, `budget_exhausted`), because the
   * operator's move is the same for all four even though the *reason* they are
   * shown differs: take the full form, which is carrying what they typed, and
   * write the fields themselves.
   *
   * Nothing is written on this path. A teammate is created only from a design
   * the host returned whole.
   */
  designRefused: boolean;
}): AddTeammateSurface {
  // Issue #753: `echo` is the offline brain — there is no model on this path at
  // all. The reduced dialog's Create IS a model call: it sends the sentence to
  // `POST {scope}/team/design` and writes what comes back. On `echo` that pass
  // can only ever refuse, so every create through the reduced dialog would end
  // in the hand-over below — the full form, one wasted round trip later. And
  // the page it would otherwise land on has its own copilot switched off on
  // this path outright (`disabled={saving || cognition === "echo" ||
  // !draft.role.trim()}` in `AgentDetailView.tsx`), so there is nothing at
  // either end of the handoff. Showing the form up front is the same answer
  // arrived at honestly.
  //
  // Deciding it here rather than leaving it to the refusal is what keeps that
  // honest. `designRefused` is the *unexpected* failure — a provider that did
  // not answer, an unreadable reply — and dressing a company that structurally
  // cannot draft as one that tried and failed would tell the operator to retry
  // something that has no model behind it.
  //
  // This is NOT the argument #1988 settled, and citing that issue here would be
  // wrong: its dialog reduced to one box on EVERY company (commit `a318c92ad`,
  // not an ancestor of this branch), so "the can't-draft path keeps today's
  // form" is not a decision to inherit. The reason above is this dialog's own.
  if (args.cognition === "echo") return "form";
  // The host's own answer, when it gave one. Only an explicit `false` retires
  // the reduced dialog: `undefined` is an older host that does not report the
  // capability, and `null` is a check still in flight, and both are read the
  // way an unsettled `cognition` is read — offer the reduced dialog, and meet
  // the refusal honestly if one comes. Guessing `form` there is the silent
  // wrong answer described above.
  if (args.designsProfiles === false) return "form";
  if (args.designRefused) return "form";
  return "describe";
}

/**
 * The longest sentence the reduced dialog will take, matching the host's
 * `MAX_DESIGN_BRIEF`.
 *
 * Held on the box itself so the operator meets the limit while typing rather
 * than in a record that quietly lost the end of what they wrote. The host used
 * to cut the brief at `MAX_DESCRIPTION` — 200 characters, a *roster card
 * layout* bound — before the model read it, and nothing on this side said so:
 * the textarea had no limit, and the stored description is the model's rather
 * than the operator's, so a requirement written past character 200 left no
 * trace at all.
 *
 * 2000 is the bound every other piece of operator free text going into a
 * copilot prompt already obeys (`MAX_TURN_CHARS`). It is far past anything
 * typed into a four-row box; what it stops is a paste.
 */
export const MAX_DESIGN_BRIEF = 2000;

/** What the reduced dialog collects, before it is turned into a create. */
export interface DescribedTeammate {
  name: string;
  description: string;
}

/** What the reduced dialog's Create writes, once the host has designed it. */
export interface DesignedTeammateFields {
  name: string;
  role: string;
  description: string;
  instructions: string;
}

/**
 * Why the reduced dialog's Create cannot run yet, or `null` when it can.
 *
 * Here rather than inline in each dialog because both of them ask it and their
 * two copies had already begun to differ in wording. Nothing about it is
 * clever; what matters is that there is one answer.
 */
export function describeBlocked(described: DescribedTeammate): string | null {
  if (!described.name.trim()) return "A name is required.";
  if (!described.description.trim()) return "Say what they should do.";
  return null;
}

/**
 * What the full form should start from when it takes over from the reduced one,
 * or `null` when there is nothing to carry.
 *
 * ## The bug this exists for
 *
 * The two dialogs are two separate sets of state. The reduced one writes
 * `described`; the full one writes its own name and description fields. Nothing
 * moves between them on its own, and there are **two** ways the surface flips:
 *
 * - **A hand-over after a refused design.** Already carried — `handOver` was
 *   written for exactly this and copies the two values across.
 * - **A cognition read that lands late.** Not carried, and it was the silent
 *   one. `cognition` is `null` while `/inference` is in flight, and
 *   `addTeammateSurface` deliberately reads that as `describe` (see above), so
 *   on an `echo` company with a slow `/inference` the operator gets the reduced
 *   dialog, starts typing, and the answer arrives and swaps the form under
 *   them. Everything typed vanished, with no error and nothing to retry: it
 *   reads as the console eating your input, which is the loudest kind of wrong
 *   this dialog can be.
 *
 * Carrying is the fix rather than holding the form back until the check
 * settles, because on a host without the route `cognition` stays `null`
 * forever (issue #753) — deferring there is a spinner that never resolves.
 *
 * ## Why it refuses to overwrite
 *
 * It answers `null` when the full form already holds anything, so a re-render,
 * or a second flip, cannot put the sentence back over a description the
 * operator has since edited. The reduced values are a *starting point* for a
 * form nobody has touched, never a correction to one they have.
 */
export function carriedDescribe(
  described: DescribedTeammate,
  current: { name: string; description: string },
): { name: string; description: string } | null {
  const name = described.name.trim();
  const description = described.description.trim();
  if (!name && !description) return null;
  if (current.name.trim() || current.description.trim()) return null;
  return { name, description };
}

/**
 * A design already paid for that still answers what is in the box, or `null`.
 *
 * ## Why a dialog holds one at all
 *
 * A design is a model call the company is metered for. Before this, the dialog
 * called `onAdd` and cleared itself on the next line without waiting, so a
 * `POST {scope}/team` that 5xx'd or dropped left an open, blank, enabled form
 * and threw away three things at once: the name, the sentence, and the design.
 * Pressing Create again bought the same design a second time.
 *
 * The write is awaited now and the dialog clears only when it lands, which
 * leaves the design in hand — and this is the guard on reusing it. A design
 * belongs to the sentence it was written from, so the held one is spent only
 * when the name and the sentence are still character-for-character what they
 * were when the host answered. Edit either and it is dropped: reusing it then
 * would store an answer to a question nobody asked.
 */
export function heldFields(
  held: { name: string; description: string; fields: DesignedTeammateFields } | null,
  described: DescribedTeammate,
): DesignedTeammateFields | null {
  if (!held) return null;
  if (held.name !== described.name.trim()) return null;
  if (held.description !== described.description.trim()) return null;
  return held.fields;
}

/**
 * What `POST {scope}/team` is sent, given what the operator typed and what the
 * host designed — or `null` when the design cannot be written.
 *
 * ## Why the name is the operator's and the other three are the model's
 *
 * The name is the one thing no pass can produce: it is how the teammate is
 * addressed, on every roster card and beside every message it sends, and a
 * model asked for one either invents a person or restates the job. The operator
 * types it, and it is sent exactly as typed.
 *
 * Role, mandate and persona are the model's, together, from the one sentence.
 * They are not stitched from three separate answers: a persona written against
 * a role that was drafted in a different call can disagree with it, and
 * reconciling that is precisely the work the reduced dialog exists to save.
 *
 * ## Why this can still answer `null`
 *
 * A design that refused carries no fields, and a design missing any one of the
 * three is not a partial success to salvage — a teammate with a real mandate
 * and a fragment for a role is what this whole change removes, and it looks
 * finished on screen. All-or-nothing here, hand-over in the dialog.
 */
export function designedTeammateFields(
  described: DescribedTeammate,
  design: TeammateDesign,
): DesignedTeammateFields | null {
  const name = described.name.trim();
  const role = design.role?.trim() ?? "";
  const description = design.description?.trim() ?? "";
  const instructions = design.instructions?.trim() ?? "";
  if (!name || !role || !description || !instructions) return null;
  // A designed record must never carry the failure the split produced. The host
  // refuses to truncate a role and this asserts it a second time, because the
  // console is where the operator would meet it and this is the last place that
  // can decline to write one.
  if (role.includes("…")) return null;
  return { name, role, description, instructions };
}
