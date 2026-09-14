import { PanelLeftClose, PanelLeftOpen } from "lucide-react";

import { useSidebar } from "@/components/ui/sidebar";
import { TITLE_BAR_ICON_BUTTON } from "@/components/window-title-bar";

/**
 * A sidebar row at rest: dimmed until you reach for it.
 *
 * The sidebar is standing furniture, on screen behind every view — holding the
 * whole list at full strength makes ten equal-weight rows compete with the
 * content beside them. Hover, keyboard focus, and the active row all come back
 * to full, so nothing is ever dimmed at the moment you are using it.
 */
// `data-active` is a bare boolean attribute on these buttons, not
// `data-active="true"` — match it the same way the sidebar's own styles do.
export const RESTING_ROW =
  "opacity-60 transition-opacity hover:opacity-100 focus-visible:opacity-100 data-active:opacity-100";

/*
 * `SidebarUtilityBar` used to live here — Settings, Feedback and Discord as
 * three labelled rows in the sidebar's footer, plus an Overview row drawn
 * `md:hidden` to cover the width at which the title row dropped its glyph.
 *
 * All four are in the window's title row now, as glyphs
 * (`components/title-bar-utilities.tsx`, and `OverviewButton` beside them).
 * None of the three is a place inside the company, which is what this column
 * enumerates; a footer under the destinations was saying "not one of these" by
 * position, inside the one region whose whole job is to list destinations.
 *
 * Deleted rather than left exported and unrendered: an unused export is a third
 * state — not drawn, not gone, and free to be re-added by someone who does not
 * know why it left. `DISCORD_BLURPLE` and `DISCORD_INVITE_URL` went with it and
 * now live beside the control that draws them.
 */

/**
 * Show or hide the sidebar. A glyph in the window's title row.
 *
 * ## Where it has been, and why it is here
 *
 * It began as a `SidebarMenuButton` directly under the host switcher — full
 * width, icon then label — which is the nav row shape exactly, so the eye filed
 * it as the first destination in a list of destinations. It is not one:
 * everything else in that column takes you somewhere, and this changes the
 * chrome and leaves you where you are.
 *
 * Its next home was the sidebar's own header, which put the control that
 * *hides* a panel inside the panel it hides: collapsing the column took the
 * button with it, so the rail had to keep a version of it standing in 32px.
 *
 * Then it was a floating button straddling the content card's leading edge,
 * absolutely positioned out of `SidebarInset`. That fixed both problems and
 * bought a third: a control with no surface behind it, bleeding over the seam
 * between two panes, which needed its own fill and its own shadow to read as
 * pressable at all — and which sat on top of whatever the page drew underneath
 * it.
 *
 * It is a title-row glyph now, beside the company switcher whose column it
 * acts on. That is where the console's other chrome controls already are, it
 * costs the page no layout and overlaps nothing, and it needs no fill of its
 * own because it has four neighbours to belong to. `TITLE_BAR_ICON_BUTTON` is
 * the shared shape, so it moves with them.
 *
 * ## The label carries the state
 *
 * "Collapse sidebar" while the column is showing, "Expand sidebar" once it is a
 * rail — so a reader is told what pressing does and, by the change, what
 * happened. Deliberately NO `aria-expanded` on top of that: it announces the
 * state twice ("Expand sidebar, collapsed"), and as a styling hook it means
 * "the popup under me is open", which is what it does on the dropdown triggers
 * beside this one.
 */
export function SidebarCollapseButton() {
  const { toggleSidebar, state, isMobile } = useSidebar();
  // `state` tracks the DESKTOP open flag; the sheet has its own (`openMobile`).
  // Reading it unguarded labels an open sheet "Expand sidebar" whenever the
  // desktop state happens to be collapsed — which, since issue #1176 stopped
  // the sidebar auto-collapsing, is a state an operator can leave behind and
  // come back to on a phone.
  //
  // Defence in depth rather than the live path: `app-shell.tsx` gates this
  // control at `md`, which is exactly where `useIsMobile` flips, because
  // treating mobile as not-collapsed also meant a CLOSED sheet got "Collapse
  // sidebar" and the close icon while pressing it opened the sheet. Below `md`
  // the way back is the shell's own `md:hidden` "Toggle sidebar" bar, which
  // reserves its own row instead of floating over the content (issue #1265).
  const collapsed = !isMobile && state === "collapsed";
  const label = collapsed ? "Expand sidebar" : "Collapse sidebar";
  const Icon = collapsed ? PanelLeftOpen : PanelLeftClose;

  return (
    <button
      type="button"
      // The accessible name, and the only name this control has — an icon-only
      // button with no label is otherwise announced as "button". `title` is the
      // whole of what a sighted operator gets on hover, and the two say the
      // same word for the same reason every other glyph in this row does.
      aria-label={label}
      title={label}
      data-testid="sidebar-collapse"
      onClick={toggleSidebar}
      className={TITLE_BAR_ICON_BUTTON}
    >
      <Icon aria-hidden="true" className="size-4" />
    </button>
  );
}
