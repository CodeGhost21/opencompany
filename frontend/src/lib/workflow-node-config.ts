// The five withheld node kinds' config forms (issue #541, P4). The pure half:
// the field table both the create dialog and its renderer read from, and the
// hydrate/serialize/validate helpers that turn a saved node's free-form
// `config` into form strings and back.
//
// The engine (`vendor/openhuman/vendor/tinyflows`) parses and runs all five
// kinds already; only the console lacked controls to author their config, so
// `tool_call`/`http_request`/`switch`/`output_parser`/`sub_workflow` were kept
// off the palette (`CREATABLE_NODE_KINDS`) rather than shipping a node that
// errors at run time. These forms emit EXACTLY the keys the tinyflows executors
// read — verified against `catalog.rs`, `nodes/integration/*`, and
// OpenCompany's `translate.rs` (which lays a node's `config` over the derived
// defaults verbatim):
//
//   tool_call     { slug, args }
//   http_request  { method, url, headers, body }
//   switch        { field | expression }        (cases are EDGE labels, not config)
//   output_parser { schema, auto_fix }
//   sub_workflow  { workflow_id }
//
// `condition` is a core palette kind (not one of the withheld five), but the
// host now REQUIRES `config.field` at author time (#661 M1) — the engine
// truthiness-tests that expression, and without it a condition always routed
// `true`. So it carries a form too: `condition { field }` (the `yes`/`no`
// branches are EDGE labels, not config).
//
// Keys the host REJECTS inside `config` — `on_error`, `retry`,
// `requires_approval`, `schedule`, `destination`, `agent_ref` — are first-class
// node fields and are never emitted here (see `src/company/workflow_file.rs`).

/** How a config field is rendered. `workflow-ref` is a picker fed by the
 * company's other workflows (with a free-text fallback). */
export type ConfigControl = "line" | "json" | "select" | "workflow-ref";

/** One authored config field: its engine key, how it renders, and its rules. */
export interface ConfigFieldSpec {
  /** The engine config key this field writes (e.g. `slug`, `url`). */
  key: string;
  label: string;
  control: ConfigControl;
  placeholder?: string;
  /** A one-line explanation shown under the control. */
  hint?: string;
  /** Blocks submit when empty (checked in {@link configDraftProblem}). */
  required?: boolean;
  /** The draft's starting value for this field (else `""`). */
  default?: string;
  /** `select` options. An option whose `value` is `""` means "unset / omit". */
  options?: readonly { value: string; label: string }[];
  /** A `select` whose chosen value serializes as a JSON boolean (`auto_fix`). */
  boolean?: boolean;
  /**
   * For a `json` field, the value shape the engine key accepts. Absent = any
   * valid JSON. `object` rejects arrays/scalars (e.g. `args`, `headers`);
   * `object-or-string` also allows a bare JSON string (e.g. an HTTP `body`).
   */
  jsonShape?: "object" | "object-or-string";
}

/**
 * The per-kind field table. A kind absent from here has no config form — its
 * `config` (if any) rides through an edit untouched, as before.
 */
export const NODE_CONFIG_FIELDS: Record<string, readonly ConfigFieldSpec[]> = {
  condition: [
    {
      key: "field",
      label: "Field",
      control: "line",
      required: true,
      placeholder: "=item.approved",
      hint: "The boolean expression the branch tests. The `yes` edge takes the true branch, `no` the false.",
    },
  ],
  tool_call: [
    {
      key: "slug",
      label: "Tool slug",
      control: "line",
      required: true,
      placeholder: "e.g. web_search",
      hint: "The tool's id. Without it the engine falls back to the node id and runs the wrong tool.",
    },
    {
      key: "args",
      label: "Arguments",
      control: "json",
      jsonShape: "object",
      placeholder: '{ "query": "=item.topic" }',
      hint: "A JSON object of arguments. `=`-expressions resolve per input item at run time.",
    },
  ],
  http_request: [
    {
      key: "method",
      label: "Method",
      control: "select",
      default: "GET",
      options: [
        { value: "GET", label: "GET" },
        { value: "POST", label: "POST" },
        { value: "PUT", label: "PUT" },
        { value: "PATCH", label: "PATCH" },
        { value: "DELETE", label: "DELETE" },
      ],
    },
    {
      key: "url",
      label: "URL",
      control: "line",
      required: true,
      placeholder: "https://api.example.com/v1  or  =item.url",
      hint: "The request URL. May be an `=`-expression, e.g. `=item.url`.",
    },
    {
      key: "headers",
      label: "Headers",
      control: "json",
      jsonShape: "object",
      placeholder: '{ "Authorization": "Bearer …" }',
      hint: "A JSON object of request headers.",
    },
    {
      key: "body",
      label: "Body",
      control: "json",
      jsonShape: "object-or-string",
      placeholder: '{ "hello": "world" }',
      hint: "A JSON object sent as the body, or a JSON string like `\"raw text\"`.",
    },
  ],
  switch: [
    {
      key: "field",
      label: "Field",
      control: "line",
      placeholder: "e.g. status",
      hint: "A key on the first input item to branch on.",
    },
    {
      key: "expression",
      label: "Expression",
      control: "line",
      placeholder: "=item.score > 0.5",
      hint: "An `=`-expression to branch on. Give a field OR an expression.",
    },
  ],
  output_parser: [
    {
      key: "schema",
      label: "Schema",
      control: "json",
      placeholder: '{ "type": "object", "properties": { "name": { "type": "string" } } }',
      hint: "A JSON-Schema subset. Leave empty to pass the output through unchanged.",
    },
    {
      key: "auto_fix",
      label: "Auto-fix malformed output",
      control: "select",
      boolean: true,
      options: [
        { value: "", label: "Default (on)" },
        { value: "true", label: "On — repair to match the schema" },
        { value: "false", label: "Off" },
      ],
    },
  ],
  sub_workflow: [
    {
      key: "workflow_id",
      label: "Workflow to run",
      control: "workflow-ref",
      required: true,
      placeholder: "another workflow's id",
      hint: "The id of the workflow to run. It can't be this workflow's own id.",
    },
  ],
};

