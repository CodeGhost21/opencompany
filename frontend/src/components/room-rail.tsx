import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

import { useSidebar } from "@/components/ui/sidebar";

/**
 * The seam that lets Room's channel list live in the app sidebar while its
 * data keeps living in `ChatView`.
 *
 * The sidebar and the content pane are siblings under `SidebarProvider`, so the
 * sidebar cannot reach into `ChatView`'s state and `ChatView` cannot render into
 * the sidebar's tree. Two ways out of that: lift the whole rail model — sections,
 * unread, mentions, the roster, the fold set, the two dialogs it opens — up into
 * the shell, or leave it exactly where it is and portal the rendered rail into a
 * slot the sidebar owns.
 *
 * This is the second. `ChatView` stays the one owner of the chat model (it is
 * 2,400 lines of it), the rail keeps rendering from that state on the same
 * render pass, and `NewMessageDialog` and `ChannelCreateDialog` keep opening
 * from inside `ChatView`'s React tree even though their triggers are painted in
 * the sidebar — a portal moves the DOM node, not the component tree, so context,
 * events and focus management all still resolve against Chat.
 *
 * Lifting the state instead would have meant an effect in `ChatView` writing the
 * model up to the shell and a second render of the whole console every time an
 * unread count changed.
 *
 * The slot is `null` whenever the Room section is not expanded — a different
 * section is active, or the mobile sheet is closed and has unmounted its
 * contents. `ChatView` renders no rail at all then, which is the intended
 * behaviour rather than a fallback.
 *
 * The sidebar's own density travels the same way. `ChatView` used to keep a
 * `collapsed` flag of its own in `localStorage` (`lib/chat-rail.ts`), because the
 * rail was its own column and nothing else governed it. It is a section of the
 * app sidebar now, so the sidebar's state IS the rail's state — one control, one
 * persisted preference (the sidebar's cookie), and no way for the two to
 * disagree.
 */
interface RoomRailSlot {
  /** The sidebar's mount point, or `null` while Room is not expanded. */
  element: HTMLElement | null;
  /** Called by the sidebar with its slot node, as a ref callback. */
  setElement: (element: HTMLElement | null) => void;
  /** The sidebar is a 3rem icon rail: the channel list renders compact. */
  collapsed: boolean;
  /** Put the sidebar back to a full column. */
  expand: () => void;
  /**
   * Whether the rail is currently covering the transcript — true only on a
   * phone, where the sidebar is a sheet over the whole screen. `ChatView` gates
   * mention-clearing on it: a mention that lands while the operator is looking
   * at the channel list must not be marked read behind their back.
   */
  covering: boolean;
  /**
   * Opens the rail from the chat header, or `undefined` where the rail is
   * already on screen beside the transcript. Absent rather than disabled, the
   * rule this codebase follows for a control that would do nothing.
   */
  reveal?: () => void;
}

const RoomRailSlotContext = createContext<RoomRailSlot | null>(null);

export function RoomRailSlotProvider({ children }: { children: ReactNode }) {
  const [element, setElement] = useState<HTMLElement | null>(null);
  const { state, isMobile, openMobile, setOpenMobile, toggleSidebar } = useSidebar();
  // `state` tracks the DESKTOP open flag; the sheet has its own. Reading it
  // unguarded would render the compact rail inside an open sheet whenever the
  // desktop sidebar happened to be collapsed — the same trap
  // `SidebarCollapseButton` documents.
  const collapsed = !isMobile && state === "collapsed";
  const covering = isMobile && openMobile;
  const value = useMemo<RoomRailSlot>(
    () => ({
      element,
      setElement,
      collapsed,
      expand: toggleSidebar,
      covering,
      reveal: isMobile ? () => setOpenMobile(true) : undefined,
    }),
    [element, collapsed, covering, isMobile, setOpenMobile, toggleSidebar],
  );
  return <RoomRailSlotContext.Provider value={value}>{children}</RoomRailSlotContext.Provider>;
}

/**
 * The Room slot, for the sidebar (which sets it) and for `ChatView` (which
 * portals into it).
 *
 * Returns a nulled slot outside the provider so a standalone `ChatView` — the
 * unit tests, a future embed — renders no rail rather than throwing.
 */
export function useRoomRailSlot(): RoomRailSlot {
  return useContext(RoomRailSlotContext) ?? NO_SLOT;
}

const NO_SLOT: RoomRailSlot = {
  element: null,
  setElement: () => {},
  collapsed: false,
  expand: () => {},
  covering: false,
};
