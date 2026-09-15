import { Button } from "@/components/ui/button";

/**
 * "Use the same key for Composio?" — offered when the company's TinyHumans
 * account key exists but the managed Composio slot has nothing of its own
 * (keys rework, issue #2306, slice 4c;
 * `docs/key-reworks/phase-4c-reuse-banner.md` §4 step 10).
 *
 * Self-contained rather than shared: the LLM page's own banner (a separate,
 * concurrent dispatch's `frontend/src/inference/ReuseAccountKeyBanner.tsx`)
 * needs a model-choice follow-up step this one never does — a Composio copy
 * is a single-slot fill with nothing left to configure — so a shared
 * component would need a variant prop neither caller could give an honest
 * default for. Worth revisiting once both exist side by side: if the two
 * really do converge to a plain "Yes / Not now" question over a caption, a
 * later change can move this shape to `@/components` and have the LLM page's
 * simple case (no `needsModel`) use it too.
 *
 * Deliberately dumb: no fetch, no dismissal state, no visibility decision.
 * `ComposioSection.tsx` decides whether to render this at all
 * (`@/composio/reuse-banner`'s `showsComposioReuseBanner`) and owns every
 * callback's effect.
 */
export function ReuseAccountKeyBanner({
  testId,
  text,
  busy,
  onYes,
  onNotNow,
}: {
  /** Root test id. The two buttons append `-yes` / `-not-now`. */
  testId: string;
  /** The question, in full — this component adds no copy of its own. */
  text: string;
  /** Disables "Yes" while the copy is in flight. "Not now" stays clickable. */
  busy: boolean;
  onYes: () => void;
  onNotNow: () => void;
}) {
  return (
    <div
      role="status"
      data-testid={testId}
      className="flex flex-wrap items-center gap-3 rounded-md border border-border bg-muted/40 p-3 text-xs"
    >
      <span className="min-w-0 flex-1 text-foreground">{text}</span>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          size="sm"
          disabled={busy}
          data-testid={`${testId}-yes`}
          onClick={onYes}
        >
          Yes
        </Button>
        <Button
          size="sm"
          variant="ghost"
          data-testid={`${testId}-not-now`}
          onClick={onNotNow}
        >
          Not now
        </Button>
      </div>
    </div>
  );
}