/**
 * A static caption shown under a kind's fields — for what the form deliberately
 * does NOT collect. `switch` cases are the biggest trap: they read like config
 * but are the labels on the node's outgoing edges.
 */
export const NODE_CONFIG_NOTES: Record<string, string> = {
  switch:
    "Cases aren't set here — they're the labels on this node's outgoing edges. An unlabeled edge is the `default` branch.",
};

/**
 * Config keys the host rejects inside `config` (they are first-class node
 * fields). Dropped, never preserved, so a form's output can never carry one —
 * even if a hand-authored graph smuggled it in.
 */
export const RESERVED_CONFIG_KEYS: readonly string[] = [
  "on_error",
  "retry",
  "requires_approval",
  "schedule",
  "destination",
  "agent_ref",
];

/** The config fields for `kind`, or `[]` when it has no config form. */
export function configFieldSpecs(kind: string): readonly ConfigFieldSpec[] {
  return NODE_CONFIG_FIELDS[kind] ?? [];
}

/** Whether `kind` is one of the five that author config through a form. */
export function hasConfigForm(kind: string): boolean {
  return kind in NODE_CONFIG_FIELDS;
}

/** A fresh draft for `kind`: one entry per field, each at its default (or `""`). */
export function blankConfigDraft(kind: string): Record<string, string> {
  const draft: Record<string, string> = {};
  for (const spec of configFieldSpecs(kind)) draft[spec.key] = spec.default ?? "";
  return draft;
}

/** A single field's value, as a form string. */
function stringifyConfigValue(spec: ConfigFieldSpec, value: unknown): string {
  if (value === undefined || value === null) return "";
  // `json` fields always round-trip through JSON so a stored raw string
  // (a valid `body`) comes back quoted and re-parses to the same string.
  if (spec.control === "json") return JSON.stringify(value, null, 2);
  if (spec.boolean) return value === true ? "true" : value === false ? "false" : String(value);
  return typeof value === "string" ? value : String(value);
}

/**
 * Hydrate a saved node's `config` into a form draft (issue #259/#541).
 *
 * Known keys become form strings. **Every other key is preserved verbatim in
 * `extra`** — the anti-data-loss guard: an orchestrator-authored node can carry
 * keys this form has no control for (`connection_ref` on a tool/http node,
 * `execution`/`concurrency`/`inputs` on a sub_workflow), and rebuilding the
 * node from the visible controls alone would silently delete them on the first
 * save. Reserved keys are the one exception — dropped, since the host would
 * reject them anyway.
 */
export function configDraftFrom(
  kind: string,
  config: unknown,
): { draft: Record<string, string>; extra: Record<string, unknown> } {
  const draft = blankConfigDraft(kind);
  const extra: Record<string, unknown> = {};
  const specs = configFieldSpecs(kind);
  const byKey = new Map(specs.map((s) => [s.key, s]));

  if (config && typeof config === "object" && !Array.isArray(config)) {
    for (const [key, value] of Object.entries(config as Record<string, unknown>)) {
      const spec = byKey.get(key);
      if (spec) {
        draft[key] = stringifyConfigValue(spec, value);
      } else if (!RESERVED_CONFIG_KEYS.includes(key)) {
        extra[key] = value;
      }
    }
  }
  return { draft, extra };
}

