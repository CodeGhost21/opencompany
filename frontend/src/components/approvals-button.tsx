// What is waiting on you, in the window's title row.
//
// # Why the count moved out of the sidebar
//
// It was a `SidebarMenuBadge` on the Approvals row, and `SidebarMenuBadge`
// carries `group-data-[collapsible=icon]:hidden` — correct on its own terms, a
// two-digit count does not fit a 32px rail, but it made the console's only
// attention signal disappear exactly when the sidebar was collapsed. Issue
// #1018 papered over that with a second element, `SidebarMenuDot`: the same
// `pending` value rendered as a bare mark that survives 32px, mirrored so that
// precisely one of the two ever showed.
//
// Two mechanisms for one fact, and the second existed only because the first
// hid itself. In the title row neither problem exists: this band is chrome, it
// never collapses, and it is on screen on every page in every sidebar state.
// So the badge and the dot are both deleted rather than maintained — the dot
// was protecting against a disappearance that can no longer happen.
//
// # What survives the move
//
// **The accessible name is the whole of what a screen reader gets** from an
// icon-only control, so it says what is waiting and how many — the exact
// sentence the dot's `aria-label` carried, verbatim, because that wording was
// the deliberate answer to "colour alone is not a signal everyone receives".
//
// **The count is one value, from one place.** `pending` is
// `feed.status.pending_approvals`, threaded through the shell — never counted
// again here. A second source is a second answer, and the contract issue #932
// pins is that there is one.

import { ShieldCheck } from "lucide-react";

import { TITLE_BAR_ICON_BUTTON } from "@/components/window-title-bar";
import { cn } from "@/lib/utils";

/**
 * Above this the chip prints `99+` instead of the number.
 *
 * Three digits do not fit a mark sitting on the corner of a 32px control
 * without either overflowing the button or shrinking the type below the design
 * system's floor. The true number stays in {@link approvalsLabel}, so nothing
 * is lost — the digits an operator cannot read are traded for an exact count a
 * screen reader still gets.
 */
export const APPROVALS_COUNT_CAP = 99;

/**
 * What this control is called, given how much is waiting.
 *
 * The `pending > 0` sentence is the one the collapsed-rail dot carried before
 * this row existed, kept word for word: it names WHAT is waiting and HOW MANY,
 * which is what makes the signal reach someone who never sees the chip.
 *
 * At zero it is just the destination's name. "0 approvals need you" would be a
 * sentence about attention at the moment nothing wants any.
 */
export function approvalsLabel(pending: number): string {
  if (pending <= 0) return "Approvals";
  return `${pending} ${pending === 1 ? "approval needs" : "approvals need"} you`;
}

/** What the chip prints — the count, or `99+` past {@link APPROVALS_COUNT_CAP}. */
export function approvalsCount(pending: number): string {
  return pending > APPROVALS_COUNT_CAP ? `${APPROVALS_COUNT_CAP}+` : String(pending);
}

export function ApprovalsButton({
  pending,
  active = false,
  onNavigate,
  className,
}: {
  /**
   * How many approvals are waiting — `feed.status.pending_approvals`, passed
   * through unchanged. Zero is an ordinary state: the glyph stays and the chip
   * does not appear.
   */
  pending: number;
  /** Whether the approvals queue is the view on screen. */
  active?: boolean;
  onNavigate: () => void;
  className?: string;
}) {
  const label = approvalsLabel(pending);
  return (
    <button
      type="button"
      data-testid="title-bar-approvals"
      // So a test can read the count off the closed control without depending
      // on the chip's text, which is capped and therefore not the number.
      data-pending={pending}
      onClick={onNavigate}
      aria-current={active ? "page" : undefined}
      aria-label={label}
      title={label}
      className={cn(TITLE_BAR_ICON_BUTTON, className)}
    >
      <ShieldCheck aria-hidden="true" className="size-4" />
      {pending > 0 && (
        <span
          data-testid="title-bar-approvals-count"
          // The digits are decoration for anyone reading the label: the button
          // already says "3 approvals need you", and announcing "3" again after
          // it is the same fact twice.
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center",
            "rounded-full px-1 text-3xs leading-none font-medium tabular-nums select-none",
            // `--status-blocked` is the token for "waiting on someone", which is
            // exactly what a pending approval is; the dot this replaces used the
            // same one. Soft fill plus the matching text tone rather than a solid
            // block, because a solid fill has no foreground token that themes
            // with it — and this pair is what `workflow-node` already uses for a
            // blocked state in both light and dark.
            "bg-status-blocked-soft text-status-blocked-text",
            // A cut-out of the ground, so the chip reads as sitting ON the glyph
            // rather than merged into it. The title row is the `--chrome` layer.
            "ring-2 ring-chrome",
          )}
        >
          {approvalsCount(pending)}
        </span>
      )}
    </button>
  );
}
