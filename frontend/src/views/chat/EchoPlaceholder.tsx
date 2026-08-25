import type { CognitionState } from "@/api/types";

/**
 * The marker that says a line was produced by the offline echo brain rather
 * than the teammate whose name and avatar it is rendered under (issue #1734).
 *
 * Lives in its own module because two surfaces render an author line: the
 * channel transcript (`MessageRow`) and the thread panel (`ThreadPanel`, which
 * has its own compact `Line`). A reply read inside a thread is exactly the same
 * false attribution as one read in the channel, and the first cut of this fix
 * marked only the first of them — so the marker, and the sentence behind it,
 * are one copy that both import rather than two that can drift.
 */

/**
 * The cognition states that mean this company's replies came from the echo
 * brain, or `null` when they did not — or when the host never said.
 *
 * `undefined` (an older host, or one that could not answer) is **unknown**, and
 * unknown is not an echo. Asserting one on a host that cannot be asked would be
 * the same unfounded claim this fix removes, pointed the other way.
 */
export function echoCause(cognition: CognitionState | null | undefined): CognitionState | null {
  return cognition === "unconfigured" || cognition === "unavailable" ? cognition : null;
}

/**
 * Why this line is not the named teammate's words, in the operator's terms.
 *
 * The two causes get different sentences because they have different remedies,
 * which is the whole reason the host reports a discriminated state instead of a
 * boolean. A tooltip that says "no model configured" on a host with no harness
 * contradicts the banner directly above it, which is telling that same operator
 * that no setting will help.
 */
function reason(author: string, cause: CognitionState): string {
  const why =
    cause === "unconfigured"
      ? "The company has no model configured"
      : "No agent harness is available on this host";
  return `${author} did not write this. ${why}, so the offline echo brain answered instead.`;
}

/**
 * The chip itself. Short, because it sits on every company-side row; the
 * sentence is the tooltip, following the `disabledReason` idiom `MessageRow`
 * uses — a label that just changes shape reads as a bug, so the reason is
 * available without leaving the row.
 */
export function EchoPlaceholder({ author, cause }: { author: string; cause: CognitionState }) {
  return (
    <span
      data-testid="chat-echo-placeholder"
      title={reason(author, cause)}
      className="shrink-0 rounded-full bg-muted px-1.5 py-px text-2xs font-medium text-muted-foreground"
    >
      Placeholder
    </span>
  );
}
