import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

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
 */
interface RoomRailSlot {
  /** The sidebar's mount point, or `null` while Room is not expanded. */
  element: HTMLElement | null;
  /** Called by the sidebar with its slot node, as a ref callback. */
  setElement: (element: HTMLElement | null) => void;
}

const RoomRailSlotContext = createContext<RoomRailSlot | null>(null);

export function RoomRailSlotProvider({ children }: { children: ReactNode }) {
  const [element, setElement] = useState<HTMLElement | null>(null);
  const value = useMemo(() => ({ element, setElement }), [element]);
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

const NO_SLOT: RoomRailSlot = { element: null, setElement: () => {} };
