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
 * tokens only.
 *
 * # Why it resolves the theme itself
 *
 * That independence has a cost that is easy to miss until you look at it in a
 * browser, which is how this was found: `next-themes` stamps `class="dark"` on
 * `<html>` from an **effect**, and a crash during the first render means that
 * effect never runs. Everything in `index.css` under `.dark` is therefore
 * unset, and the crash screen paints in full light on a machine that has never
 * shown a light pixel — a white flash, at the moment the operator is least
 * inclined to trust what they are looking at.
 *
 * So it reads `next-themes`' own `localStorage` key, falls back to the OS
 * preference, and puts `dark` on its own container. Tokens are custom
 * properties, so redefining them on that element is enough for everything
 * inside it.
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

/**
 * Whether to paint dark, resolved without `next-themes`.
 *
 * `"theme"` is that library's default storage key, and the values it writes are
 * `light` / `dark` / `system`. Anything else — including `system` and an absent
 * key — falls through to the OS preference, which is what `enableSystem` means
 * in `main.tsx`.
 *
 * Every access is guarded: a browser with site data blocked throws on the
 * `localStorage` getter itself, and a crash screen that crashes is the one
 * failure this component may not have.
 */
function prefersDark(): boolean {
  try {
    const stored = window.localStorage.getItem("theme");
    if (stored === "dark") return true;
    if (stored === "light") return false;
  } catch {
    // Fall through to the OS preference.
  }
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false;
  }
}

/** The message to show for a thrown value, which is not necessarily an `Error`. */
function describe(error: unknown): string {
  if (error instanceof Error) return error.message || error.name;
  if (typeof error === "string") return error;
  return "An unexpected error occurred.";
}

export function CrashFallback({ error, eventId, onReset }: CrashFallbackProps) {
  const [copied, setCopied] = useState(false);
  // Resolved once, at mount. Nothing can change it while this screen is up:
  // the theme toggle lives in the tree that just unmounted.
  const [dark] = useState(prefersDark);
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
      // `dark` on this element rather than on `<html>`: the tokens are custom
      // properties, so redefining them here covers everything inside, and this
      // component still touches nothing it does not own.
      className={`${dark ? "dark " : ""}bg-background text-foreground flex min-h-screen items-center justify-center p-6`}
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
