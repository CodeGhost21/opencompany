// One agent, read and edited (issue #264). The pure half of the detail view:
// the field definitions both the create dialog and the edit form render from,
// and the three derivations that are easy to get quietly wrong.

import type { AgentDetailDto, AgentToolsDto, EditAgentInput, HarnessDto } from "@/api/types";

/** The fields that describe an agent, in the order both forms show them. */
export type AgentFieldKey = "name" | "role" | "description" | "instructions";

export interface AgentFieldSpec {
  key: AgentFieldKey;
  label: string;
  placeholder: string;
  /** `prose` renders a textarea; `line` renders a single-line input. */
  kind: "line" | "prose";
  /**
   * How many rows a `prose` field's textarea shows. Instructions is the whole
   * persona an agent runs on — longer than a one-line "what they do" — so it
   * asks for more room. Ignored for a `line` field.
   */
  rows?: number;
}

/**
 * The single definition of an agent's authored fields.
 *
 * Shared deliberately by "Add teammate" and the detail view's edit form: the
 * two collect the same three things, and before this they would have collected
 * them under two sets of labels and placeholders that drifted apart. The host
 * accepts exactly these keys in a `PATCH`, so the list is also the client half
 * of that contract.
 */
export const AGENT_FIELDS: AgentFieldSpec[] = [
  { key: "name", label: "Name", placeholder: "e.g. Nova", kind: "line" },
  { key: "role", label: "Role", placeholder: "e.g. Growth Marketer", kind: "line" },
  {
    key: "description",
    label: "What they do",
    placeholder: "e.g. Runs paid acquisition and reports on ROAS.",
    kind: "prose",
  },
  {
    key: "instructions",
    label: "Instructions",
    placeholder:
      "e.g. Always confirm the budget before launching a campaign. Report ROAS weekly and flag anything under 2x.",
    kind: "prose",
    // The persona the agent runs on, appended verbatim to its system prompt —
    // longer than the one-line "what they do", so it gets a taller box.
    rows: 8,
  },
];

/** The three authored values, as a form holds them. */
export type AgentDraft = Record<AgentFieldKey, string>;

/** A blank draft, for the create form and for a detail view that has not loaded. */
export function emptyDraft(): AgentDraft {
  return { name: "", role: "", description: "", instructions: "" };
}

/** The draft a detail response starts an edit from. */
export function draftFrom(detail: AgentDetailDto): AgentDraft {
  return {
    // A manifest teammate carries no name of its own and is shown by its role,
    // so the form starts from the same thing the card does.
    name: detail.name ?? "",
    role: detail.role,
    description: detail.description ?? "",
    // The **effective** instructions — the override when one is set, else the
    // blueprint seed — so an edit starts from what the agent actually runs on.
    instructions: detail.instructions ?? "",
  };
}

/**
 * Whether the host will accept an edit to this field.
 *
 * Read from the host's own `editable` list rather than inferred from `source`.
 * The two agree today, and the moment they stop agreeing the host is right:
 * duplicating the rule here is how a console starts offering a field that saves
 * with a 409.
 */
export function isEditable(detail: AgentDetailDto, key: AgentFieldKey): boolean {
  return detail.editable.includes(key);
}

/**
 * The `PATCH` body for a draft, or `null` when nothing changed.
 *
 * Two rules, and both matter:
 *
 * - **Only changed, editable fields are sent.** An unchanged field is left out
 *   entirely, so a form that renders a read-only field cannot echo it back and
 *   have the host refuse the whole save.
 * - **A cleared description is `null`, not `""`.** `undefined` means "leave the
 *   instructions alone" on the wire; sending `undefined` for a field the
 *   operator deliberately emptied would silently keep the old text, and the
 *   operator would watch their deletion come back.
 */
export function agentEdits(detail: AgentDetailDto, draft: AgentDraft): EditAgentInput | null {
  const current = draftFrom(detail);
  const edits: EditAgentInput = {};
  let changed = false;

  for (const field of AGENT_FIELDS) {
    if (!isEditable(detail, field.key)) continue;
    const next = draft[field.key].trim();
    if (next === current[field.key].trim()) continue;
    changed = true;
    if (field.key === "description") {
      edits.description = next === "" ? null : next;
    } else if (field.key === "instructions") {
      // Same three-state as description: an emptied field is `null`, which on
      // the host clears the override and resets the teammate to its blueprint —
      // not `""`, which would try to store an empty persona.
      edits.instructions = next === "" ? null : next;
    } else if (field.key === "name") {
      edits.name = next;
    } else {
      edits.role = next;
    }
  }

  return changed ? edits : null;
}

