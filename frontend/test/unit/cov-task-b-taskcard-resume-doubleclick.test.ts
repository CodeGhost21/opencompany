// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Task } from "@/api/tasks";
import { taskApprovalRows } from "@/lib/task-approvals";
import { TaskItem } from "@/views/TaskCard";

/**
 * A real double-click on the board card's Resume button.
 *
 * `task-blocked-card.test.ts` proves Resume is *disabled* whenever the card's
 * own approvals are outstanding — the derived-state half of the rule. It never
 * drives two rapid clicks through the button while the card is clear to
 * resume, which is the other half: nothing in `resumeButton`'s `onClick`
 * depends on anything that changes between the two clicks of a real
 * double-click, so a debounce would have to live in the handler itself.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function card(): Task {
  return {
    id: "task-1",
    title: "Triage the release blockers",
    column: "working",
    stage: "paused",
    priority: "high",
    assignee: "qa",
    updatedAt: T0,
  } as Task;
}

let container: HTMLDivElement;
let root: Root;

async function render(onResume: () => void) {
  await act(async () => {
    root.render(
      createElement(TaskItem, {
        task: card(),
        dragging: false,
        // No approvals outstanding: the card is clear to resume, which is the
        // only state a real double-click on a live Resume is possible in.
        rows: taskApprovalRows([], {}, "task-1"),
        now: T0 + 60_000,
        askerNames: new Map(),
        deciding: new Map(),
        failed: {},
        onDecide: () => {},
        onOpen: () => {},
        onResume,
      }),
    );
  });
}

function resumeButton(): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Resume"),
  );
  if (!found) throw new Error(`no Resume button in:\n${container.innerHTML}`);
  return found as HTMLButtonElement;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("A real double-click on Resume", () => {
  // FINDING: `TaskItem`'s Resume button has no click-time debounce of its own
  // — `disabled={rows.length > 0}` is the only gate, and `rows` does not
  // change between two synchronous clicks. Two clicks before the caller's
  // `onResume` has had a chance to do anything both reach it, so a real
  // double-click can fire two re-dispatches. Confirmed failing against the
  // current source (frontend/src/views/TaskCard.tsx, the Resume `onClick`
  // handler) before being marked skip; not fixed here (frontend-only,
  // test-only task) — see the report.
  it.skip("reaches the resume handler once, not twice, for two clicks before either settles", async () => {
    const onResume = vi.fn();
    await render(onResume);

    const button = resumeButton();
    expect(button.disabled).toBe(false);

    // Two synchronous handler invocations, exactly as two real clicks would
    // land before a re-render (or any async work `onResume` kicks off) has a
    // chance to run.
    await act(async () => {
      button.click();
      button.click();
    });

    expect(onResume).toHaveBeenCalledTimes(1);
  });
});

describe("A resume that did not land", () => {
  it("leaves Resume live rather than stuck disabled — the card holds no busy state of its own", async () => {
    // A rejected `patchTask` inside the caller's `onResume` (e.g. `LedgersView`'s
    // `resume()`, which toasts the failure) — represented here as a promise
    // the card is never given back, since `onResume: () => void` is
    // fire-and-forget from `TaskItem`'s own point of view.
    const onResume = vi.fn(() => {
      void Promise.reject(new Error("network_error")).catch(() => {});
    });
    await render(onResume);

    const button = resumeButton();
    expect(button.disabled).toBe(false);

    await act(async () => {
      button.click();
    });
    // Never held down waiting on a caller's own async work — the card holds
    // no "resuming" state of its own that a rejection could leave stuck.
    expect(button.disabled).toBe(false);

    await act(async () => {
      button.click();
    });
    expect(onResume).toHaveBeenCalledTimes(2);
  });
});
