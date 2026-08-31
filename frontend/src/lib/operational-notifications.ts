import type { NotificationDto } from "@/api/types";

/**
 * The non-mention rows the honest-verdicts work (issue #1865) started
 * writing through the same durable store `GET /notifications` returns:
 * `dispatch_failed`, `approval_expired`, `workflow_run_failed` /
 * `_stranded` / `_blocked`.
 *
 * Every consumer of that feed on the console — `mentionCountsByChannel`,
 * `mentionsToClear`, `threadsToReReadForMentions` — filters to
 * `kind === "mention"` by design (a reply or reaction must not silently
 * start badging as a summons). That is correct for those three, but it left
 * these rows with nothing: no badge, no rendered item anywhere, and no path
 * back to the server to mark them read, so they sat unread forever despite
 * being returned on every poll (Codex #1883 P1). This module is the minimal
 * surface that closes the loop — a one-shot toast per row, immediately
 * eligible to be marked read the same way a viewed mention is.
 */
export function isOperationalNotification(notification: NotificationDto): boolean {
  return notification.kind !== "mention";
}

/**
 * Unread operational rows not yet announced this session.
 *
 * `announced` is the caller's running set of ids already toasted — a row is
 * durable and keeps coming back on every poll until it is marked read, so
 * without this guard the same dispatch failure would toast once per poll
 * interval rather than once, ever.
 */
export function operationalNotificationsToAnnounce(
  notifications: readonly NotificationDto[],
  announced: ReadonlySet<string>,
): NotificationDto[] {
  return notifications.filter(
    (n) => n.readAt === undefined && isOperationalNotification(n) && !announced.has(n.id),
  );
}

/** Toast severity for an operational row's `kind`. */
export type OperationalNotificationSeverity = "error" | "warning";

/**
 * `dispatch_failed` and every `workflow_run_*` kind name a run that did not
 * complete — an error. `approval_expired` is a deadline that passed rather
 * than a failure in the strict sense, so it gets the lighter warning
 * treatment. An unrecognized future kind defaults to warning rather than
 * error: this module cannot know its severity, and under-alarming a novel
 * kind is the safer default than crying wolf on it.
 */
export function operationalNotificationSeverity(
  notification: NotificationDto,
): OperationalNotificationSeverity {
  if (notification.kind === "dispatch_failed" || notification.kind.startsWith("workflow_run_")) {
    return "error";
  }
  return "warning";
}