/**
 * Serialize a draft back into a node's `config`, or `undefined` when it would
 * be empty (so the field is omitted from the body).
 *
 * Empty fields are dropped; `json` fields are parsed (the caller has validated
 * them via {@link configDraftProblem}, so a parse here would be a programmer
 * error, not user input); the preserved `extra` bag is re-merged verbatim.
 */
export function configFromDraft(
  kind: string,
  draft: Record<string, string>,
  extra?: Record<string, unknown>,
): Record<string, unknown> | undefined {
  const config: Record<string, unknown> = {};
  for (const spec of configFieldSpecs(kind)) {
    const raw = (draft[spec.key] ?? "").trim();
    if (raw === "") continue;
    if (spec.control === "json") {
      config[spec.key] = JSON.parse(raw);
    } else if (spec.boolean) {
      config[spec.key] = raw === "true";
    } else {
      config[spec.key] = raw;
    }
  }
  if (extra) {
    for (const [key, value] of Object.entries(extra)) {
      if (!RESERVED_CONFIG_KEYS.includes(key)) config[key] = value;
    }
  }
  return Object.keys(config).length > 0 ? config : undefined;
}

/**
 * One field's own problem, raised on blur. `null` when fine.
 *
 * An EMPTY field is never flagged here — "you haven't filled this in yet" is
 * nagging, not feedback; emptiness is {@link configDraftProblem}'s business at
 * submit. Only a malformed `json` field has a rule of its own.
 */
export function configFieldProblem(spec: ConfigFieldSpec, value: string): string | null {
  const raw = value.trim();
  if (raw === "") return null;
  if (spec.control === "json") {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return `${spec.label} must be valid JSON.`;
    }
    return jsonShapeProblem(spec, parsed);
  }
  return null;
}

/** A plain JSON object — not an array, not `null`, not a scalar. */
function isJsonObject(value: unknown): boolean {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Whether a parsed `json` field's VALUE fits the shape the engine key wants.
 * `null` when fine (or when the field declares no shape, i.e. any JSON goes).
 * Catches `[]`/numbers/booleans slipping into an `args`/`headers` object, and
 * anything but an object-or-string for an HTTP `body`.
 */
function jsonShapeProblem(spec: ConfigFieldSpec, parsed: unknown): string | null {
  switch (spec.jsonShape) {
    case "object":
      return isJsonObject(parsed) ? null : `${spec.label} must be a JSON object.`;
    case "object-or-string":
      return isJsonObject(parsed) || typeof parsed === "string"
        ? null
        : `${spec.label} must be a JSON object or a JSON string.`;
    default:
      return null;
  }
}

/** The submit-time message for a missing required field. */
function requiredMessage(kind: string, spec: ConfigFieldSpec): string {
  if (kind === "tool_call" && spec.key === "slug") {
    return "A tool call needs a tool slug — name the tool it runs.";
  }
  if (kind === "http_request" && spec.key === "url") {
    return "An HTTP request needs a URL.";
  }
  if (kind === "sub_workflow" && spec.key === "workflow_id") {
    return "A sub-workflow needs the id of the workflow to run.";
  }
  return `${spec.label} is required.`;
}

/**
 * The first submit-blocking problem with a config draft, or `null` when it is
 * postable. Pure, so the dialog's `validate()` and the unit tests share it.
 *
 * Covers: required fields, malformed JSON, the `switch` field-or-expression
 * rule, and a `sub_workflow` pointed at its own id.
 */
export function configDraftProblem(
  kind: string,
  selfId: string,
  draft: Record<string, string>,
): string | null {
  const specs = configFieldSpecs(kind);
  if (specs.length === 0) return null;

  for (const spec of specs) {
    if (spec.required && !(draft[spec.key] ?? "").trim()) {
      return requiredMessage(kind, spec);
    }
  }
  for (const spec of specs) {
    const problem = configFieldProblem(spec, draft[spec.key] ?? "");
    if (problem) return problem;
  }

  if (kind === "switch") {
    const field = (draft.field ?? "").trim();
    const expression = (draft.expression ?? "").trim();
    if (!field && !expression) {
      return "A switch needs a field or an expression to branch on.";
    }
    if (field && expression) {
      return "A switch can use a field or an expression, not both.";
    }
  }
  if (kind === "sub_workflow") {
    const wid = (draft.workflow_id ?? "").trim();
    if (wid && selfId.trim() && wid === selfId.trim()) {
      return "A sub-workflow can't call itself — point it at a different workflow's id.";
    }
  }
  return null;
}
