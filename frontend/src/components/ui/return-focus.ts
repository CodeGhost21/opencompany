"use client"

import * as React from "react"
import type { Dialog as DialogPrimitive } from "@base-ui/react/dialog"

type OpenChange = NonNullable<DialogPrimitive.Root.Props["onOpenChange"]>

/**
 * The element a popup should hand focus back to when it closes.
 *
 * `null` outside a `Dialog`/`Sheet` root, in which case the popup falls back to
 * Base UI's own return-focus handling.
 */
const ReturnFocusContext =
  React.createContext<React.RefObject<HTMLElement | null> | null>(null)

/**
 * Remembers what was focused when a popup opened, so it can be refocused on
 * close.
 *
 * Most console dialogs and sheets are controlled and have no trigger, so Base
 * UI has no element to restore focus to and leaves it on `<body>` — keyboard
 * users lose their place (issue #1387). Capturing `document.activeElement` is
 * the fix, but it has to happen on the **closed → open transition**: these
 * popups stay mounted while closed, so capturing once on first render would
 * pin whatever happened to be focused during an unrelated ancestor rerender
 * (typing in the ledger's search box, say) and return focus there instead of
 * to the control that opened the popup.
 *
 * A controlled root flips `open` without ever calling `onOpenChange`, so that
 * transition is only visible during render; an uncontrolled root only reports
 * it through `onOpenChange`. Both are watched.
 */
function useReturnFocus(
  open: boolean | undefined,
  onOpenChange: OpenChange | undefined
) {
  const target = React.useRef<HTMLElement | null>(null)
  // Seeded closed even when the first render is already open: a popup whose
  // owner returns `null` while closed (`CreateTaskDialog`) mounts open, and
  // that mount *is* its opening transition.
  const wasOpen = React.useRef(false)

  const capture = () => {
    // Cleared first, every time. The target belongs to *this* opening: if
    // nothing is focused now — a dialog opened from code after its opener was
    // removed, say — the previous opening's target is not a sensible answer,
    // and it may not even be in the document any more. A `null` ref makes Base
    // UI fall back to its own return-focus handling, which is what should
    // happen when the console has nothing better to offer.
    target.current = null
    if (typeof document === "undefined") return
    const active = document.activeElement
    if (active instanceof HTMLElement && active !== document.body) {
      target.current = active
    }
  }

  // Controlled: the `open` prop is the only authority. A consumer is free to
  // refuse a requested dismissal — `SetupDialog` hands Base UI a no-op handler,
  // `DeskCreateDialog` swallows it while submitting — so believing the callback
  // here would record a close that never happened and make the next render look
  // like a fresh opening, capturing an element *inside* the still-open popup.
  if (open !== undefined) {
    if (open && !wasOpen.current) capture()
    wasOpen.current = open
  }

  const handleOpenChange: OpenChange = (nextOpen, ...rest) => {
    // Uncontrolled: Base UI owns the state and the callback is the only report
    // of it, so it is authoritative here for exactly the same reason.
    if (open === undefined) {
      if (nextOpen && !wasOpen.current) capture()
      wasOpen.current = nextOpen
    }
    onOpenChange?.(nextOpen, ...rest)
  }

  return { target, handleOpenChange }
}

export { ReturnFocusContext, useReturnFocus }
