// Issue #1776: drafting ONE teammate's mandate or persona.
//
// ## Nothing here saves
//
// This module asks the host for text and hands it back. It calls no write, and
// deliberately does not import one: the draft is rendered beside the field for
// the operator to keep or throw away, and it only ever becomes a teammate's
// persona through the ordinary `PATCH` that a Save already performs.
//
// That is the whole reason a model may write into these two fields at all. The
// host's roster designer keeps a model out of a teammate's standing
// instructions (`src/company/setup.rs`) because there the text reaches a system
// prompt with nobody having read it. Here a person reads it, takes it, and then
// saves it — two deliberate actions — which is the same stance the workflow
// copilot's proposal protocol takes: the model's output is data in a reply, and
// the operator's own action is what writes.
//
// ## A conversation, not a hint box
//
// It started as one shot: a note box, a Draft button, one answer. Using it made
// the problem obvious — a single note cannot say "no, more like this", and
// refining meant retyping the whole instruction each time, because nothing
// carried. So each request now sends the transcript, and the copilot answers in
// turns: a sentence about what it changed, and the whole field rewritten. It
// may also ASK, and answer with no draft at all, which is the part a one-shot
// pass structurally cannot do.
//
// The transcript lives in the panel's state and nowhere else. The host stores
// nothing, which is why this needed no journal, no thread id, and no cleanup.
//
// ## The grounding is the host's, not ours
//
// The console holds the roster, the company name and this teammate's fields
// already, and could have composed the prompt itself — the workflow copilot
// does exactly that. It must not here. A grounding the caller composes is one
// the caller can widen, and this one is deliberately narrow: the teammate, its
// neighbours' ids and roles, and nothing else about the company. So the request
// carries only which field to draft and what the operator typed.

import type { OpenCompanyClient } from "./client";

/** The teammate fields a draft can be asked for. */
export type DraftableField = "description" | "instructions";

/** Who said one thing in a copilot conversation. */
export type TurnRole = "operator" | "copilot";

/**
 * One turn, as the panel holds it and as the host is told about it.
 *
 * The console owns the transcript and sends it back each turn — the host stores
 * nothing. That is the whole of "in-session": closing the form ends the
 * conversation, and there is no journal to rehydrate, no thread id to collide,
 * and nothing to clean up.
 */
export interface CopilotTurn {
  role: TurnRole;
  /** What was said. For a copilot turn, its reply and the draft it produced. */
  text: string;
  /**
   * The field text this turn produced, when it drafted rather than asked.
   *
   * Held apart from {@link text} because the two are for different readers:
   * `text` is what goes back to the model as its own prior turn, and this is
   * what "Use it" puts in the box.
   */
  draft?: string;
}

/**
 * What a copilot turn is sent back to the model as.
 *
 * Its reply *and* its draft, because "shorter" has to mean shorter than
 * something the model can see. Sending only the reply would leave it iterating
 * on a description of a draft rather than on the draft.
 */
export function turnForWire(turn: CopilotTurn): { role: TurnRole; text: string } {
  return { role: turn.role, text: turn.text };
}

/** Composes a copilot turn's wire text from what it said and what it drafted. */
export function copilotTurnText(reply: string, draft?: string): string {
  return [reply.trim(), draft?.trim() ? `Draft:\n${draft.trim()}` : ""]
    .filter(Boolean)
    .join("\n\n");
}

/**
 * Why there is no draft.
 *
 * Several reasons rather than one, because the operator's next move differs for
 * each — the same split the roster proposal's `RosterFallback` makes, and for
 * the same reason: a single sentence covering all of them is too vague to act
 * on. Only one of these is worth retrying.
 */
export type DraftRefusal =
  /** Nothing is wired, so no call ran. Set up a model. */
  | "no_model"
  /** A model is wired and the call did not land. Retry, or check the provider. */
  | "model_unreachable"
  /** A model answered and the answer could not be used. Say more, or write it. */
  | "unreadable"
  /** The company's token ceiling for the period is spent. Nothing to retry. */
  | "budget_exhausted";

