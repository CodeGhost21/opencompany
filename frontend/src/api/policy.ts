// The autonomy-tier API (issue #562): the console reads and writes the
// company's effective `[policy]` through the host's `.../policy` routes (REST,
// camelCase over the wire).
//
// The effective policy is the operator's console override where it sets a
// field, and the committed manifest `[policy]` everywhere else. The console
// never writes the manifest — a rebuild re-persists that from the seed, and for
// `[policy]` that is a deliberate security property — so a change here is a
// durable, attributed *override* the host resolves ahead of the manifest.
//
// Standalone functions over the shared client (mirrors `api/inference.ts`), so
// no change to `OpenCompanyClient` or the shared `api/types.ts` is needed.

import type { OpenCompanyClient } from "./client";

/**
 * One selectable tier, with the host's own words for what it means.
 *
 * The prose comes from the host rather than living here on purpose: it
 * describes what this runtime's approval gate actually does, and a copy in
 * TypeScript would drift from the behaviour it claims to describe. The list
 * also only contains tiers the host accepts, so a console built against a newer
 * or older host offers exactly what that host can honour.
 */
export interface PolicyTier {
  /** The `[policy].mode` word. */
  value: string;
  /** The operator-facing label. */
  label: string;
  /** What choosing it means, in consequences rather than tier vocabulary. */
  description: string;
}

/** The company's effective policy, plus what a reset would restore. */
export interface PolicyStatus {
  /** The tier actually in force. */
  mode: string;
  /**
   * The always-ask list actually in force — the operator's real lever. It wins
   * over every tier, `full` included.
   */
  alwaysApprove: string[];
  /** Spend strictly under this amount without an approval; `null` means no cap. */
  autoApproveUnderUsd: number | null;
  /** How long an undecided approval remains actionable. */
  approvalTtlHours: number;
  /** The manifest's tier, so "reset" can name what it would restore. */
  manifestMode: string;
  /** The manifest's always-ask list, for the same reason. */
  manifestAlwaysApprove: string[];
  /** The manifest's spend cap before a console override. */
  manifestAutoApproveUnderUsd: number | null;
  /** The manifest's configured deadline, or `null` when it uses the default. */
  manifestApprovalTtlHours: number | null;
  /**
   * Whether an operator override is in force.
   *
   * Deliberately not derivable by comparing the values: an override that
   * happens to match the manifest is still an override, and is still what a
   * reset would remove.
   */
  overridden: boolean;
  /** Who set the override, if one is set. */
  setBy?: string;
  /** When it was set (epoch millis), if one is set. */
  setAtMillis?: number;
  /** The selectable tiers, in increasing order of autonomy. */
  tiers: PolicyTier[];
  /**
   * When a change bites, in the host's words.
   *
   * Rendered rather than paraphrased: a tier change lands on the company's
   * NEXT turn, so a turn already running finishes under the previous tier.
   * Since "stop the flood now" is what an operator comes here to do, that gap
   * is worth stating rather than leaving them to discover.
   */
  takesEffect: string;
  /**
   * Every tool name this build's approval gate can match — the complete
   * registry, not the workflow-authorable subset served by
   * `/workflows/tool-slugs`. The "is this a real tool?" note under the field
   * compares against this when the host serves it, so a wired agent tool
   * (`hosting_launch_site`, `publish_artifact`) is never called a mistake just
   * because it cannot be a workflow node. Absent on a host predating the field.
   */
  knownTools?: string[];
}

/**
 * The set-policy body. Omit a field to leave it alone; send `null` to stop
 * overriding it.
 *
 * `alwaysApprove: []` is an operator deliberately clearing the always-ask list,
 * which is NOT the same as omitting the field. Sending neither field is a 422
 * rather than a silent no-op.
 */
export interface SetPolicyInput {
  mode?: string | null;
  alwaysApprove?: string[] | null;
  /** `null` means no spend cap; omit to leave the cap alone. */
  autoApproveUnderUsd?: number | null;
  /** `null` stops overriding the deadline; omit to leave it alone. */
  approvalTtlHours?: number | null;
}

/**
 * One renderable tier.
 *
 * Split out because `Array.isArray(tiers)` alone is not the fence it looks
 * like: `{"tiers": [null]}` satisfies it, and the very next thing every reader
 * does is `tiers.find((tier) => tier.value === ...)`, which throws on the first
 * member. A body that passes the shape check and then crashes the render is the
 * exact failure {@link isPolicyStatus} exists to stop, one level down.
 *
 * All three fields are required rather than optional, and that is safe against
 * a real host: `TierDto` (`src/server/ops/policy.rs:96`) declares `value`,
 * `label` and `description` as `&'static str`, so every tier the runtime serves
 * carries all three. The leniency this fence deliberately keeps is at the
 * *status* level — an optional field a host may not have grown yet — not here,
 * where a partial member is a member the menu cannot draw.
 */
