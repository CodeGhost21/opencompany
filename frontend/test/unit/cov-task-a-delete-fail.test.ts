// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { Task } from "@/api/tasks";

/**
 * `DELETE …/tasks/{id}` (`delete_task`) is `ScopedCompany` too — any company
 * member, refused only for an in-flight card (`409`, pinned by
 * `task-edit-dialog-gates.test.ts`). What that file never drives is delete
 * actually failing for a reason other than the in-flight guard — a host that
 * is simply down when the confirm fires — and this pins the card staying put,
 * with an honest error, rather than the dialog closing on a delete that never
 * landed.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

const { TaskEditDialog } = await import("@/views/TaskEditDialog");

const TASK: Task = {
  id: "task-1",
  title: "Pay the invoice",
  note: "the original note",
  column: "pending",
  stage: "pending",
  priority: "medium",
  assignee: "",
  updatedAt: 1_700_000_000_000,
};

const LEDGERS = {
  ledgers: [
    {
      slug: "tasks",
      title: "Tasks",
      purpose: "The company's work board.",
      source: "native",
      derived: "derived/TASKS.md",
      writtenBy: "the board",
      builtin: true,
      fields: [],
      statuses: [
        { name: "pending", label: "Pending" },
        { name: "working", label: "Working" },
        { name: "done", label: "Done", closed: true },
      ],
      sections: [],
      open: 1,
      closed: 0,
    },
  ],
  faults: [],
  remaining: 3,
};

/** A member-shaped client — every read this dialog makes, and no `/auth/me`. */
function fakeClient(delImpl: (path: string) => Promise<void>) {
  const paths: string[] = [];
  const del = vi.fn(delImpl);
  const patch = vi.fn(async () => TASK);
  const client = {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    get: vi.fn(async (path: string) => {
      paths.push(path);
      return path.endsWith("/ledgers") ? LEDGERS : [];
    }),
    patch,
    del,
    listDesks: vi.fn(async () => []),
    listTeam: vi.fn(async () => []),
  } as unknown as OpenCompanyClient;
  return { client, del, paths };
}

let container: HTMLDivElement;
let root: Root;

function buttons(): HTMLButtonElement[] {
  return Array.from(document.querySelectorAll("button"));
}

function button(label: string): HTMLButtonElement {
  const found = buttons().find((b) => b.textContent?.trim() === label);
  if (!found) {
    throw new Error(
      `no “${label}” button; saw: ${buttons().map((b) => b.textContent?.trim()).join(" | ")}`,
    );
  }
  return found;
}

async function mount(client: OpenCompanyClient) {
  const onClose = vi.fn();
  const onSaved = vi.fn();
  const onDeleted = vi.fn();
  await act(async () => {
    root.render(
      createElement(TaskEditDialog, {
        task: TASK,
        onClose,
        onSaved,
        onDeleted,
        client,
        company: "acme",
        irreversible: [],
        historyIncomplete: false,
      }),
    );
  });
  await act(async () => {});
  await act(async () => {});
  return { onClose, onSaved, onDeleted };
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("Edit → Delete, by authority (delete_task's ScopedCompany write)", () => {
  it("offers a plain member client a live Delete on a card nothing is running, asking no role question first", async () => {
    const { client, paths } = fakeClient(async () => undefined);
    await mount(client);

    const remove = button("Delete");
    expect(remove.disabled).toBe(false);
    expect(paths.some((p) => p.includes("/auth/me"))).toBe(false);
  });
});

describe("Edit → Delete, a DELETE the host refuses", () => {
  it("shows the host's own message and leaves the card in place", async () => {
    const { client, del } = fakeClient(async () => {
      throw new ApiError(500, "internal", "the task store could not process this", true);
    });
    const { onDeleted } = await mount(client);

    await act(async () => {
      button("Delete").click();
    });
    await act(async () => {
      button("Delete task").click();
    });

    expect(del).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith("the task store could not process this");
    expect(onDeleted).not.toHaveBeenCalled();
    // Not silently vanished: the dialog is still open on the same card.
    expect(document.querySelector("#task-title")).not.toBeNull();
  });

  it("re-enables Delete rather than leaving the form stuck busy on a network failure", async () => {
    const { client } = fakeClient(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    await mount(client);

    await act(async () => {
      button("Delete").click();
    });
    await act(async () => {
      button("Delete task").click();
    });

    expect(toasts.error).toHaveBeenCalledWith("cannot reach the company host");
    expect(button("Delete").disabled).toBe(false);
  });
});
