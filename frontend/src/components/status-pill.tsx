import { cn } from "@/lib/utils";
import { lifecycle } from "@/lib/language";

const TONE_STYLES: Record<string, { dot: string; text: string }> = {
  live: { dot: "bg-status-done", text: "text-status-done-text" },
  idle: { dot: "bg-status-blocked", text: "text-status-blocked-text" },
  stopped: { dot: "bg-status-failed", text: "text-status-failed-text" },
};

/** A small lifecycle indicator: a colored dot + plain-language label. */
export function StatusPill({
  lifecycle: state,
  emergencyPaused,
  className,
}: {
  lifecycle: string;
  /** Issue #86: the kill switch, which outranks the lifecycle value. */
  emergencyPaused?: boolean;
  className?: string;
}) {
  const { label, tone } = lifecycle(state, emergencyPaused);
  const style = TONE_STYLES[tone];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border bg-card px-2.5 py-0.5 text-xs font-medium",
        style.text,
        className,
      )}
    >
      <span className={cn("size-1.5 rounded-full", style.dot, tone === "live" && "animate-pulse")} />
      {label}
    </span>
  );
}
