import { describe, expect, it } from "vitest";

import type { NotificationDto } from "@/api/types";
import {
  isOperationalNotification,
  operationalNotificationSeverity,
  operationalNotificationsToAnnounce,
} from "@/lib/operational-notifications";

/**
 * `mentionCountsByChannel` / `mentionsToClear` / `threadsToReReadForMentions`
 * (see `chat-mention-badge.test.ts`) all filter to `kind === "mention"` by
 * design, which left `dispatch_failed` / `approval_expired` /
 * `workflow_run_*` rows with no rendering and no acknowledgement path even
 * though `GET /notifications` returns them (Codex #1883 P1). These tests
 * pin the pure logic behind the toast-based fix.
 */

const note = (over: Partial<NotificationDto> & Pick<NotificationDto, "id" | "kind">): NotificationDto => ({
  subjectKind: "task",
  subjectId: "t-1",
  title: "A card's dispatch failed and returned to To-do: boom",
  createdAt: 1,
  ...over,
});

describe("isOperationalNotification", () => {
  it("is false for mentions", () => {
    expect(isOperationalNotification(note({ id: "a", kind: "mention" }))).toBe(false);
  });

  it("is true for every non-mention kind the runtime writes", () => {
    for (const kind of [
      "dispatch_failed",
      "approval_expired",
      "workflow_run_failed",
      "workflow_run_stranded",
      "workflow_run_blocked",
    ]) {
      expect(isOperationalNotification(note({ id: kind, kind }))).toBe(true);
    }
  });
});

describe("operationalNotificationsToAnnounce", () => {
  it("returns unread operational rows not already announced", () => {
    const rows = [
      note({ id: "a", kind: "dispatch_failed" }),
      note({ id: "b", kind: "mention" }),
      note({ id: "c", kind: "approval_expired" }),
    ];
    expect(operationalNotificationsToAnnounce(rows, new Set()).map((n) => n.id)).toEqual([
      "a",
      "c",
    ]);
  });

  it("excludes rows already read", () => {
    const rows = [note({ id: "a", kind: "dispatch_failed", readAt: 5 })];
    expect(operationalNotificationsToAnnounce(rows, new Set())).toEqual([]);
  });

  it("excludes rows already announced this session", () => {
    const rows = [note({ id: "a", kind: "dispatch_failed" })];
    expect(operationalNotificationsToAnnounce(rows, new Set(["a"]))).toEqual([]);
  });

  it("does not re-announce a row on the next poll once it is in the guard set", () => {
    const announced = new Set<string>();
    const rows = [note({ id: "a", kind: "dispatch_failed" })];
    const first = operationalNotificationsToAnnounce(rows, announced);
    expect(first.map((n) => n.id)).toEqual(["a"]);
    first.forEach((n) => announced.add(n.id));
    // The row is durable and keeps coming back on every poll until the
    // server marks it read — the guard, not the row disappearing, is what
    // stops the repeat toast.
    expect(operationalNotificationsToAnnounce(rows, announced)).toEqual([]);
  });
});

describe("operationalNotificationSeverity", () => {
  it("treats dispatch_failed as an error", () => {
    expect(operationalNotificationSeverity(note({ id: "a", kind: "dispatch_failed" }))).toBe(
      "error",
    );
  });

  it("treats every workflow_run_* kind as an error", () => {
    for (const kind of ["workflow_run_failed", "workflow_run_stranded", "workflow_run_blocked"]) {
      expect(operationalNotificationSeverity(note({ id: kind, kind }))).toBe("error");
    }
  });

  it("treats approval_expired as a warning", () => {
    expect(operationalNotificationSeverity(note({ id: "a", kind: "approval_expired" }))).toBe(
      "warning",
    );
  });

  it("defaults an unrecognized kind to warning rather than error", () => {
    expect(operationalNotificationSeverity(note({ id: "a", kind: "something_new" }))).toBe(
      "warning",
    );
  });
});
