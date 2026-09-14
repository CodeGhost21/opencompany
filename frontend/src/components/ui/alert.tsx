import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const alertVariants = cva(
  "group/alert relative grid w-full gap-0.5 rounded-lg border px-2.5 py-2 text-left text-sm has-data-[slot=alert-action]:relative has-data-[slot=alert-action]:pr-18 has-[>svg]:grid-cols-[auto_1fr] has-[>svg]:gap-x-2 *:[svg]:row-span-2 *:[svg]:translate-y-0.5 *:[svg]:text-current *:[svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-card text-card-foreground",
        /*
         * Something is wrong but nothing has failed — a setting that will be
         * stored and do nothing, a build missing the feature it configures.
         *
         * These were `default`, which paints a card: an alert with a warning
         * triangle in it, in the same grey as the panel it sits on, reads as a
         * caption rather than as a warning, and the one thing it has to do is
         * be noticed. `destructive` is the wrong end — nothing here has broken
         * or been refused — so this is the amber the console already uses for
         * "blocked" everywhere else (`--status-blocked`), tinted rather than
         * filled so it still reads as a notice on the page and not an error
         * dialog.
         *
         * The text used to be `--foreground` with a `--muted-foreground`
         * description: neutral grey on an amber wash, which reads as two
         * components stacked rather than one notice — the tint says "warning"
         * and the words say "body copy". Both now take `--status-blocked-text`,
         * which is the amber the icon already used, so the whole alert is one
         * colour family. It stays legible at both ends: 4.80:1 in light
         * (`--amber-text` on a 16%-amber wash over the page ground) and
         * `--amber-bright` in dark, the same pairing every other blocked
         * surface in the console uses.
         */
        warning:
          "border-status-blocked/40 bg-status-blocked-soft text-status-blocked-text " +
          "*:data-[slot=alert-title]:text-status-blocked-text " +
          "*:data-[slot=alert-description]:text-status-blocked-text " +
          "*:[svg]:text-status-blocked-text " +
          // Inline `code` inherits the alert's colour instead of keeping the
          // page's neutral chip. A grey chip on an amber wash reads as a
          // fragment of some other component that landed here by accident —
          // which is exactly what a feature name in a warning must not look
          // like, since it is the part the reader has to act on.
          "[&_code]:bg-status-blocked/15 [&_code]:text-status-blocked-text " +
          "[&_code]:rounded [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-xs",
        destructive:
          "bg-card text-destructive *:data-[slot=alert-description]:text-destructive/90 *:[svg]:text-current",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

function Alert({
  className,
  variant,
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof alertVariants>) {
  // `role="alert"` is an assertive live region, so hard-coding it on every Alert
  // made a standing informational notice — one present on mount, like the
  // "Saved to this browser as a draft" note on Settings — interrupt a screen
  // reader on page load even though nothing had changed (issue #1392).
  //
  // The default is `role="status"`, not no role at all: a polite live region is
  // silent for a notice that is already there when the page renders, but still
  // announces one mounted in response to something the operator did — the
  // feedback form's success alert after Send, or the warning that appears when
  // the last teammate is removed in the setup wizard. Dropping the role
  // outright would have made those silent.
  return (
    <div
      data-slot="alert"
      role={variant === "destructive" ? "alert" : "status"}
      className={cn(alertVariants({ variant }), className)}
      {...props}
    />
  )
}

function AlertTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-title"
      className={cn(
        "font-medium group-has-[>svg]/alert:col-start-2 [&_a]:underline [&_a]:underline-offset-3 [&_a]:hover:text-foreground",
        className
      )}
      {...props}
    />
  )
}

function AlertDescription({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-description"
      className={cn(
        "text-sm text-balance text-muted-foreground md:text-pretty [&_a]:underline [&_a]:underline-offset-3 [&_a]:hover:text-foreground [&_p:not(:last-child)]:mb-4",
        className
      )}
      {...props}
    />
  )
}

function AlertAction({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-action"
      className={cn("absolute top-2 right-2", className)}
      {...props}
    />
  )
}

export { Alert, AlertTitle, AlertDescription, AlertAction }
