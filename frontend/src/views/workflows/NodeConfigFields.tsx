// The config controls for the five withheld node kinds (issue #541), rendered
// from one spec table so every kind's form stays in step with the engine keys
// it emits. Mirrors `AgentFields` (issue #264): a pure, spec-driven renderer
// with no per-kind branches beyond the control switch.

import type { WorkflowSummary } from "@/api/workflows";
import {
  NODE_CONFIG_NOTES,
  configFieldSpecs,
  type ConfigFieldSpec,
} from "@/lib/workflow-node-config";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

/** A Base UI `SelectItem` cannot carry an empty string value, so the "unset"
 * option (`value: ""` in the spec) stands in as this sentinel. */
const SELECT_UNSET = "__unset__";

/**
 * A kind's config fields (issue #541).
 *
 * `draft` holds one string per field, keyed by the field's engine key; the
 * dialog owns it and threads changes back through `onChange`. `errors` carries
 * blur-time problems keyed the same way. `workflows` feeds the `workflow-ref`
 * picker (the sub_workflow target) and `selfId` drops the graph's own id from
 * it — a sub-workflow can't call itself.
 */
export function NodeConfigFields({
  idPrefix,
  kind,
  draft,
  errors,
  workflows,
  selfId,
  onChange,
  onValidate,
}: {
  idPrefix: string;
  kind: string;
  draft: Record<string, string>;
  errors: Record<string, string | undefined>;
  workflows: WorkflowSummary[];
  selfId: string;
  onChange: (key: string, value: string) => void;
  onValidate: (key: string, value: string) => void;
}) {
  const specs = configFieldSpecs(kind);
  if (specs.length === 0) return null;
  const note = NODE_CONFIG_NOTES[kind];

  return (
    <div
      className="grid gap-2 rounded-md border border-dashed p-2"
      data-testid={`node-config-${kind}`}
    >
      {specs.map((spec) => {
        const id = `${idPrefix}-config-${spec.key}`;
        const errorId = `${id}-error`;
        const value = draft[spec.key] ?? "";
        const error = errors[spec.key];
        return (
          <div key={spec.key} className="grid gap-1">
            <Label htmlFor={id} className="text-xs">
              {spec.label}
              {spec.required && <span className="text-destructive"> *</span>}
            </Label>
            <Control
              id={id}
              spec={spec}
              value={value}
              error={error}
              errorId={errorId}
              workflows={workflows}
              selfId={selfId}
              onChange={(v) => onChange(spec.key, v)}
              onBlurValidate={(v) => onValidate(spec.key, v)}
            />
            {error && (
              <p id={errorId} className="text-[11px] leading-snug text-destructive">
                {error}
              </p>
            )}
            {spec.hint && (
              <p className="text-[10px] leading-snug text-muted-foreground">{spec.hint}</p>
            )}
          </div>
        );
      })}
      {note && <p className="text-[10px] leading-snug text-muted-foreground">{note}</p>}
    </div>
  );
}

/** The one control switch — the only place a field's `control` is read. */
function Control({
  id,
  spec,
  value,
  error,
  errorId,
  workflows,
  selfId,
  onChange,
  onBlurValidate,
}: {
  id: string;
  spec: ConfigFieldSpec;
  value: string;
  error?: string;
  errorId: string;
  workflows: WorkflowSummary[];
  selfId: string;
  onChange: (value: string) => void;
  onBlurValidate: (value: string) => void;
}) {
  const describedBy = error ? errorId : undefined;
  const ariaLabel = spec.label;

  if (spec.control === "json") {
    return (
      <Textarea
        id={id}
        rows={3}
        className="font-mono text-xs"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={(e) => onBlurValidate(e.target.value)}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        placeholder={spec.placeholder}
        aria-label={ariaLabel}
        data-testid={`config-field-${spec.key}`}
      />
    );
  }

  if (spec.control === "select") {
    return (
      <Select
        value={value === "" ? SELECT_UNSET : value}
        onValueChange={(v) => onChange(!v || v === SELECT_UNSET ? "" : v)}
      >
        <SelectTrigger className="h-8" aria-label={ariaLabel}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {(spec.options ?? []).map((opt) => (
            <SelectItem key={opt.value || SELECT_UNSET} value={opt.value || SELECT_UNSET}>
              {opt.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  }

  if (spec.control === "workflow-ref") {
    // A picker fed by the company's other workflows, mirroring the agent-node
    // roster→free-text fallback: when the host offers no roster (or none but
    // this graph), an author can still type an id by hand.
    const others = workflows.filter((w) => w.id !== selfId.trim());
    if (others.length > 0) {
      return (
        <Select value={value} onValueChange={(v) => onChange(v ?? "")}>
          <SelectTrigger className="h-8" aria-label={ariaLabel}>
            <SelectValue placeholder="Pick a workflow" />
          </SelectTrigger>
          <SelectContent>
            {others.map((w) => (
              <SelectItem key={w.id} value={w.id}>
                {w.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }
    return (
      <Input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={(e) => onBlurValidate(e.target.value)}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy}
        placeholder={spec.placeholder}
        aria-label={ariaLabel}
        data-testid={`config-field-${spec.key}`}
      />
    );
  }

  // `line`
  return (
    <Input
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onBlur={(e) => onBlurValidate(e.target.value)}
      aria-invalid={Boolean(error)}
      aria-describedby={describedBy}
      placeholder={spec.placeholder}
      aria-label={ariaLabel}
      data-testid={`config-field-${spec.key}`}
    />
  );
}
