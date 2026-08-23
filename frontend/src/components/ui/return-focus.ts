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
    if (typeof document === "undefined") return
    const active = document.activeElement
    if (active instanceof HTMLElement && active !== document.body) {
      target.current = active
    }
  }

  if (open !== undefined) {
    if (open && !wasOpen.current) capture()
    wasOpen.current = open
  }

  const handleOpenChange: OpenChange = (nextOpen, ...rest) => {
    if (nextOpen && !wasOpen.current) capture()
    wasOpen.current = nextOpen
    onOpenChange?.(nextOpen, ...rest)
  }

  return { target, handleOpenChange }
}

export { ReturnFocusContext, useReturnFocus }