/** One copilot turn, as the host answers. */
export interface ProfileDraft {
  /**
   * The field this draft is for, echoed by the host.
   *
   * Load-bearing rather than decorative: two fields share one form, so a
   * response landing after the operator moved to the other box has to be
   * matched to the box it was asked for.
   */
  field: DraftableField;
  /**
   * What the copilot says — what it changed, or what it needs to know. Absent
   * on a refusal.
   */
  reply?: string;
  /**
   * The whole field as it now stands, already clamped host-side.
   *
   * Absent on two different occasions, and `source` is what tells them apart: a
   * turn that asked a question instead of drafting (`source: "model"`), and a
   * turn that could not happen at all (`source: "unavailable"`). Letting the
   * copilot ask is what makes this a conversation rather than a slot machine.
   */
  text?: string;
  /**
   * Who wrote this.
   *
   * `"model"` — a model drafted it from this teammate and its neighbours.
   * `"unavailable"` — there is no draft, and `reason` says why.
   *
   * There is deliberately no curated fallback: "what does this particular
   * teammate own" has no canned answer the way a starting roster does, so the
   * honest response is the refusal rather than text nobody wrote.
   */
  source: "model" | "unavailable";
  /** Why there is no draft. Present only when `source` is `"unavailable"`. */
  reason?: DraftRefusal;
}

/**
 * What to tell the operator when no draft came back.
 *
 * Keyed by reason because the sentence has to name a different next move each
 * time; a shared "couldn't draft that" would send someone to check a credential
 * that is working, or to rewrite a note when the provider is simply down.
 */
export function refusalNotice(reason: DraftRefusal | undefined): string {
  switch (reason) {
    case "no_model":
      return "This company has no model configured, so the copilot can't draft yet — set one up in Settings → Inference, or write the field yourself.";
    case "model_unreachable":
      return "The model didn't answer in time. Try again, or check the provider in Settings → Inference.";
    case "unreadable":
      return "The model's answer couldn't be used. Try again, or add a note saying what this teammate should own.";
    case "budget_exhausted":
      // The one reason with nothing to retry: the ceiling is a plan setting,
      // not a transient failure, so "try again" would be advice that cannot
      // work. Says what to change instead.
      return "This company has spent its token budget for the period, so the copilot can't draft until it resets — raise the ceiling in Settings, or write the field yourself.";
    default:
      // A host too old to send a reason, or one that grew a reason this console
      // does not know. Says what happened without inventing which of the three
      // it was — guessing here would send the operator to fix the wrong thing.
      return "The copilot couldn't draft that. Try again, or write the field yourself.";
  }
}

/**
 * Ask the host to draft one field for one teammate.
 *
 * Never throws for a *drafting* reason: a company with no model, a provider
 * that did not answer and an answer that could not be read all come back as a
 * `200` carrying a reason, because none of them is a failure of the request.
 * A rejection here is a genuine transport, auth or not-found failure.
 */
export function draftAgentField(
  client: OpenCompanyClient,
  company: string | null,
  agentId: string,
  field: DraftableField,
  conversation: CopilotTurn[],
  onScreen?: {
    description?: string;
    instructions?: string;
    role?: string;
    name?: string;
  },
): Promise<ProfileDraft> {
  return client.post<ProfileDraft>(
    `${client.scopeFor(company)}/team/${encodeURIComponent(agentId)}/draft`,
    {
      field,
      messages: conversation.map(turnForWire),
      // What the form holds right now, which is not always what the host has
      // stored: the operator may have taken a draft with `Use it` and not
      // pressed Save. The host prefers these when present, because "make it
      // shorter" has to mean shorter than the text they are looking at.
      description: onScreen?.description?.trim() || undefined,
      instructions: onScreen?.instructions?.trim() || undefined,
      // The identity on screen, for the same reason and one more: both prompts
      // are written FROM the role, so an operator who repurposes a teammate and
      // drafts before saving would otherwise get a mandate for its old job.
      role: onScreen?.role?.trim() || undefined,
      name: onScreen?.name?.trim() || undefined,
    },
  );
}

/**
 * Ask the host to draft one field for a teammate that does not exist yet — the
 * Add-teammate form.
 *
 * The teammate's own fields ride the request because nothing has been created
 * and there is nowhere else to read them from. That is not the widening the
 * id-bearing call refuses: these are the fields being authored on screen right
 * now. What stays host-side is the part that matters — the rest of the company.
 *
 * A blank `role` is refused by the host, because both prompts are written from
 * it. The form disables the control for the same reason, so this is the host
 * stating the rule rather than trusting the console to.
 */
export function draftNewAgentField(
  client: OpenCompanyClient,
  company: string | null,
  field: DraftableField,
  conversation: CopilotTurn[],
  teammate: {
    role: string;
    name?: string;
    description?: string;
    instructions?: string;
  },
): Promise<ProfileDraft> {
  return client.post<ProfileDraft>(`${client.scopeFor(company)}/team/draft`, {
    field,
    messages: conversation.map(turnForWire),
    role: teammate.role.trim(),
    name: teammate.name?.trim() || undefined,
    description: teammate.description?.trim() || undefined,
    instructions: teammate.instructions?.trim() || undefined,
  });
}

