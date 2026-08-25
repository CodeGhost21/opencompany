import type { ReactNode } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { AGENT_FIELDS, type AgentDraft, type AgentFieldKey } from "@/lib/agent";

/**
 * An agent's authored fields, rendered from one definition (issue #264).
 *
 * Used by both "Add teammate" and the detail view's edit form. They collect
 * the same three things, and rendering them from
 * [`AGENT_FIELDS`](@/lib/agent) is what keeps the labels, the placeholders and
 * the order the same in both places rather than the same by coincidence.
 *
 * `copilot` renders the drafting control under a field (issue #1776). A render
 * prop rather than a set of handlers, because the two surfaces ask the host
 * different questions — the detail view addresses a teammate by id, the
 * Add-teammate form sends the role being typed — and neither difference is this
 * component's business. What IS its business is the rule about *where* the
 * control may appear, and that rule lives here so it cannot be half-applied:
 * only under a `prose` field, and never under a locked one. Offering to draft
 * text into a box the host will refuse to store is a dead end, and drafting a
 * `name` or a `role` is deliberately not on the table — a role is what
 * delegation grounds on, so a drafted one would change who the company routes
 * work to.
 *
 * `readOnly` is a predicate rather than a boolean because editability is
 * per-field and decided by the **host**: the detail response carries an
 * `editable` list, and a manifest teammate's fields are not in it. A locked
 * field is still rendered, because "you cannot change this here" is information
 * an operator needs; hiding it would just recreate the dead end.
 *
 * Locked means the native `readOnly`, NOT `disabled`. A disabled input is
 * removed from the tab order and from the accessibility tree's interactive
 * surface, so the very values this screen exists to show would be unreachable
 * by keyboard and awkward to select or copy. `readOnly` refuses the edit and
 * keeps the value reachable, which is the behaviour a read-only field wants.
 */
export function AgentFields({
  idPrefix,
  draft,
  onChange,
  readOnly,
  copilot,
}: {
  /** Namespaces the DOM ids, so two of these can be mounted at once. */
  idPrefix: string;
  draft: AgentDraft;
  onChange: (key: AgentFieldKey, value: string) => void;
  /** Whether a given field is read-only. Defaults to all-editable. */
  readOnly?: (key: AgentFieldKey) => boolean;
  /**
   * The drafting control for one field, when this surface offers one. Called
   * only for an editable `prose` field; omit it and the fields render exactly
   * as they did before.
   */
  copilot?: (key: AgentFieldKey) => ReactNode;
}) {
  return (
    <>
      {AGENT_FIELDS.map((field) => {
        const id = `${idPrefix}-${field.key}`;
        const locked = readOnly?.(field.key) ?? false;
        return (
          <div key={field.key} className="grid gap-2">
            <Label htmlFor={id}>{field.label}</Label>
            {field.kind === "prose" ? (
              <Textarea
                id={id}
                rows={field.rows ?? 4}
                value={draft[field.key]}
                readOnly={locked}
                onChange={(e) => onChange(field.key, e.target.value)}
                placeholder={field.placeholder}
                data-testid={`agent-field-${field.key}`}
              />
            ) : (
              <Input
                id={id}
                value={draft[field.key]}
                readOnly={locked}
                onChange={(e) => onChange(field.key, e.target.value)}
                placeholder={field.placeholder}
                data-testid={`agent-field-${field.key}`}
              />
            )}
            {field.kind === "prose" && !locked && copilot?.(field.key)}
          </div>
        );
      })}
    </>
  );
}
