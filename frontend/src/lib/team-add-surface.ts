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
  // Issue #753: `echo` is the offline brain, and the reduced dialog is a
  // handoff — it collects a name and a sentence and sends the operator to
  // `#/team/<id>?edit` for everything else. On `echo` there is nothing at the
  // other end of that handoff. No draft-a-whole-teammate route exists to fill
  // the dialog in before the write (see the module header: `draftAgentField`
  // drafts ONE field, and `name`/`role` are excluded on purpose), and the
  // detail page's copilot switches itself off on this path outright —
  // `disabled={saving || cognition === "echo" || !draft.role.trim()}` in
  // `AgentDetailView.tsx`. Reducing the dialog here would therefore land the
  // operator on a page whose drafting is dead, having already stopped asking
  // for the fields that page can no longer write. The full form asks for what
  // nothing on this path can supply, which is the only honest answer.
  //
  // This is NOT the argument #1988 settled, and citing that issue here would be
  // wrong: its dialog reduced to one box on EVERY company (commit `a318c92ad`,
  // not an ancestor of this branch), so "the can't-draft path keeps today's
  // form" is not a decision to inherit. #1988's dialog had a whole-thing draft
  // route behind it; this one does not. The reason above is this dialog's own.
  if (args.cognition === "echo") return "form";
  if (args.designRefused) return "form";
  return "describe";
}

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
