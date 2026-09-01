import type { ReactNode } from "react";


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
 *
 * # Every page is framed, and there is no escape hatch
 *
 * There was one. `unframed` rendered a page edge-to-edge, and two surfaces took
 * it: the Overview knowledge graph and the React Flow workflow canvas. The
 * reference shell this was ported from keeps that prop for a reason that does
 * not exist here — its provider webviews were composited by CEF *above* the
 * HTML layer, so a rounded card underneath showed four square corners punching
 * through, maskable by no CSS. Nothing in this console draws above the HTML
 * layer, so the constraint is absent, and a mechanism with no consumers is a
 * thing that rots. Both canvases sit on this card like every other page.
 *
 * If a surface ever genuinely cannot be framed, this is where the prop goes
 * back — but it should come back with the surface that needs it, not before.
 */

/**
 * `--frame-inset` on all four sides, so the frame is one quantity rather than
 * four numbers that happen to agree — and so it reads as a deliberate frame
 * rather than the hairline sliver a three-sided inset gives.
 *
 * `min-h-0` is what lets a view's own `overflow-y-auto` actually scroll: a flex
 * item's default `min-height: auto` floors it at its content's height, so
 * without this the surface grows to fit the page and the scroll happens on the
 * window instead — the same failure `SidebarInset`'s `min-w-0` fixes on the
 * other axis (issue #334).
 *
 * The edge is a full-perimeter hairline rather than the offset one the
 * reference shell uses. That shell insets the card to reveal an *animated*
 * backdrop, which does the separating; here the chrome is a static tint, and in
 * dark mode the card (#08090B) and the chrome (#121315) are 1.07:1 apart — fill
 * contrast alone at that range is a gradient, not an edge. `shadow-sm` carries
 * the lift, and it already resolves to the theme's own treatment: a tinted drop
 * shadow in light, a 1px inset top highlight in dark, which is what actually
 * reads as "raised" against near-black.
 */
const CARD =
  "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden " +
  // `--frame-inset` on three sides, and a thinner top.
  //
  // The frame used to be one quantity because nothing sat above the card. The
  // title row does now, so a full inset there stacks the row's own bottom
  // padding on top of the card's margin and reads as a gap twice the size of
  // the one on the other three edges — which is what it is.
  "mx-(--frame-inset) mb-(--frame-inset) mt-0.5 rounded-2xl border border-chrome-border bg-background shadow-sm";

export function ContentSurface({ children }: { children: ReactNode }) {
  return (
    <div className={CARD} data-testid="content-surface">
      {/* No drag band here any more.
          
          This card carried one because the window drew no title bar of its own,
          so the top of the content was the only thing left to grab. There is a
          real full-width title row now (`window-title-bar.tsx`), which drags
          where it is not covered by a control — so this band stopped being the
          handle and stayed only as 28px at the top of every page that quietly
          refused a press. It was the spacing under the new row, and the reason
          the two canvases you can drag lost their top edge. */}
      {children}
    </div>
  );
}
