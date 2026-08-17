// The first-run setup API (`/api/v1/setup`): read what this instance is
// configured with, and apply a completed wizard.
//
// Standalone functions over the shared client, mirroring `api/policy.ts` and
// `api/inference.ts`, so neither `OpenCompanyClient` nor the shared
// `api/types.ts` needs to change.
//
// Field names are snake_case here rather than the camelCase most console
// surfaces use, matching the host's own DTOs. That is deliberate: half the
// payload's content *is* `config.toml` keys (`workspace.max_blob_mb`), and
// camel-casing the wrapper around snake_case keys reads worse than being
// consistent with the file the whole surface exists to write.

import type { OpenCompanyClient } from "./client";

/** Which precedence layer supplied a field's current value. */
export type ConfigLayer = "env" | "config.toml" | "manifest" | "default";

/**
 * One configurable setting, with the layer that currently owns it.
 *
 * The layer is the important part. Config resolution is
 * `env ⟵ config.toml ⟵ manifest ⟵ default`, and setup can only write the
 * `config.toml` layer — so on a hosted instance, where the control plane
 * injects `OPENCOMPANY_*`, an edit to an env-owned field would write a file
 * that the next boot ignores. `editable` is the host's answer to that, and the
 * UI must render a non-editable field read-only rather than letting an operator
 * submit a change that will be refused.
 */
export interface SetupField {
  /** The dotted `config.toml` key, e.g. `bind` or `workspace.max_blob_mb`. */
  key: string;
  /** The value held in `config.toml`, or null when the file does not set it. */
  value: string | null;
  /** Which layer supplied it. */
  layer: ConfigLayer;
  /** False when `env` owns the field — `config.toml` cannot outrank it. */
  editable: boolean;
  /** Whether a change takes effect only after the host restarts. */
  requires_restart: boolean;
  /** A credential: `value` is always null, and the UI shows only its status. */
  secret: boolean;
}

/** A company template this instance can be seeded from. */
export interface SetupTemplate {
  /** Stable preset slug, e.g. `agentic_marketing_agency`. */
  id: string;
  /** Human-readable name. */
  name: string;
  /** How many agents the template's roster declares. */
  agent_count: number;
  /** What a company built from this template produces. */
  output: string | null;
}

/**
 * Which optional surfaces are compiled into the host's build.
 *
 * These are cargo features, not settings — nothing the wizard writes turns one
 * on. They exist so the flow can say "not in this build" instead of offering a
 * switch that does nothing.
 */
export interface SetupBuild {
  acp_in_build: boolean;
  /**
   * Whether the ACP JSON-RPC transport is actually mounted, which is a separate
   * question from whether the feature is compiled in: the host builds the ACP
   * session model but mounts no `/acp` route, so a client would get a 404 even
   * on a build with the feature on.
   */
  acp_transport_mounted: boolean;
  mcp_in_build: boolean;
  harness_in_build: boolean;
  oauth_in_build: boolean;
}

/** Everything the wizard needs to draw itself. */
export interface SetupStatus {
  /** Whether setup has already been completed on this instance. */
  complete: boolean;
  /** Absolute path of the `config.toml` a write lands in. */
  config_path: string;
  /** Every configurable field, in a stable order. */
  fields: SetupField[];
  /** The company templates this build ships. */
  templates: SetupTemplate[];
  /** Sign-in modes this host accepts. `none` is absent on a routable bind. */
  auth_modes: string[];
  /** Which optional surfaces this build has. */
  build: SetupBuild;
  /** Companies already registered. Non-empty means the seed step is skipped. */
  companies: string[];
}

/**
 * A completed wizard.
 *
 * A `null` field value clears the key, letting the next precedence layer supply
 * it — which is not the same as sending `""`, a set-but-empty value that would
 * shadow the layer below instead of deferring to it.
 */
export interface SetupInput {
  fields: Record<string, string | null>;
  /** Ignored by the host when a company already exists. */
  template?: string | null;
}

/** What an apply reports back. */
export interface SetupApplied {
  complete: boolean;
  config_path: string;
  /** Keys whose new value only takes effect after a restart. */
  restart_required: string[];
  /** The company seeded by this call, if any. */
  seeded_company: string | null;
}

/** Read this instance's setup state. */
export function getSetup(client: OpenCompanyClient): Promise<SetupStatus> {
  return client.get<SetupStatus>("/api/v1/setup");
}

/** Apply a completed wizard. All-or-nothing: a refusal writes nothing. */
export function submitSetup(
  client: OpenCompanyClient,
  body: SetupInput,
): Promise<SetupApplied> {
  return client.post<SetupApplied>("/api/v1/setup", body);
}

/** The subset of fields a given wizard step owns, in payload order. */
export function fieldsFor(status: SetupStatus, keys: readonly string[]): SetupField[] {
  return keys
    .map((key) => status.fields.find((f) => f.key === key))
    .filter((f): f is SetupField => f !== undefined);
}

/**
 * The fields a wizard should actually submit, given the form's current values.
 *
 * Pure, and separate from the component, because three rules meet here and each
 * one silently corrupts a config if it is wrong:
 *
 *   1. **Unchanged fields are omitted.** Re-sending a field's existing value is
 *      harmless for most keys but would re-assert a value the operator never
 *      looked at, turning "I set the bind address" into "I confirmed all
 *      thirteen of these", which is not what they did.
 *   2. **Env-owned fields are never sent.** The host refuses them, and since an
 *      apply is all-or-nothing, including one would fail the whole submission
 *      over a field the form rendered read-only in the first place.
 *   3. **Secrets are write-only.** The host never echoes a credential, so
 *      "unchanged" cannot be detected by comparison — an empty box means "leave
 *      it alone", never "clear it". Only a typed value is sent.
 *
 * A cleared (empty) editable field becomes `null`, which deletes the key so the
 * next precedence layer applies. Sending `""` instead would write a
 * set-but-empty value that shadows that layer rather than deferring to it.
 */
export function changedFields(
  status: SetupStatus,
  values: Record<string, string>,
): Record<string, string | null> {
  const out: Record<string, string | null> = {};
  for (const f of status.fields) {
    const typed = values[f.key];
    if (f.secret) {
      // Rule 3 — and rule 2 still applies: an env-owned secret is not writable.
      if (f.editable && typed !== undefined && typed !== "") out[f.key] = typed;
      continue;
    }
    if (!f.editable) continue; // rule 2
    // Absent is "untouched", not "cleared". The two are only the same when the
    // form has an entry for every field, which is true today (the wizard seeds
    // from the file on load) and is exactly the kind of invariant that stops
    // being true quietly. Reading absent as a clear would delete a key the
    // operator never saw, on a step they skipped.
    if (typed === undefined) continue;
    if (typed === (f.value ?? "")) continue; // rule 1
    out[f.key] = typed === "" ? null : typed;
  }
  return out;
}
