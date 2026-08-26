// Company creation: the pure half (issue #1807).
//
// Everything about provisioning a company that is decidable without a network
// or a DOM — building the minimal manifest the host accepts, and turning a
// refused provision into a sentence an operator can act on. Kept here, out of
// the dialog, so both are unit-testable as plain functions and the component
// stays a rendering concern.

import { ApiError } from "@/api/types";

/** What the New-company form collects. */
export interface ManifestInput {
  /** The company name — the one required field; the host derives the id from it. */
  name: string;
  /**
   * An email that may sign in as an admin without an invite first. Optional:
   * on a hosted tenant the manager injects `OPENCOMPANY_ADMIN_EMAIL` as a
   * standing admin, so a company provisioned with none is not a dead end.
   */
  adminEmail?: string;
  /**
   * The approval tier, when the operator overrode it. Omitted for the default:
   * the host records `[policy].mode = "auto"` for a manifest that names none,
   * so leaving it out is how the operator says "use the host default" rather
   * than pinning `auto` in the manifest text.
   */
  policyMode?: string;
}

/** The named escapes a TOML basic string gives control characters. */
const TOML_NAMED_ESCAPES: Record<string, string> = {
  "\b": "\\b",
  "\t": "\\t",
  "\n": "\\n",
  "\f": "\\f",
  "\r": "\\r",
};

/**
 * One TOML basic string, with the escapes the spec requires.
 *
 * A company name is operator-typed free text, so it can hold a quote, a
 * backslash, or a stray control character — each of which would otherwise
 * either break the parse or, worse, parse into something other than what was
 * typed. Escaping here is what lets `buildManifestToml` interpolate the value
 * without the caller having to sanitise it first.
 *
 * Built by walking code points rather than a control-character regex range so
 * the source carries no literal control byte of its own.
 */
function tomlString(value: string): string {
  let out = '"';
  for (const ch of value) {
    if (ch === "\\") {
      out += "\\\\";
    } else if (ch === '"') {
      out += '\\"';
    } else if (ch in TOML_NAMED_ESCAPES) {
      out += TOML_NAMED_ESCAPES[ch];
    } else if (ch.charCodeAt(0) < 0x20) {
      out += `\\u${ch.charCodeAt(0).toString(16).padStart(4, "0")}`;
    } else {
      out += ch;
    }
  }
  return `${out}"`;
}

/**
 * The smallest manifest that provisions the company the operator described.
 *
 * `[company].name` is the only section always present — the host injects the
 * policy tier and the user auth mode when the text omits them, so a name alone
 * is a complete, valid body (`server/provision.rs`). The two optional sections
 * are written only when the operator gave a value: an empty `[users].admins`
 * or a redundant `[policy].mode = "auto"` would say something the operator did
 * not, and the omitted-field form is exactly what the host reads as "use the
 * default".
 */
export function buildManifestToml(input: ManifestInput): string {
  const lines: string[] = ["[company]", `name = ${tomlString(input.name)}`];

  const email = input.adminEmail?.trim();
  if (email) {
    lines.push("", "[users]", `admins = [${tomlString(email)}]`);
  }

  const mode = input.policyMode?.trim();
  if (mode) {
    lines.push("", "[policy]", `mode = ${tomlString(mode)}`);
  }

  return `${lines.join("\n")}\n`;
}

/**
 * The provision-error codes this surface words specially.
 *
 * The host's own message is already prose for most refusals (a quota is "tenant
 * company quota of N reached", ownership is a full sentence ending "retry the
 * request"), so those are shown verbatim. Only two codes get a console-authored
 * line: `company_exists`, where the host names the id but the operator typed a
 * name, and the platform-scope refusal, which the console can explain in terms
 * of the sign-in rather than the raw `401`.
 */
export function describeProvisionError(err: unknown): string {
  if (!(err instanceof ApiError)) {
    return "Something went wrong creating the company. Try again.";
  }

  // A session cookie can never reach `PlatformScope`, whatever it holds. The
  // control is gated on `carriesPlatformBearer` so this should be unreachable
  // from the UI, but a race (a bearer that lost its scope, a token swapped mid
  // session) still lands here, and the honest answer is about the sign-in.
  if (err.status === 401) {
    return "This sign-in can't create companies — that needs a platform credential, which a person signed in here doesn't hold.";
  }

  switch (err.code) {
    case "company_exists":
      return "A company with that name already exists on this host. Choose a different name.";
    case "network_error":
      return err.message;
    // quota_exceeded, ownership_not_persisted, auth_mode_none_not_allowed,
    // manifest_parse, invalid_request and the manifest-validation envelope all
    // arrive as operator-readable prose from the host — show it verbatim.
    default:
      return err.message;
  }
}

