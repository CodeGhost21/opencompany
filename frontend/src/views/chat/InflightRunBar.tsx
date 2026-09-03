import { useCallback, useState } from "react";
import { Loader2, X } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { steerTask, supportsSteerAction, type InflightRun } from "@/api/tasks";
import { ApiError } from "@/api/types";
import { Button } from "@/components/ui/button";

/**
 * The company's in-flight runs, with the controls each one supports, above the
 * composer in a Room.
 *
 * ## Why the runs arrive as a prop
 *
 * The shell already reads `GET …/tasks/inflight` on the company poll and on
 * every task-lifecycle tick. A second reader here would double that traffic and
 * give two surfaces two different answers to "what is running", so the shell
 * owns the read and this renders it.
 *
 * ## Why it is company-scoped inside a room
 *
 * A run carries no conversation id, so there is no honest way to say which room
 * a delegation belongs to. The choice is between showing the company's runs in
 * every room and showing them nowhere; a runaway sub-agent the operator cannot
 * stop is the worse of the two. The heading says "company" so the list does not
 * read as "this channel's work".
 *
 * ## Why a row is keyed on `run.key`
 *
 * Every run in the in-flight read is steerable by its `key`, and a delegation's
 * `taskId` is null. Keying a row on the card would drop exactly the runs that
 * have no other control.
 */
export function InflightRunBar({
  client,
  company,
  runs,
  onSteered,
}: {
  client: OpenCompanyClient;
  company: string | null;
  runs: readonly InflightRun[];
  /** Re-read the inflight list, so a settled or cancelled run leaves the bar. */
  onSteered: () => void;
}) {
  if (runs.length === 0) return null;

  return (
    <div className="border-t bg-muted/30" data-testid="inflight-run-bar">
      <div className="mx-auto w-full max-w-3xl px-4 py-2">
        <p className="mb-1.5 px-1 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          In flight across this company · {runs.length}
        </p>
        <div className="flex flex-col gap-1.5">
          {runs.map((run) => (
            <InflightRunRow
              key={run.key}
              run={run}
              client={client}
              company={company}
              onSteered={onSteered}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

/** Past-tense badge copy while a steer of the given verb is already in flight. */
const PENDING_LABEL: Record<string, string> = {
  pause: "pausing…",
  cancel: "cancelling…",
  redirect: "redirecting…",
};

export function InflightRunRow({
  run,
  client,
  company,
  onSteered,
}: {
  run: InflightRun;
  client: OpenCompanyClient;
  company: string | null;
  onSteered: () => void;
}) {
  const [busy, setBusy] = useState(false);

  const pending = run.pendingAction ?? null;
  const disabled = busy || pending !== null;

  const cancel = useCallback(async () => {
    if (!window.confirm(`Cancel “${run.title}”? This stops the run.`)) return;
    setBusy(true);
    try {
      await steerTask(client, company, run.key, { action: "cancel", confirm: true });
      toast.success(`Cancelling “${run.title}”…`);
    } catch (e) {
      // A run that settled while the click was in flight is no longer in the
      // registry, and the host says so with 404/409. That is the run ending on
      // its own, not a failure to act on it — reporting it as an error would
      // tell the operator their cancel broke when the work is already over.
      if (isAlreadyGone(e)) {
        toast.info(`“${run.title}” had already finished.`);
      } else {
        toast.error(e instanceof Error ? e.message : "could not cancel the run");
      }
    } finally {
      setBusy(false);
      // Always re-read, on both paths: the cancel landed, or the row is stale
      // and the refetch is what removes it.
      onSteered();
    }
  }, [client, company, run.key, run.title, onSteered]);

  return (
    <div className="rounded-lg border bg-card px-2.5 py-1.5" data-testid="inflight-run-row">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">{run.title}</p>
          <p className="truncate text-2xs text-muted-foreground">
            {run.kind === "delegation" ? "Delegation" : "Task"} · {run.agentId}
          </p>
        </div>

        {pending !== null ? (
          <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-3xs font-medium text-muted-foreground">
            {PENDING_LABEL[pending] ?? "steering…"}
          </span>
        ) : (
          <div className="flex shrink-0 items-center gap-1">
            {busy && <Loader2 className="size-3.5 animate-spin text-muted-foreground" />}
            {supportsSteerAction(run, "cancel") && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-xs text-destructive hover:text-destructive"
                disabled={disabled}
                aria-label={`Cancel ${run.title}`}
                onClick={() => void cancel()}
              >
                <X className="mr-1 size-3.5" />
                Cancel
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Whether a failed steer means the run is no longer in flight.
 *
 * The host answers `404` for a key it does not know and `409` for a card that
 * exists but is not running; both mean the same thing to an operator who just
 * asked to stop something. `fromHost` is required, so a proxy's own 404 — which
 * says nothing about the run — still surfaces as the error it is.
 */
function isAlreadyGone(e: unknown): boolean {
  return (
    e instanceof ApiError && e.fromHost && (e.status === 404 || e.status === 409)
  );
}
