import { useCallback } from "react";
import {
  Activity,
  BookText,
  Brain,
  FolderClosed,
  LayoutDashboard,
  type LucideIcon,
  MessagesSquare,
  Network,
  Plug,
  ShieldCheck,
  Wallet,
  Workflow,
} from "lucide-react";

import {
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
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
  { view: "overview", label: "Overview", icon: LayoutDashboard },
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
  { view: "approvals", label: "Approvals", icon: ShieldCheck },
  { view: "workflows", label: "Workflows", icon: Workflow },
  // What the agents actually did, run by run — the read-only companion to
  // Workflows' authoring canvas. See docs/spec/runtime/deep-trace.md.
  { view: "observatory", label: "Observatory", icon: Activity },
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
 * The sidebar's four rows, each expanding to show what is filed under it.
 *
 * Only the active section expands. A sidebar that showed every section's
 * contents at once would be the twenty-row wall this restructure exists to
 * remove; a sidebar that expanded on hover or on a disclosure chevron would make
 * getting to a page a two-gesture affair on the console's most-used surface.
 * Expanding what you are in costs nothing and answers "where am I" at the same
 * time as "where else can I go".
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
    <SidebarGroup>
      <SidebarMenu>
        {NAV_SECTIONS.map((section) => {
          const expanded = section === active;
          const openChild = expanded
            ? section.children?.find((child) => childActive(section, child, view, sub))
            : undefined;
          return (
            <SidebarMenuItem key={section.view} data-tour={`nav-${section.view}`}>
              <SidebarMenuButton
                // The parent row carries the accent only when nothing beneath
                // it does. With a child lit, a lit parent is two highlights for
                // one location; the section still reads as the one you are in
                // because it is the only one showing its contents, and because
                // `data-section-active` lifts it out of the resting dim.
                isActive={expanded && !openChild}
                data-section-active={expanded ? "" : undefined}
                tooltip={section.label}
                onClick={() => navigate(section.view, section.sub)}
                className={cn(RESTING_ROW, "data-section-active:opacity-100")}
              >
                <section.icon />
                <span>{section.label}</span>
              </SidebarMenuButton>

              {expanded && section.children && (
                // Named, not headed. `nav-rail-headings.test.ts` (issue #1392)
                // forbids an `h1`–`h6` inside a `<nav>`: the sidebar renders
                // before the page, so a heading here would meet a screen
                // reader's heading navigation ahead of the page's own `h1`.
                // `aria-label` gives the list its name without entering the
                // document outline.
                <SidebarMenuSub aria-label={`${section.label} pages`}>
                  {section.children.map((child) => (
                    <SidebarMenuSubItem key={`${child.view}/${child.sub ?? ""}`}>
                      <SidebarMenuSubButton
                        isActive={child === openChild}
                        aria-current={child === openChild ? "page" : undefined}
                        data-tour={`nav-${child.sub ?? child.view}`}
                        onClick={() => navigate(child.view, child.sub)}
                        render={<button type="button" />}
                      >
                        <child.icon />
                        <span>{child.label}</span>
                      </SidebarMenuSubButton>
                    </SidebarMenuSubItem>
                  ))}
                </SidebarMenuSub>
              )}

              {/* Room's contents. The node is the portal target; what lands in
                  it is `ChatView`'s own `ChannelRail`, unchanged. Rendered on
                  the collapsed rail too — the rail has a compact variant, and
                  dropping the channel list at 3rem is exactly the regression
                  issue #1018 filed about the approvals badge. */}
              {expanded && section.slot === "room" && (
                <div ref={setElement} data-testid="room-rail-slot" className="min-w-0" />
              )}
            </SidebarMenuItem>
          );
        })}
      </SidebarMenu>
    </SidebarGroup>
  );
}
