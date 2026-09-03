/**
 * What the console paints when a render crashes hard enough that nothing else
 * can (`docs/spec/runtime/crash-reporting.md`).
 *
 * # Why it exists
 *
 * Before this there was no top-level error boundary at all: a throw inside any
 * component unmounted the whole React tree and left a white page. The operator
 * saw nothing, could do nothing, and — since the crash never reached a
 * reporting seam either — nobody found out. This is the seam and the screen.
 *
 * # Why it depends on nothing
 *
 * It renders OUTSIDE `ThemeProvider`, `TooltipProvider` and the connection
 * registry, because the thing that crashed may be one of them. So: no `Button`,
 * no `Tooltip`, no context, no hook but `useState`. Plain elements and design
 * tokens, which are defined on `:root` in `index.css` and therefore resolve
 * whether or not `next-themes` ever got as far as stamping a class.
 *
 * # The event id, and why it is only sometimes shown
 *
 * Sentry hands the boundary an event id whether or not reporting is switched
 * on — the SDK mints one locally. Showing it on an install with no DSN would
 * give the operator a reference nobody can look up, which costs them a support
 * round trip to find out. It is shown only when an event was actually sent.
 */

import { useState } from "react";

export interface CrashFallbackProps {
  /** What was thrown. Rendered as a message only; never as HTML. */
  error: unknown;
  /** The Sentry event id, when crash reporting is on and sent one. */
  eventId?: string | null;
  /** Re-mounts the tree that crashed, for a fault that was transient. */
  onReset: () => void;
}

/** The message to show for a thrown value, which is not necessarily an `Error`. */
function describe(error: unknown): string {
  if (error instanceof Error) return error.message || error.name;
  if (typeof error === "string") return error;
  return "An unexpected error occurred.";
}

export function CrashFallback({ error, eventId, onReset }: CrashFallbackProps) {
  const [copied, setCopied] = useState(false);
  const reference = typeof eventId === "string" && eventId.length > 0 ? eventId : null;

  const copyReference = () => {
    if (!reference) return;
    void navigator.clipboard?.writeText(reference).then(
      () => setCopied(true),
      // A clipboard the browser refuses is not worth an error state: the id is
      // on screen and selectable either way.
      () => setCopied(false),
    );
  };

  return (
    <div
      role="alert"
      className="bg-background text-foreground flex min-h-screen items-center justify-center p-6"
    >
      <div className="w-full max-w-md space-y-4">
        <h1 className="text-lg font-semibold">The console hit an error</h1>
        <p className="text-muted-foreground text-sm">
          Something failed while rendering this page. Your company and its data are
          unaffected — this is the browser only.
        </p>
        <p className="border-border bg-muted text-muted-foreground overflow-x-auto rounded-md border p-3 font-mono text-xs">
          {describe(error)}
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onReset}
            className="border-border hover:bg-accent rounded-md border px-3 py-1.5 text-sm"
          >
            Try again
          </button>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="border-border hover:bg-accent rounded-md border px-3 py-1.5 text-sm"
          >
            Reload the console
          </button>
        </div>
        {reference ? (
          <p className="text-muted-foreground text-xs">
            Reference{" "}
            <button
              type="button"
              onClick={copyReference}
              className="text-foreground font-mono underline underline-offset-2"
            >
              {reference}
            </button>
            {copied ? " · copied" : null}
          </p>
        ) : null}
      </div>
    </div>
  );
}
