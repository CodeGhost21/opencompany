// The declare wizard's pure logic: presets and the one assembly path from a
// wizard draft to the `LedgerSpec`-shaped body `defineLedger()` posts.
//
// Pulled out of the wizard component for the same reason `filteredEmptyNotice`
// and `statusFilterLabel` live in `api/ledgers.ts` rather than inside a view —
// the decision is the whole thing worth getting right, and it should be
// assertable without a render.
//
// # The wire shape, exactly
//
// `src/ledger/spec.rs`'s `LedgerSpec` is `#[serde(rename_all = "camelCase")]`
// at its own top level, but none of the keys this module writes at that level
// (`slug`, `title`, `purpose`, `fields`, `statuses`, `sections`, `checks`)
// contain an underscore, so that rename is invisible here. Its nested structs
// are not renamed: `StatusSpec` stays plain snake_case *on purpose* (issue
// #1266's near-miss, documented on `StatusSpec::needs_reason` itself) because
// it round-trips through every store backend as-is, so this module writes
// `needs_reason`, never `needsReason`, inside `statuses[]`. `Field` and
// `Section` have no snake_case keys to get wrong either way.

import type { FieldRole, LedgerField, LedgerSection } from "@/api/ledgers";

/** A status, as the wizard collects it — the wire shape's own field names,
 * so `buildLedgerSpec` below is a direct pass-through rather than a second
 * translation that could drift from the first. */
export interface WizardStatus {
  name: string;
  closed: boolean;
  needs_reason: boolean;
}

/** A row-detail field the wizard offers, beyond the three the engine always
 * needs (id, title, status — see {@link buildLedgerSpec}). */
export interface WizardField {
  name: string;
  role: FieldRole;
  description?: string;
}

export type StagePresetId = "todo-in-progress-done" | "open-closed" | "custom";

export interface StagePreset {
  id: StagePresetId;
  label: string;
  hint: string;
  statuses: WizardStatus[];
}

/**
 * The two presets step 3 leads with, plus the escape hatch.
 *
 * Both are worked examples of the one rule a custom stage list has to get
 * right on its own: something has to close, and a status that closes ought to
 * say why. `todo-in-progress-done` deliberately does not ask for a reason —
 * "done" is not a verdict the way "kept" or "broken" is — while
 * `open-closed` does, matching the shape `TEMPLATE` used to ship as the JSON
 * dialog's worked example (`kept`/`broken`, both `needs_reason: true`).
 */
export const STAGE_PRESETS: readonly StagePreset[] = [
  {
    id: "todo-in-progress-done",
    label: "To do / In progress / Done",
    hint: "A task-shaped list: each row moves through work toward finished.",
    statuses: [
      { name: "todo", closed: false, needs_reason: false },
      { name: "in_progress", closed: false, needs_reason: false },
      { name: "done", closed: true, needs_reason: false },
    ],
  },
  {
    id: "open-closed",
    label: "Open / Closed",
    hint: "An event-shaped list: each row waits, then resolves one way or another.",
    statuses: [
      { name: "open", closed: false, needs_reason: false },
      { name: "closed", closed: true, needs_reason: true },
    ],
  },
];

export function stagePreset(id: StagePresetId): StagePreset | undefined {
  return STAGE_PRESETS.find((preset) => preset.id === id);
}

export type FieldPresetId = "owner" | "notes" | "due-date";

export interface FieldPreset {
  id: FieldPresetId;
  label: string;
  field: WizardField;
}

/** The three details `TEMPLATE`'s worked example reached for beyond
 * title/status/reason, offered as toggles rather than typed by hand. */
export const FIELD_PRESETS: readonly FieldPreset[] = [
  { id: "owner", label: "Owner", field: { name: "owner", role: "owner" } },
  { id: "notes", label: "Notes", field: { name: "notes", role: "prose" } },
  { id: "due-date", label: "Due date", field: { name: "due", role: "date" } },
];

export function fieldPreset(id: FieldPresetId): FieldPreset | undefined {
  return FIELD_PRESETS.find((preset) => preset.id === id);
}

/**
 * Derives a slug from a title — `"Customer promises"` → `"customer-promises"`
 * — guaranteed to satisfy `src/ledger/spec.rs::normalize_slug`'s rule
 * (lowercase letters, digits and hyphens, up to 48 characters, never starting
 * or ending with one) or to be empty.
 *
 * `existing` disambiguates a collision the way a filesystem does: `-2`,
 * `-3`, … appended until the slug is free. An empty derivation (a title with
 * nothing slug-safe in it, e.g. "???") is returned as `""` — the caller's
 * signal to leave the field for the operator rather than silently writing a
 * slug nobody chose.
 */
