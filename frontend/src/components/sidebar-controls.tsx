import { MessageSquareWarning, PanelLeftClose, PanelLeftOpen, Settings } from "lucide-react";

import type { View } from "@/components/app-shell";

import { Button } from "@/components/ui/button";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DiscordIcon } from "@/components/discord-icon";
import { DISCORD_INVITE_URL } from "@/lib/links";
import { cn } from "@/lib/utils";

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

// Discord's brand blurple, lifted a step in dark mode so it clears the
// sidebar's surface instead of sinking into it. Named tokens rather than raw
// hex — the colour is deliberately not ours, and saying so in the token name
// is what stops it being "fixed" into the palette later. See `--brand-discord`
// in index.css.
const DISCORD_BLURPLE =
  "text-(--brand-discord-on-light) dark:text-(--brand-discord-on-dark)";

/**
 * The utility bar: Settings, Feedback and Discord, at the foot of the column.
 *
 * **Rows, not an icon strip.** These were three icon-only buttons — the shape
 * OpenHuman's shell uses in its own sidebar header — chosen when they sat
 * *above* the destinations, where three labelled rows would have pushed the
 * company's own state further down the column every time the nav list grew.
 * That argument died with the move to the footer: below the destinations there
 * is nothing left for them to push, and the cost of the strip was that three
 * unlabelled glyphs floated at the bottom of a column whose every other entry
 * says what it is. Naming them costs a column that is already scrolled to its
 * end nothing, and it puts them on the same rhythm as the nav rows they sit
 * under rather than reading as a separate object bolted on.
 *
 * They use the nav's own row primitive for that reason: one shape, one hover,
 * one active treatment, and the tooltip on the collapsed rail comes free. What
 * keeps them from reading as destinations is position — after the list, in the
 * footer — rather than a different shape. Settings keeps its `data-tour`
 * anchor, so the guided tour's "Connect your tools" stop still has something to
 * spotlight.
 */
