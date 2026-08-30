// The readable leaf of an engine error (issue #1366).
//
// The harness supplies useful operator prose, but its error wrapping arrives
// as a stack of implementation labels such as
// `harness error: capability error: agent: ...`. Those labels explain where
// the Rust error crossed a boundary, not what the operator should act on. Peel
// only the wrapper vocabulary the workflow contract owns; an unfamiliar prefix
// stays byte-for-byte intact rather than being mistaken for a wrapper.

import { WORKFLOW_NODE_KINDS } from "@/api/workflows";

const ENGINE_PREFIXES = new Set([
  "harness error",
  "capability error",
  ...WORKFLOW_NODE_KINDS,
]);

const PREFIX = /^([^:\r\n]+):[ \t]*/;

/**
 * Removes known harness, capability, and workflow-kind prefixes from an error.
 *
 * A raw message is the fallback whenever its first prefix is unknown or every
 * recognised prefix leads only to whitespace. That protects error messages
 * whose actual prose happens to contain a colon and means no information is
 * lost when the engine introduces a new wrapper.
 */
export function stripEnginePrefixes(full: string): string {
  let remaining = full;
  let stripped = false;

  while (true) {
    const match = PREFIX.exec(remaining);
    if (!match || !ENGINE_PREFIXES.has(match[1].trim().toLowerCase())) {
      return stripped ? remaining : full;
    }

    const next = remaining.slice(match[0].length);
    // A chain of labels without a human-readable leaf is less useful than the
    // original diagnostic, and returning the original preserves it verbatim.
    if (!next.trim()) return full;

    remaining = next;
    stripped = true;
  }
}