function isPolicyTier(value: unknown): value is PolicyTier {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const tier = value as Partial<PolicyTier>;
  return (
    typeof tier.value === "string" &&
    typeof tier.label === "string" &&
    typeof tier.description === "string"
  );
}

/**
 * What a *renderable* policy is, as distinct from what the host sent.
 *
 * ## The crash this fences
 *
 * A 200 whose body is not a policy used to reach the render untouched, and the
 * first `status.tiers.find(...)` threw. `AutonomyPill` is mounted for the entire
 * life of the console on every view (`window-title-bar.tsx`), there is no error
 * boundary anywhere in `src/`, and React unmounts the whole tree on a throw
 * during render — so one malformed `/policy` response blanked the ENTIRE
 * console, on every page, with no way back but a reload. The console E2E lane
 * met it as `TypeError: Cannot read properties of undefined (reading 'find')`
 * against an empty document, and thirty-plus specs sat on their timeouts.
 *
 * ## Why it is a predicate here and not a check on the fetch
 *
 * Because "usable" depends on the reader. `useApprovalDeadline` wants
 * `approvalTtlHours` and documents that an older host omits it; making the
 * transport insist on the tier list would break a hook that is deliberately
 * lenient. So the three functions below stay plain typed requests and each
 * consumer that puts a policy ON SCREEN gates on this instead — `useAutonomy`
 * for the title row, `apply`/`load` for the settings page.
 *
 * ## Why these three fields and no more
 *
 * They are exactly what the render paths dereference without a guard —
 * `tiers.find`, `tiers.map`, `alwaysApprove.join` — plus the `mode` they
 * compare against. Every optional field stays optional: this is a crash fence,
 * not a schema, and a host that grows or drops a field must keep working. See
 * `knownTools` above.
 *
 * The two required lists are checked to their MEMBERS, though, because the
 * container check alone does not fence what it is here to fence: `tiers: [null]`
 * is an array, passes, and throws inside `tiers.find` on the very next line —
 * the same blank console this predicate was written to stop. See
 * {@link isPolicyTier}.
 */
export function isPolicyStatus(body: unknown): body is PolicyStatus {
  if (typeof body !== "object" || body === null || Array.isArray(body)) return false;
  const candidate = body as Partial<PolicyStatus>;
  return (
    typeof candidate.mode === "string" &&
    Array.isArray(candidate.tiers) &&
    // Each MEMBER, not just the container. See `isPolicyTier`: `[null]` passes
    // `Array.isArray` and throws in `tiers.find` a moment later, which is the
    // same blank console by a slightly longer route.
    candidate.tiers.every(isPolicyTier) &&
    Array.isArray(candidate.alwaysApprove) &&
    // The list is rendered with `.join(", ")` and typed `string[]`, and a
    // non-string member is a wrong sentence rather than a crash — the settings
    // page would seed its always-ask box with "[object Object]" and save that
    // back as a gate. A policy the console cannot state truthfully is not a
    // renderable policy.
    candidate.alwaysApprove.every((entry) => typeof entry === "string")
  );
}

/**
 * What a reader says when it is handed something that is not a policy.
 *
 * One sentence, shared, so the settings page's `loadError` and the title row's
 * failed write read as the same fact rather than as two unrelated faults.
 */
export const NOT_A_POLICY = "The host did not answer with an autonomy policy.";

/** The company's effective policy. */
export function getPolicy(
  client: OpenCompanyClient,
  company: string | null,
): Promise<PolicyStatus> {
  return client.get<PolicyStatus>(`${client.scopeFor(company)}/policy`);
}

/** Set the tier and/or the always-ask list. Admin-only, attributed. */
export function setPolicy(
  client: OpenCompanyClient,
  company: string | null,
  body: SetPolicyInput,
): Promise<PolicyStatus> {
  return client.put<PolicyStatus>(`${client.scopeFor(company)}/policy`, body);
}

/** Drop the override so the manifest's `[policy]` applies again. */
export function resetPolicy(
  client: OpenCompanyClient,
  company: string | null,
): Promise<PolicyStatus> {
  return client.del<PolicyStatus>(`${client.scopeFor(company)}/policy`);
}