export function SidebarUtilityBar({
  view,
  onNavigate,
}: {
  /** The active view, so Settings and Feedback can show as current. */
  view: View;
  onNavigate: (view: View) => void;
}) {
  const { isMobile, setOpenMobile } = useSidebar();

  const navigate = (next: View) => {
    onNavigate(next);
    // The sheet is the whole screen on a phone; leaving it open would hide the
    // page just navigated to. Same rule the nav rows follow.
    if (isMobile) setOpenMobile(false);
  };

  return (
    // `role="group"` so the bar has a name of its own. It sits in the sidebar's
    // footer, under the `Main navigation` landmark's destinations on purpose —
    // the landmark is the places an operator works out of, and these are the
    // utilities that act on the console itself.
    <SidebarMenu role="group" aria-label="Console utilities" data-testid="sidebar-utilities">
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={view === "settings"}
          // `aria-current`, not just `isActive`: the row primitive renders
          // `data-active` for its styling and announces nothing. These are
          // destinations, so a reader is told which one is open — absent, not
          // `false`, because `aria-current="false"` is announced by some.
          aria-current={view === "settings" ? "page" : undefined}
          data-tour="nav-settings"
          tooltip="Settings"
          onClick={() => navigate("settings")}
          className={RESTING_ROW}
        >
          <Settings />
          <span>Settings</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton
          isActive={view === "feedback"}
          // `aria-current`, not just `isActive`: the row primitive renders
          // `data-active` for its styling and announces nothing. These are
          // destinations, so a reader is told which one is open — absent, not
          // `false`, because `aria-current="false"` is announced by some.
          aria-current={view === "feedback" ? "page" : undefined}
          tooltip="Feedback"
          onClick={() => navigate("feedback")}
          className={RESTING_ROW}
        >
          <MessageSquareWarning />
          <span>Feedback</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        {/* Deliberately NOT dimmed with the others.

            `RESTING_ROW` dims by opacity, which is safe for near-white text and
            destroys a mid-tone hue: the blurple measures 6.36:1 at full strength
            and 3.04:1 dimmed. Recovering that inside the dim would mean
            lightening the blurple until it is a pale lavender that no longer
            reads as Discord's colour. The hue already sets this row apart
            without help from the property doing the damage. */}
        <SidebarMenuButton
          tooltip="Join our Discord"
          className={cn(
            DISCORD_BLURPLE,
            "hover:text-(--brand-discord-on-light) dark:hover:text-(--brand-discord-on-dark)",
          )}
          render={<a href={DISCORD_INVITE_URL} target="_blank" rel="noreferrer" />}
        >
          <DiscordIcon className="size-4" />
          <span>Join our Discord</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

/**
 * Show or hide the sidebar. A button on the content card's leading seam.
 *
 * ## Why it is not a row (issue #1177)
 *
 * It used to be a `SidebarMenuButton` — full width, icon then label, `h-8`,
 * `bg-sidebar-accent` on hover — sitting directly under the host switcher and
 * directly above Overview. That is the nav row shape exactly, so the eye filed
 * it as the first destination in the list. It is not a destination: everything
 * else in that column takes you somewhere, and this one changes the chrome and
 * leaves you where you are.
 *
 * Colouring it differently would not have fixed that; the shape is what says
 * "row". So it stops using the row primitive altogether and becomes the
 * console's ordinary icon button.
 *
 * ## Why it is not in the sidebar at all
 *
 * Its next home was the sidebar's own header, beside the host switcher. That
 * put the control that *hides* a panel inside the panel it hides: collapsing
 * the column took the button with it, and the rail had to keep a version of it
 * standing in 32px of content box.
 *
 * Both of those are gone now. The switcher moved to the window's title row
 * (`window-title-bar.tsx`) and the header went with it, so this button is
 * rendered from `app-shell.tsx` inside `SidebarInset`, absolutely positioned on
 * the leading border of the content card — `left-(--frame-inset)` puts it at
 * the edge and `-translate-x-1/2` straddles it. It is one control in both
 * states, it points at the edge that moves, and it costs the page no layout.
 * `sidebar-toggle-reachable.spec.ts` pins that placement.
 *
 * It carries its own fill at rest for the same reason: alone on a border, with
 * no neighbours to belong to and no surface behind it, a ghost glyph read as
 * something drawn on the seam rather than as something pressable. See the
 * class list below.
 */
export function SidebarCollapseButton() {
  const { toggleSidebar, state, isMobile } = useSidebar();
  // `state` tracks the DESKTOP open flag; the sheet has its own (`openMobile`).
  // Reading it unguarded labels an open sheet "Expand sidebar" whenever the
  // desktop state happens to be collapsed — which, since issue #1176 stopped
  // the sidebar auto-collapsing, is now a state an operator can leave behind
  // and come back to on a phone.
  const collapsed = !isMobile && state === "collapsed";
  const label = collapsed ? "Expand sidebar" : "Collapse sidebar";
  const Icon = collapsed ? PanelLeftOpen : PanelLeftClose;

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            // The accessible name, and the only name this control has — an
            // icon-only button with no label is otherwise announced as
            // "button". The tooltip says the same words, but a tooltip is a
            // visual affordance and cannot be relied on for the name.
            aria-label={label}
            // Deliberately NO `aria-expanded`, and not an oversight.
            //
            // The name already carries the state: it says "Collapse sidebar"
            // while the column is showing and "Expand sidebar" once it is a
            // rail, so a reader is told what pressing does and, by the change,
            // what happened. `aria-expanded` on top of that announces the
            // state twice ("Expand sidebar, collapsed") — and `ghost` styles
            // the attribute as "the popup under me is open", which is what it
            // means on the dropdown triggers that variant was written for. On
            // this button it painted a pressed chip for as long as the sidebar
            // was open, and Tailwind sorts `aria-expanded:` after `hover:`, so
            // overriding the chip also swallowed the hover feedback. A second
            // channel saying the same thing is not worth either.
            data-testid="sidebar-collapse"
            onClick={toggleSidebar}
            className={cn(
              // The same resting dim as the rows below, reached through the
              // ink's alpha rather than `RESTING_ROW`'s `opacity-60`: opacity
              // dims the whole box, focus ring included, and the ring on an
              // unlabelled button is the only thing saying where the keyboard
              // is. (`RESTING_ROW` also carries `data-active:opacity-100`,
              // which is a nav row's business and never this one's.)
                // Filled at REST, not only on hover.
                //
                // In the sidebar's header this was one icon among four, and the
                // resting dim kept it from shouting over its neighbours. It now
                // sits alone, centred on the seam between the rail and the
                // content card — no neighbours to belong to and no surface
                // behind it — and at ghost weight it read there as a stray
                // glyph drawn on the border rather than as something pressable.
                // Carrying its own fill is what makes it legible as a control
                // where it now lives; hover then deepens the fill instead of
                // being the only thing that draws it — the rest state is the token
                // at 70%, hover the full strength, so the press feedback still
                // moves in the direction it always did.
                "shrink-0 bg-sidebar-accent/70 text-sidebar-accent-foreground",
              // Three classes replacing exactly one of `ghost`'s each, so
              // tailwind-merge drops the original rather than leaving the two
              // to race: `hover:bg-muted`, `hover:text-foreground` and
              // `dark:hover:bg-muted/50`. The muted tint is tuned against the
              // canvas; this button is on the sidebar's surface, which is a
              // different rung and moving again in issue #1178. The accent is
              // also what every row in this column already hovers to.
                "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground dark:hover:bg-sidebar-accent",
              "focus-visible:ring-sidebar-ring/50",
              // No `group-data-[collapsible=icon]:size-8` any more, and its
              // absence is the point. `group` is on `[data-slot=sidebar]`
              // (`ui/sidebar.tsx`) and this button is no longer inside it, so
              // that variant could never match again — it would have been a
              // class that reads as a collapsed-state size and silently is not.
              // One size in both states, which is what a control on the seam
              // wants: it does not live in the 3rem rail and has no rhythm of
              // nav icons to land on.
            )}
          />
        }
      >
        <Icon />
      </TooltipTrigger>
      {/*
        The raw tooltip primitive rather than `SidebarMenuButton`'s `tooltip`
        prop, which renders its content with `hidden={state !== "collapsed"}`.
        That is right for a nav row — expanded, the row already carries its
        label — and wrong here: this button is icon-only in BOTH states, and
        expanded is the state in which a reader has never seen the word.

        `side="right"` in both states, matching every other tooltip in this
        column, and the one side that is clear of the sidebar either way.
      */}
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