/**
 * A whole teammate, as one design pass wrote it (issue #1989).
 *
 * Three fields or none: `source` is `"unavailable"` when the pass could not
 * run, and `reason` says which of the four. A partial design is not a shape the
 * host can return — a teammate with a real mandate and a fragment for a role is
 * exactly what this replaced, and it looks finished.
 */
export interface TeammateDesign {
  /** The job title. Absent on a refusal. */
  role?: string;
  /** The mandate — one line on what this teammate owns. Absent on a refusal. */
  description?: string;
  /** The standing instructions. Absent on a refusal. */
  instructions?: string;
  /** `"model"` when a model designed it, `"unavailable"` when none could. */
  source: "model" | "unavailable";
  /** Why there is none. Present only when `source` is `"unavailable"`. */
  reason?: DraftRefusal;
}

/**
 * How long a design pass may take, end to end.
 *
 * The host's own `PERSONA_TIMEOUT` is 90 seconds; this is that plus room for
 * the round trip, so the console never gives up on a pass the host is still
 * willing to finish. Only the desktop transport reads it — see the note on
 * {@link designTeammate}.
 */
const DESIGN_TIMEOUT_MS = 105_000;

/**
 * Ask the host to design a whole teammate — role, mandate and persona — from
 * the name and the sentence the reduced Add-teammate dialog collected.
 *
 * ## Why this is not `draftNewAgentField` with `field: "role"`
 *
 * {@link DraftableField} excludes `role` and keeps excluding it. Its reason is
 * that a role is what delegation grounds on, so a drafted one would change who
 * the company routes work to — a statement about *editing a teammate that
 * exists*. This route takes no agent id at all, so there is no teammate whose
 * routing it could change; it is the creation case, where the alternative to a
 * designed role was the console cutting the operator's sentence at sixty
 * characters and storing the front half as a job title.
 *
 * It is also one call rather than three. The three fields are not independent —
 * a persona written against a separately-drafted role can disagree with it —
 * and the operator is watching a spinner while this runs.
 *
 * Never throws for a *design* reason: all four refusals come back as a `200`
 * carrying `source: "unavailable"`, because none of them is a failure of the
 * request. A rejection here is a genuine transport, auth or not-found failure —
 * or the caller's own `signal`, which rejects with an `AbortError`.
 *
 * ## Why this one takes a signal when the draft routes do not
 *
 * It is the only copilot call the operator can walk away from mid-flight. A
 * field draft is asked for by a control inside a panel that stays open; this is
 * asked for by **Create**, and the dialog it belongs to has an Escape key, a
 * backdrop and a close icon. Without a signal, closing during the ninety
 * seconds this can take leaves the pass running to completion on the host, its
 * tokens metered against the company's plan, and its answer dropped on the
 * floor by the dialog's `attempt` guard — spend with nothing at either end of
 * it. Dropping the connection drops the handler future with it; the route holds
 * no write lock and returns text (see `design_teammate`), so there is nothing
 * half-done for an abandoned request to leave behind.
 *
 * ## Why it names a deadline when no other mutation does
 *
 * A mutation's duration is normally the host's to decide, and `client.post`
 * leaves it unbounded for exactly that reason. This one has to say a number
 * anyway, because the **desktop** app imposes a deadline the console cannot
 * see: `ProxyTransport` goes through the core's `oc_request`, which applied a
 * flat 30-second `reqwest` timeout. The host deliberately allows this pass 90
 * seconds (`PERSONA_TIMEOUT`), and a measured persona response has taken 40, so
 * every slow-but-valid design on desktop came back as a transport failure and
 * handed the operator the full form — a refusal for a pass that was working.
 *
 * The number is the host's deadline plus room for the round trip, and it is
 * carried across the bridge rather than hard-coded in the core, because only
 * the caller knows which route it is asking for. In a browser it changes
 * nothing: `BrowserTransport` has no deadline of its own.
 *
 * ## What cancelling does not do
 *
 * It does not make the pass free. The drop lands between the provider call and
 * `record_profile_draft_usage`, so what the provider had already generated is
 * billed upstream and recorded in no ledger of ours. The host logs that seam
 * (`DesignSeam`) rather than hiding it, and what a cancelled pass should be
 * charged is an open decision. Cancelling stops the host early; it is not a
 * refund.
 */
export function designTeammate(
  client: OpenCompanyClient,
  company: string | null,
  teammate: { name?: string; description: string },
  signal?: AbortSignal,
): Promise<TeammateDesign> {
  return client.post<TeammateDesign>(
    `${client.scopeFor(company)}/team/design`,
    {
      name: teammate.name?.trim() || undefined,
      description: teammate.description.trim(),
    },
    { signal, timeoutMs: DESIGN_TIMEOUT_MS },
  );
}