export function slugify(title: string, existing: readonly string[] = []): string {
  const base = title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/g, "");
  if (!base) return "";
  const taken = new Set(existing);
  if (!taken.has(base)) return base;
  let n = 2;
  // 48 is the slug's own ceiling; the suffix needs room too.
  const room = 48 - String(n).length - 1;
  let candidate = `${base.slice(0, room)}-${n}`;
  while (taken.has(candidate)) {
    n += 1;
    const nextRoom = 48 - String(n).length - 1;
    candidate = `${base.slice(0, nextRoom)}-${n}`;
  }
  return candidate;
}

/** The three checks every wizard-built spec asks for — the same three
 * `TEMPLATE` shipped, and the only three the engine defines
 * (`src/ledger/spec.rs::Check`). Nothing a 4-step wizard produces can trip
 * them if the wizard itself is honest (a status that needs a reason always
 * gets a `reason` field — see {@link buildLedgerSpec}), so this is fixed
 * rather than asked about. */
export const WIZARD_CHECKS = ["required-field", "known-status", "closed-needs-reason"] as const;

export interface WizardDraft {
  purpose: string;
  title: string;
  slug: string;
  statuses: WizardStatus[];
  fields: WizardField[];
}

/**
 * The exact body `defineLedger()` posts.
 *
 * Kept loosely typed (`LedgerField`/`LedgerSection` from `api/ledgers.ts` are
 * the console's *read*-side types — camelCase `needsReason` — which is not
 * this shape; the request body's `statuses[]` stays snake_case per the module
 * doc above) rather than forcing a shared interface that would paper over
 * that one genuine difference between what the console reads and what it
 * posts.
 */
export interface WizardLedgerSpec {
  slug: string;
  title: string;
  purpose: string;
  fields: Array<Pick<LedgerField, "name" | "role"> & { description?: string; required?: boolean }>;
  statuses: WizardStatus[];
  sections: Array<Omit<LedgerSection, "cap"> & { cap?: number }>;
  checks: readonly string[];
}

/**
 * Assembles a `WizardDraft` into the spec the host accepts — the one path
 * both the review step's plain-language summary and the submit handler read,
 * so they cannot show one shape and post another.
 *
 * Always prepends the three fields the engine requires regardless of what the
 * wizard's own step asked about (`id`, `title`, `status`), and appends a
 * `reason` field — the field `REASON_FIELD` names in `src/ledger/spec.rs` —
 * exactly when some status in the draft sets `needs_reason`, so a spec that
 * demands a reason always has somewhere to put one. That auto-add is the
 * whole reason `WIZARD_CHECKS` can stay fixed: a wizard-built spec cannot
 * trip `closed-needs-reason` on its own.
 *
 * `sections` mirrors `TEMPLATE`'s own two-section shape: one grouping every
 * open status ("Outstanding"), one grouping every closed one ("Settled"),
 * newest-first within each — an operator who wanted a different layout still
 * has the same `POST …/ledgers` body reachable directly.
 */
export function buildLedgerSpec(draft: WizardDraft): WizardLedgerSpec {
  const needsReason = draft.statuses.some((status) => status.needs_reason);

  const fields: WizardLedgerSpec["fields"] = [
    { name: "id", role: "id" },
    { name: "title", role: "title", required: true },
    { name: "status", role: "status", required: true },
    ...draft.fields.map((field) => ({
      name: field.name,
      role: field.role,
      ...(field.description ? { description: field.description } : {}),
    })),
  ];
  if (needsReason) {
    fields.push({ name: "reason", role: "prose" });
  }

  const openNames = draft.statuses.filter((status) => !status.closed).map((s) => s.name);
  const closedNames = draft.statuses.filter((status) => status.closed).map((s) => s.name);
  const sections: WizardLedgerSpec["sections"] = [];
  if (openNames.length > 0) {
    sections.push({
      heading: "Outstanding",
      blurb: "Not yet closed. Most recently updated first.",
      statuses: openNames,
      order: "recent",
    });
  }
  if (closedNames.length > 0) {
    sections.push({
      heading: "Settled",
      blurb: needsReason
        ? "Closed, each with the reason."
        : "Closed.",
      statuses: closedNames,
      order: "recent",
    });
  }

  return {
    slug: draft.slug,
    title: draft.title,
    purpose: draft.purpose,
    fields,
    statuses: draft.statuses,
    sections,
    checks: WIZARD_CHECKS,
  };
}

/**
 * The review step's plain-language sentence, built from the same draft the
 * submit button posts — so what the operator reads and what the host
 * receives can never say two different things.
 */
export function summarize(draft: WizardDraft): string {
  const title = draft.title || "This list";
  const stageNames = draft.statuses.map((s) => s.name).join(" → ");
  const detailNames = draft.fields.map((f) => f.name);
  const details = detailNames.length > 0 ? `Each row has ${detailNames.join(", ")}.` : "";
  return `${title}, tracked ${stageNames}. ${details}`.trim();
}