/** Whether a draft could be saved at all: name and role are required. */
export function draftIsValid(detail: AgentDetailDto, draft: AgentDraft): boolean {
  return AGENT_FIELDS.every(
    (field) =>
      field.kind !== "line" || !isEditable(detail, field.key) || draft[field.key].trim() !== "",
  );
}

/**
 * The `PATCH` value for a teammate's model-override edit, or `undefined` when
 * the draft did not actually change (issue #1245's per-agent follow-up).
 *
 * Its own function rather than a case inside `agentEdits`: the model lives in
 * its own section of the detail view (admin-gated, separate from the
 * name/role/description group `AGENT_FIELDS` edits together), so it needs its
 * own draft, not a fourth `AgentFieldKey`.
 *
 * Same three-way contract as `description`'s handling inside `agentEdits`,
 * and for the same reason: a blank draft means "clear the override, use the
 * harness's own default" (`null`), not "leave it alone" (`undefined`) — an
 * operator who emptied the field on purpose must see that take effect, not
 * watch the old value silently survive the save.
 */
export function modelEdit(current: string | undefined, draft: string): string | null | undefined {
  const next = draft.trim();
  const before = current ?? "";
  if (next === before) return undefined;
  return next === "" ? null : next;
}

/**
 * The `PATCH` value for a teammate's harness-binding edit, or `undefined`
 * when the draft did not change (issue #1245's harness-picker follow-up).
 *
 * Same three-way contract as [`modelEdit`], and deliberately the sibling
 * function rather than a shared generic: the two fields read alike today, but
 * a harness id is a select's value (never free text an operator can leave
 * whitespace in), so this skips the `.trim()` `modelEdit` needs.
 */
export function harnessEdit(current: string | undefined, draft: string): string | null | undefined {
  const before = current ?? "";
  if (draft === before) return undefined;
  return draft === "" ? null : draft;
}

/**
 * Which declared harness `harnessId` resolves to, and its `kind` — the
 * question the Harness & Model editor needs answered to know whether to show
 * the model field at all (only an `acp` harness has anywhere to forward it).
 *
 * `harnessId` absent or `""` means "the company default", the same contract
 * `AgentDetailDto.harness` and a blank select both carry: resolved against
 * whichever declared harness has `default: true`.
 */
export function resolvedHarnessKind(
  harnesses: HarnessDto[],
  harnessId: string | undefined,
): HarnessDto["kind"] | undefined {
  const id = harnessId || harnesses.find((h) => h.default)?.id;
  return harnesses.find((h) => h.id === id)?.kind;
}

/**
 * A harness's label in the picker's `<select>` — short enough for one line,
 * specific enough that an operator can tell two ACP entries apart by CLI.
 */
export function harnessOptionLabel(harness: HarnessDto): string {
  if (harness.kind === "acp") {
    const cli = harness.agent ?? "external";
    return harness.transport === "runner" ? `${cli} (remote) — ${harness.id}` : `${cli} — ${harness.id}`;
  }
  return `Managed — ${harness.id}`;
}

/** What an agent's tool grants amount to, once the intersection is applied. */
export interface ToolGrantSummary {
  /** What the agent actually holds. */
  effective: string[];
  /**
   * Globs the agent asked for that the company's allow-list does not cover, so
   * they are not grants however plainly the manifest lists them. This is the
   * line an operator checking a tool change is looking for.
   */
  dropped: string[];
  /**
   * Whether the agent lists no tools of its own and therefore inherits the
   * company's whole allow-list.
   *
   * The case worth naming: an empty `requested` reads like "no tools" and means
   * the opposite. A screen that showed an empty list here would tell an
   * operator their agent is powerless when it holds everything the company
   * allows.
   */
  standardGrant: boolean;
}

export function summarizeGrants(tools: AgentToolsDto): ToolGrantSummary {
  return {
    effective: tools.effective,
    dropped: tools.requested.filter((glob) => !tools.effective.includes(glob)),
    standardGrant: tools.requested.length === 0,
  };
}

/**
 * How an agent's tier reads on screen.
 *
 * `isOrchestrator` rather than `tier === "orchestrator"`: a company that tags
 * nobody still has an orchestrator (the first agent declared), and the host has
 * already resolved which one it is.
 */
export function tierLabel(detail: AgentDetailDto): string {
  return detail.isOrchestrator ? "Orchestrator" : "Worker";
}