/**
 * A fresh id for the company a reset provisions, guaranteed distinct from the
 * one just archived.
 *
 * The provisioning route derives an id from `[company].name` whenever the
 * request omits one (`company_id_from_name`, `runtime/builder.rs`), and the
 * reset dialog prefills its name field with the archived company's own name.
 * Left unset, the default Reset path would therefore hand the host the exact
 * id the archive just freed — and `RuntimeBuilder::build` loads any existing
 * durable `CompanyRecord` for an id before building over it, carrying its
 * `lifecycle`, ledger and overlays forward. The "clean" company would come
 * back archived, with the old company's history attached, instead of empty.
 *
 * Derived from the OLD id rather than the (editable) name field, so it stays
 * distinct from the archived company regardless of whether the operator
 * changes the name before submitting — the collision this guards against is
 * about the id, not the display name.
 */
export function resetReplacementId(oldId: string): string {
  const suffix =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `${oldId}-${suffix}`;
}

/**
 * Whether a failed archive attempt means the company is already gone, rather
 * than that the archive itself was refused.
 *
 * `client.lifecycle("archive", …)` is ambiguous by construction on a dropped
 * connection: `ApiClient.request` throws the same `network_error` whether the
 * request never reached the host, or it reached the host, archived and
 * removed the company from the registry, and only the *reply* was lost in
 * transit — there is no way to tell those apart from the caught exception
 * alone. A retry of that same archive call then answers `company_not_found`:
 * the id really is gone, but only because the earlier attempt already
 * removed it, not because the archive was refused. Without this check the
 * reset dialog reads that retry as a fresh failure and reports "nothing was
 * changed" — which is false — leaving the operator stuck retrying an archive
 * that already took, unable to ever reach the create leg (codex review on
 * #1828, PR comment 3861770485).
 */
export function wasAlreadyArchived(err: unknown): boolean {
  return err instanceof ApiError && err.code === "company_not_found";
}

/**
 * Whether a failed provision is worth reconciling against the host before
 * reporting it as a failure.
 *
 * `network_error` is ambiguous the same way {@link wasAlreadyArchived}
 * describes for the archive leg: `ApiClient.request` throws it whether the
 * request never reached the host, or it reached the host, provisioned the
 * company, and only the *reply* was lost in transit. `company_exists` looks
 * like a definitive refusal, but a retry of the exact same request lands on
 * it too — the host is naming the id it, itself, just created a moment
 * earlier. Left unreconciled, the operator sees "couldn't create" (or,
 * worse on a reset, "archived X, but couldn't create the new company") for a
 * company that in fact exists, with no way back into it from this dialog
 * (codex review on #1828, PR comment 3863028397).
 *
 * The caller reconciles by looking the id up with `client.status`, and MUST
 * only do that for an id this client generated itself
 * ({@link resetReplacementId}'s random suffix can't collide with a
 * pre-existing company) — reconciling an operator-typed id the same way
 * would risk switching the console into an unrelated company that happened
 * to already sit at that id, mistaking a genuine collision for its own
 * request.
 */
export function wasAmbiguousProvisionOutcome(err: unknown): boolean {
  return (
    err instanceof ApiError && (err.code === "network_error" || err.code === "company_exists")
  );
}

/**
 * Whether `candidateId`, typed as a reset's replacement id, would collide
 * with `archivedId` once the host's shared-single-DB tenant namespacing
 * (`AppConfig::namespaced_company_id`, `runtime/types.rs`) is applied.
 *
 * The console never learns the workload's tenant namespace — it isn't part
 * of `CompanyStatus` — but the encoding is self-describing: a tenant name may
 * never contain the `--` id delimiter (`validate_tenant_namespace`), so the
 * *first* `--` in an already-namespaced id unambiguously marks the tenant
 * boundary, and everything after it is the bare id the host derived from
 * before namespacing. A bare candidate equal to that tail would be
 * re-namespaced back to the exact archived id — `namespace_company_id`
 * re-derives the same `<tenant>--` prefix for a bare id, and is a no-op only
 * for one *already* carrying it — recreating the collision this whole guard
 * exists to prevent, just spelled without the prefix the operator may not
 * know their own company id carries (codex review on #1828, PR comment
 * 3862711330).
 */
export function collidesWithArchived(candidateId: string, archivedId: string): boolean {
  if (candidateId === archivedId) return true;
  const delimiter = archivedId.indexOf("--");
  return delimiter !== -1 && candidateId === archivedId.slice(delimiter + 2);
}
