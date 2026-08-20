import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * Flex/overflow behaviour, identical framed or unframed.
 *
 * `min-h-0` is what lets a view's own `overflow-y-auto` actually scroll: a
 * flex item's default `min-height: auto` floors it at its content's height, so
 * without this the surface grows to fit the page and the scroll happens on the
 * window instead — the same failure `SidebarInset`'s `min-w-0` fixes on the
 * other axis (issue #334).
 */
const BASE = "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden";

/**
 * The inset card: an even 12px margin on all four sides, so the chrome reads as
 * a deliberate frame rather than the hairline sliver a three-sided inset gives.
 *
 * The edge is a full-perimeter hairline rather than the offset one the
 * reference shell uses. That shell insets the card to reveal an *animated*
 * backdrop, which does the separating; here the chrome is a static tint, and
 * in dark mode the card (#08090B) and the chrome (#121315) are 1.07:1 apart —
 * fill contrast alone at that range is a gradient, not an edge. `shadow-sm`
 * carries the lift, and it already resolves to the theme's own treatment:
 * a tinted drop shadow in light, a 1px inset top highlight in dark, which is
 * what actually reads as "raised" against near-black.
 */
const FRAMED = "m-3 rounded-2xl border border-chrome-border bg-background shadow-sm";

/** Edge-to-edge: no margin, no radius, no edge — the surface IS the pane. */
const UNFRAMED = "bg-background";

interface ContentSurfaceProps {
  children: ReactNode;
  /**
   * Render the page edge-to-edge instead of as an inset card.
   *
   * The escape hatch for surfaces that are *drawings*, not documents. Two ship
   * today and both are load-bearing:
   *
   *   - **Overview** — a force-directed knowledge graph sized against
   *     `h-svh`. Framed, it would be laid out 24px larger than the box it is
   *     clipped to and lose a band off two edges.
   *   - **Workflows** — the React Flow canvas, whose viewport and minimap are
   *     computed from the container's measured rect. Cropping it is the exact
   *     class of bug #1259 and #1261 were filed for.
   *
   * A page of cards and prose wants the frame. A canvas that fills the window
   * and draws its own chrome does not — for it the frame is a crop.
   */
  unframed?: boolean;
}

/**
 * The console's single content sheet — the "card" half of the two-layer shell
 * (issue #1178).
 *
 * The shell is two layers, not two panes. The window *chrome* (`--chrome`) is
 * painted exactly once, on the shell root, and both the sidebar column and the
 * margin around this card are that one surface showing through: the sidebar
 * paints no fill of its own and the panes carry no divider. Tinting each pane
 * separately would land them on different values and put back the 1px seam the
 * layout exists to remove.
 *
 * This card is the only opaque sheet left. Everything a page draws — its own
 * `bg-card` panels, its dialogs — stacks on top of it, which is why it keeps
 * `--background` rather than taking a colour of its own: page contrast is
 * unchanged from before the shell was rebuilt.
 */
export function ContentSurface({ children, unframed = false }: ContentSurfaceProps) {
  return (
    <div
      className={cn(BASE, unframed ? UNFRAMED : FRAMED)}
      data-testid="content-surface"
      data-unframed={unframed ? "true" : undefined}
    >
      {children}
    </div>
  );
}
