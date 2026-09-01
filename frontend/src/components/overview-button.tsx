// Overview, as a glyph in the window's title row.
//
// The sidebar still carries this destination as a labelled row, and will until
// the sidebar restructure lands. Here it is chrome, and that is the whole
// argument for dropping the word: a title row is a band of chrome, and a
// *labelled* button in it reads as content — the same trade in reverse to the
// one the sidebar's utility bar makes, where an unlabelled glyph in a list of
// named destinations reads as decoration. So the name goes and `aria-label`
// plus `title` keep it reachable by screen reader and by pointer; only the
// pixels are lost.
//
// It is the FIRST thing the row drops as the window narrows (below `md`, see
// {@link TITLE_BAR_LADDER}), because Overview is a destination you *choose*
// and the pending count beside it is one that chooses you.

import { LayoutDashboard } from "lucide-react";

import { TITLE_BAR_ICON_BUTTON } from "@/components/window-title-bar";
import { cn } from "@/lib/utils";

/**
 * The accessible name. A constant rather than an inline string so the test and
 * the button cannot drift, and so the one word this control has left is stated
 * once.
 */
export const OVERVIEW_LABEL = "Overview";

export function OverviewButton({
  active = false,
  onNavigate,
  className,
}: {
  /** Whether the overview is the view on screen. */
  active?: boolean;
  onNavigate: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      data-testid="title-bar-overview"
      onClick={onNavigate}
      // `aria-current="page"` rather than a colour alone: the resting and
      // active fills differ by a background, and a background is not a channel
      // every operator receives. It is also what the row's own `aria-[current]`
      // styling keys off, so the state and its appearance have one source.
      aria-current={active ? "page" : undefined}
      // Both, and deliberately the same word. `aria-label` is the whole of what
      // a screen reader gets from an icon-only control; `title` is the whole of
      // what a sighted operator gets on hover. A control that carries one and
      // not the other is unreachable by half the people using it.
      aria-label={OVERVIEW_LABEL}
      title={OVERVIEW_LABEL}
      className={cn(TITLE_BAR_ICON_BUTTON, className)}
    >
      {/* The same glyph the sidebar row carried, so an operator who learned it
          there recognises it here. */}
      <LayoutDashboard aria-hidden="true" className="size-4" />
    </button>
  );
}
