import { useState } from "react";
import { ArrowDownToLine, Loader2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAppUpdate, type UseAppUpdateOptions } from "@/hooks/use-app-update";
import { isActionable, shouldResurface, updateHeadline, updateSummary } from "@/lib/app-update";
import { cn } from "@/lib/utils";

/**
 * The desktop update banner: bottom-right, and silent almost always.
 *
 * Mounted once, for the whole session, in `App.tsx` — inside `HostsProvider`
 * and *beside* the console rather than within it, so switching or losing a host
 * does not unmount it. `AppShell` is a different component and is not where
 * this lives; putting it there would tie a banner about replacing the
 * application to whether a company is on screen.
 *
 * It renders nothing in a
 * browser and nothing while the shell is checking, finding or downloading a new
 * build — see `lib/app-update.ts` for why those three states are deliberately
 * invisible. What it does render is the one moment there is a decision to make:
 * the bytes are downloaded and verified, and applying them costs a restart.
 *
 * A thin component on purpose. Every phase transition, timer and call lives in
 * `useAppUpdate`; the only state here is whether the operator said "Later".
 */
export function AppUpdatePrompt(options: UseAppUpdateOptions = {}) {
  const { phase, info, error, install, download, reset } = useAppUpdate(options);

  const [dismissed, setDismissed] = useState(false);
  const [seenPhase, setSeenPhase] = useState(phase);
  const [dismissedError, setDismissedError] = useState<string | null>(null);

  // "Later" means later. A dismissed banner comes back when the flow re-enters
  // an actionable state — the next release, or an install the operator started
  // — but a *repeating* background failure they already waved away does not.
  if (phase !== seenPhase) {
    setSeenPhase(phase);
    if (shouldResurface(seenPhase, phase, error, dismissedError)) setDismissed(false);
  }

  if (!isActionable(phase) || dismissed) return null;

  const dismiss = () => {
    if (phase === "error") {
      setDismissedError(error);
      reset();
    }
    setDismissed(true);
  };

  const retry = () => {
    setDismissedError(null);
    reset();
    void download();
  };

  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "fixed bottom-4 right-4 z-50 w-[min(22rem,calc(100vw-2rem))]",
        "rounded-2xl border border-border bg-popover p-4 text-popover-foreground shadow-lg",
      )}
      data-testid="app-update-prompt"
      data-phase={phase}
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 inline-flex size-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
          {phase === "installing" ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <ArrowDownToLine className="size-3.5" />
          )}
        </span>

        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium leading-tight">{updateHeadline(phase)}</p>

          {phase === "ready" && (
            <>
              <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                {updateSummary(info?.availableVersion ?? null)} Restarting takes a few
                seconds, and every company running on this computer comes back up with it.
              </p>
              <div className="mt-3 flex items-center gap-2">
                <Button size="sm" onClick={() => void install()} data-testid="app-update-restart">
                  Restart now
                </Button>
                <Button size="sm" variant="ghost" onClick={dismiss} data-testid="app-update-later">
                  Later
                </Button>
              </div>
            </>
          )}

          {phase === "installing" && (
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
              Applying the update. The window will close and reopen on its own.
            </p>
          )}

          {phase === "error" && (
            <>
              <p className="mt-1 text-xs leading-relaxed text-destructive">
                {error ?? "The update could not be applied."}
              </p>
              <div className="mt-3 flex items-center gap-2">
                <Button size="sm" onClick={retry} data-testid="app-update-retry">
                  Try again
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={dismiss}
                  data-testid="app-update-dismiss"
                >
                  Dismiss
                </Button>
              </div>
            </>
          )}
        </div>

        {phase !== "installing" && (
          <Button
            size="icon-xs"
            variant="ghost"
            onClick={dismiss}
            aria-label="Dismiss the update notice"
            className="text-muted-foreground"
          >
            <X className="size-3" />
          </Button>
        )}
      </div>
    </div>
  );
}
