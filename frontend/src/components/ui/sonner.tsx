import { useEffect, useRef } from "react"
import { useTheme } from "next-themes"
import { Toaster as Sonner, toast, useSonner, type ToasterProps } from "sonner"
import { CircleCheckIcon, InfoIcon, TriangleAlertIcon, OctagonXIcon, Loader2Icon } from "lucide-react"

import { reconcileTracked, sweepToasts, type TrackedToast } from "@/lib/toast-lifetime"

/** How often the dismissal guard re-checks the toasts that are up. */
const SWEEP_INTERVAL_MS = 500

/** Is the pointer genuinely over the toaster, whatever sonner's state believes? */
function toasterHovered(): boolean {
  return Array.from(document.querySelectorAll("[data-sonner-toaster]")).some((el) =>
    el.matches(":hover"),
  )
}

/** Can this part of a toast handle its own click rather than passing it through? */
function isToastControl(target: Element): boolean {
  return target.closest(
    'a, button, input, select, textarea, [role="button"], [role="link"], [contenteditable="true"]',
  ) !== null
}

/**
 * Let a click on a toast's read-only surface reach the page behind it (issue #1303).
 *
 * The toast still receives pointer movement, which keeps sonner's useful
 * hover-to-read pause intact. Only a click on its non-interactive content is
 * relayed. Its close button and any action button remain ordinary controls, so
 * a notification can still offer a one-click recovery without eating nearby
 * page controls.
 */
function useToastClickThrough(): void {
  useEffect(() => {
    function relayClick(event: MouseEvent): void {
      if (event.button !== 0 || !(event.target instanceof Element)) return
      if (!event.target.closest("[data-sonner-toast]") || isToastControl(event.target)) return

      const toasters = Array.from(document.querySelectorAll<HTMLElement>("[data-sonner-toaster]"))
      const pointerEvents = toasters.map((toaster) => toaster.style.pointerEvents)
      for (const toaster of toasters) toaster.style.pointerEvents = "none"
      const beneath = document.elementFromPoint(event.clientX, event.clientY)
      for (const [index, toaster] of toasters.entries()) {
        toaster.style.pointerEvents = pointerEvents[index]
      }

      if (!(beneath instanceof HTMLElement) || beneath.closest("[data-sonner-toaster]")) return

      event.preventDefault()
      event.stopPropagation()
      beneath.focus({ preventScroll: true })
      beneath.click()
    }

    document.addEventListener("click", relayClick, true)
    return () => document.removeEventListener("click", relayClick, true)
  }, [])
}

/**
 * The ceiling on a toast's life that sonner does not provide (issue #933).
 *
 * sonner's auto-dismiss is a *pausable* timer, and two of the things that pause
 * it can latch with no way back — see `lib/toast-lifetime.ts` for which, and
 * why. The reported symptom was the "Starting the product tour." toast sitting
 * over every view for nine minutes with a working × and nothing else that
 * cleared it. This sweep is what makes that impossible: a toast nobody is
 * hovering, in a tab the operator is looking at, goes away once its own duration
 * plus a grace period has passed, whatever sonner thinks its timer is doing.
 *
 * Deliberately additive. Every intentional pause still holds: hover to read,
 * hidden tab, and an explicit `duration: Infinity` are all left alone.
 */
function useToastDismissalCeiling(): void {
  const { toasts } = useSonner()
  const tracked = useRef<TrackedToast[]>([])

  useEffect(() => {
    tracked.current = reconcileTracked(
      tracked.current,
      toasts.map((t) => ({ id: t.id, duration: t.duration })),
    )
  }, [toasts])

  // Only while something is on screen: the console is a long-lived dashboard, and
  // a timer that ticks all day to find nothing is a timer that keeps the tab
  // awake for no reason.
  const anyToasts = toasts.length > 0
  useEffect(() => {
    if (!anyToasts) return
    const timer = window.setInterval(() => {
      const { next, overdue } = sweepToasts(tracked.current, SWEEP_INTERVAL_MS, {
        hovered: toasterHovered(),
        documentHidden: document.hidden,
      })
      tracked.current = next
      for (const id of overdue) toast.dismiss(id)
    }, SWEEP_INTERVAL_MS)
    return () => window.clearInterval(timer)
  }, [anyToasts])
}

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme()
  useToastDismissalCeiling()
  useToastClickThrough()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: (
          <CircleCheckIcon className="size-4" />
        ),
        info: (
          <InfoIcon className="size-4" />
        ),
        warning: (
          <TriangleAlertIcon className="size-4" />
        ),
        error: (
          <OctagonXIcon className="size-4" />
        ),
        loading: (
          <Loader2Icon className="size-4 animate-spin" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      toastOptions={{
        classNames: {
          toast: "cn-toast",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
