import { useCallback } from "react";
import {
  BookText,
  Brain,
  FolderClosed,
  type LucideIcon,
  MessagesSquare,
  Network,
  Plug,
  Wallet,
  Workflow,
} from "lucide-react";

import {
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { RESTING_ROW } from "@/components/sidebar-controls";
import { useRoomRailSlot } from "@/components/room-rail";
import { isNavigationActive, type View } from "@/lib/console-routes";
import { CONNECTION_PAGES } from "@/views/connection-pages";
import { cn } from "@/lib/utils";

/**
 * One destination inside a section: a row rendered under its parent while that
 * section is the one you are in.
 *
 * `sub` is the hash's second segment, for a child that is a sub-page of its
 * parent's view (`#/connections/apps`) rather than a view of its own.
 */
export interface NavChild {
  view: View;
  sub?: string;
  label: string;
  icon: LucideIcon;
}

/**
 * A top-level sidebar row, and everything filed under it.
 *
 * A view with no row of its own but an obvious owner — `#/tasks/<id>` under
 * Work, `#/team/<id>` under Agents, `#/conversation` under Room — is claimed by
 * `isNavigationActive` in `console-routes.ts` rather than by a list here, so
 * there is one place that decides it and the routing module owns it.
 */
export interface NavSection {
  /** Where the section's own row goes. */
  view: View;
  sub?: string;
  label: string;
  icon: LucideIcon;
  children?: NavChild[];
  /**
   * Renders in place of `children`, for a section whose contents are live data
   * rather than a fixed list. Room is the only one: its contents are the
   * channel list, which `ChatView` portals in (`room-rail.tsx`).
   */
  slot?: "room";
}

/**
 * The console's four sections.
 *
 * ## Why four rows and not ten
 *
 * Ten flat rows is not a list an operator scans, it is a wall — the same
 * judgement `docs/spec/runtime/ledgers-console-ia.md` made when it rejected a
 * row per declared list. What replaces it is four things you can name without
 * reading: the room you talk in, the company you are running, what it is
 * connected to, and the work it repeats. Everything else is filed under one of
 * them, in the sidebar, visible while you are in that section.
 *
 * ## Labels and view ids are allowed to differ
 *
 * "Room" is the `chat` view and "Flows" is `workflows`, exactly as "Work" has
 * been the `ledgers` view since #1284. A view id is an **address** — every
 * `#/chat/<channelId>` link ever minted, every `#/workflows/<id>` a run row
 * points at — and renaming a row is not a reason to break them. The `data-tour`
 * anchors follow the view id for the same reason: they are how the guided tour
 * and the e2e specs find a row, and they should not move when a word does.
 *
 * ## Sub-navigation lives HERE, not in a rail inside the content area
 *
 * Finance and Settings each draw their own `w-60` rail inside the page. That is
 * the wrong place for it once more than one section has sub-pages: it puts the
 * same kind of list in two different places depending on which section you are
 * in, and it costs the content pane 240px on every one of them. A section's
 * contents belong under its row, where the sidebar already is.
 */
export const NAV_SECTIONS: NavSection[] = [
  // The chat column, whole, and the console's default landing view
  // (`app-shell.tsx`'s `useHashView` fallback). The room is where an operator
  // says what they want and where their company answers — the thing they came
  // to do — so it is what opens, and it is first.
  //
  // Its contents are not a table here: they are the channel list `ChatView`
  // already renders, portalled into the slot below. See `room-rail.tsx`.
  { view: "chat", label: "Room", icon: MessagesSquare, slot: "room" },
  // The company itself: who is in it, what they are working on, what it keeps,
  // what it remembers, and what it spends. Five surfaces that were five
  // top-level rows and are one subject.
  {
    view: "company",
    label: "Company",
    icon: Network,
    children: [
      // Today's Company page, renamed. "Company > Company" said the word twice
      // and told you nothing; what the page actually is, is the roster and the
      // org chart — the agents.
      { view: "company", label: "Agents", icon: Network },
      // Tasks by default; every other list the company declared is one click
      // away through the switcher on `LedgersView`'s own title. See
      // `docs/spec/runtime/ledgers-console-ia.md` Rule 2 for why this is one
      // row and not one per list.
      { view: "ledgers", label: "Work", icon: BookText },
      { view: "workspace", label: "Workspace", icon: FolderClosed },
      { view: "brain", label: "Brain", icon: Brain },
      { view: "finances", label: "Finance", icon: Wallet },
    ],
  },
  // What the company can act through: the apps its teammates sign in to, and
  // the MCP tool servers they can call. Its children come straight off
  // `CONNECTION_PAGES` rather than being restated here — that table is already
  // what the route resolver, the rewrites and `CONNECTIONS_NAMED_BY` read, and
  // a fourth copy of two labels is a fourth thing to forget. This section grew
  // its own content rail when it shipped (PR #1977); the rail is gone and these
  // rows are what replaced it.
  {
    view: "connections",
    label: "Connections",
    icon: Plug,
    children: CONNECTION_PAGES.map((page) => ({
      view: "connections" as const,
      sub: page.id,
      label: page.label,
      icon: page.icon,
    })),
  },
  // Was "Workflows". One word, and the word an operator uses out loud.
  { view: "workflows", label: "Flows", icon: Workflow },
  // Overview and Approvals are NOT here, and neither is Observatory. All three
  // are Rule-6 calls, made explicitly in
  // `docs/spec/runtime/ledgers-console-ia.md`:
  //
  //   - Overview and Approvals moved UP, into the window's title row, where
  //     they are chrome rather than destinations — a place you jump to from
  //     anywhere, and a count that has to be visible from every page including
  //     the collapsed rail (issue #1018). Discoverable elsewhere, in Rule 6's
  //     first sense.
  //   - Observatory moved DOWN, into Settings (`settings-pages.ts`). Bare
  //     `#/observatory` is rewritten onto it; `#/observatory/<runId>` stays a
  //     top-level route because a run is deep-linked from workflow rows,
  //     approval cards and chat.
  //
  // Agent-authored internal dashboard pages, rendered in a sandboxed iframe
  // (docs/spec/runtime/pages.md), are deliberately NOT offered here (issues
  // #1171, #1172). Do not "fix" the omission by adding a row. What keeps
  // `#/pages` answering is its entry in `@/lib/console-routes`, never a row in
  // this table — a commented row routes nothing, which is exactly how the
  // address died for four months (issue #1311).
  //
  // Settings is not here either, and its absence is deliberate in the same
  // way: it is a utility, not a place an operator works, so it sits on the
  // sidebar's footer with Feedback and Discord (`SidebarUtilityBar`), which
  // still carries the `data-tour="nav-settings"` anchor the guided tour
  // spotlights.
];

/**
 * The section an address belongs to, or `undefined` for a view that is filed
 * under none (Settings and Feedback are in the footer; Overview and Approvals
 * are in the window title row; `not-found` is nowhere by design).
 */
export function sectionOwning(view: View): NavSection | undefined {
  return NAV_SECTIONS.find(
    (section) =>
      isNavigationActive(section.view, view) ||
      section.children?.some((child) => isNavigationActive(child.view, view)),
  );
}

/**
 * Whether a child row is the one currently open.
 *
 * A child with no `sub` of its own owns the bare address AND every second
 * segment its view carries — `#/ledgers/goals` is still Work, `#/workspace/<id>`
 * is still Workspace. A child that names a `sub` owns exactly that segment, and
 * the section's first child additionally owns the bare address, because that is
 * what the parent row lands on (`#/connections` renders Apps).
 */
export function childActive(
  section: NavSection,
  child: NavChild,
  view: View,
  sub: string | null,
): boolean {
  if (!isNavigationActive(child.view, view)) return false;
  if (child.sub === undefined) return true;
  if (sub === null) return section.children?.[0] === child;
  return child.sub === sub;
}

/**
 * The sidebar: four fixed rows, then whatever is filed under the one you are in.
 *
 * ## Not an accordion
 *
 * The four rows are always visible, always contiguous, and always in the same
 * place. Selecting a section does not displace its siblings and does not expand
 * a row in place — it swaps the block BELOW the four, under a divider. Exactly
 * one section's contents are on screen at a time.
 *
 * The two blocks are separated by space rather than by a rule: the column is
 * already quiet, and the console draws no rule above its footer either, so one
 * here would have been the only seam in it.
 *
 * The alternative — each row expanding under itself, pushing the rows after it
 * down — was the first thing this looked like and is worse in two specific
 * ways. The rows move, so the muscle memory of "Flows is the fourth thing" only
 * holds while nothing above it is open. And the one section whose contents are
 * unbounded, Room, pushes every row after it off the bottom at an ordinary
 * twenty channels — which recreates, inside one row, exactly the wall this
 * restructure exists to remove (`ledgers-console-ia.md` Rule 2, Draft 1).
 *
 * With a fixed block the four rows never scroll away and the contents block is
 * the only thing that scrolls. There is also no per-row open/closed state to
 * keep: which section is showing is which section you are in, and the route
 * already carries that.
 *
 * ## On the collapsed rail
 *
 * The four icons stay; the contents block is hidden for a section with a fixed
 * list of children — those rows are 3rem of nothing without their labels, and
 * their parent icon still leads to them. Room is the exception, and
 * deliberately so: `ChannelRail` has a compact variant built for exactly this
 * width (avatars and `#` glyphs with unread dots), and dropping it would make
 * collapsing the sidebar silently lose the channel list — the same regression
 * issue #1018 filed about the approvals badge.
 */
export function SidebarNavigation({
  view,
  sub,
  onNavigate,
}: {
  view: View;
  /** The hash's second segment, so a child row can light for its own sub-page. */
  sub: string | null;
  onNavigate: (view: View, sub?: string) => void;
}) {
  const { isMobile, setOpenMobile } = useSidebar();
  const { setElement } = useRoomRailSlot();

  const navigate = useCallback(
    (next: View, nextSub?: string) => {
      onNavigate(next, nextSub);
      if (isMobile) setOpenMobile(false);
    },
    [isMobile, onNavigate, setOpenMobile],
  );

  const active = sectionOwning(view);

  return (
    <>
      {/* The four. Fixed: this group never grows, never shrinks and never
          scrolls, so the rows stay where an operator left them. */}
      <SidebarGroup className="shrink-0">
        <SidebarMenu>
          {NAV_SECTIONS.map((section) => (
            <SidebarMenuItem key={section.view} data-tour={`nav-${section.view}`}>
              <SidebarMenuButton
                isActive={section === active}
                tooltip={section.label}
                onClick={() => navigate(section.view, section.sub)}
                className={RESTING_ROW}
              >
                <section.icon />
                <span>{section.label}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroup>

      {active && (active.children || active.slot) && (
        <>
          {/* Space, not a rule.
              
              A horizontal line here was the reflex and it is the wrong mark:
              this column is already quiet, and one more seam across 13.5rem
              reads as hardware bolted on. The gap does the same work — above
              it, the four places you can go; below it, what is inside the one
              you are in — and it does it without adding anything to look at.
              The console draws no rule above its footer either, so a rule here
              would also have been the only one in the column.

              `pt-5` rather than the group's own `py-1`: deliberate, and large
              enough that the break is legible at a glance rather than a row gap
              that reads as an accident. Together with the fixed block's `pb-1`
              that is 24px of air against an 8px rhythm.

              `min-h-0 flex-1`, so a long contents block scrolls INSIDE itself
              rather than pushing the four rows or the footer off the column. A
              flex item's default `min-height: auto` floors it at its content,
              which is why the zero has to be said here as well as on the child
              that actually scrolls. */}
          <SidebarGroup
            className={cn(
              "min-h-0 flex-1 pt-5",
              !active.slot && "group-data-[collapsible=icon]:hidden",
            )}
          >
            {active.children && (
              // Named, not headed. `nav-rail-headings.test.ts` (issue #1392)
              // forbids an `h1`–`h6` inside a `<nav>`: the sidebar renders
              // before the page, so a heading here would meet a screen reader's
              // heading navigation ahead of the page's own `h1`. `aria-label`
              // names the list without entering the document outline.
              <SidebarMenu aria-label={`${active.label} pages`}>
                {active.children.map((child) => {
                  const open = childActive(active, child, view, sub);
                  return (
                    <SidebarMenuItem key={`${child.view}/${child.sub ?? ""}`}>
                      <SidebarMenuButton
                        isActive={open}
                        aria-current={open ? "page" : undefined}
                        data-tour={`nav-${child.sub ?? child.view}`}
                        tooltip={child.label}
                        onClick={() => navigate(child.view, child.sub)}
                        className={RESTING_ROW}
                      >
                        <child.icon />
                        <span>{child.label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  );
                })}
              </SidebarMenu>
            )}

            {/* Room's contents. The node is the portal target; what lands in it
                is `ChatView`'s own `ChannelRail`, unchanged — see
                `room-rail.tsx`. It scrolls rather than truncating behind a
                "show all": a channel list is scanned for a name you already
                know, and hiding its tail behind a control makes the one thing
                you came for the one thing you cannot see. */}
            {active.slot === "room" && (
              <div
                ref={setElement}
                data-testid="room-rail-slot"
                className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto"
              />
            )}
          </SidebarGroup>
        </>
      )}
    </>
  );
}
