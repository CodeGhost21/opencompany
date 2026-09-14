// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { Task } from "@/api/tasks";

/**
 * `PATCH …/tasks/{id}` (`patch_task`) is `ScopedCompany` — any company member,
 * the same authority `create_task` and every other task write answer to — so
 * Save is not, and must not become, an admin-only control. And no e2e ever
 * drove an actual PATCH from this dialog: `task-edit-dialog-gates.test.ts`
 * pins the dispatch confirmation and the roster/cancel/delete states, all
 * against a client that resolves. What is missing is the honest case — the
 * host refuses the write — which this file drives.
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
function fakeClient(patchImpl: (path: string, body: unknown) => Promise<Task>) {
  const paths: string[] = [];
  const patch = vi.fn(patchImpl);
  const del = vi.fn(async () => undefined);
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
  return { client, patch, del, paths };
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

function setValue(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(el),
    "value",
  )?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
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

describe("Edit → Save, by authority (patch_task's ScopedCompany write)", () => {
  it("offers a plain member client a live Save, asking no role question first", async () => {
    const { client, paths } = fakeClient(async () => ({ ...TASK, title: "x" }));
    await mount(client);

    const save = button("Save");
    expect(save.disabled).toBe(false);
    expect(paths.some((p) => p.includes("/auth/me"))).toBe(false);
  });
});

describe("Edit → Save, a PATCH the host refuses", () => {
  it("shows the host's own message and leaves the dialog open, unsaved", async () => {
    const { client, patch } = fakeClient(async () => {
      throw new ApiError(409, "conflict", "this card was moved since you opened it", true);
    });
    const { onSaved, onClose } = await mount(client);

    const title = document.querySelector("#task-title") as HTMLInputElement;
    await act(async () => {
      setValue(title, "Pay the March invoice");
    });
    await act(async () => {
      button("Save").click();
    });

    expect(patch).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith("this card was moved since you opened it");
    expect(onSaved).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    // The typed edit is not thrown away by a failed save — it is still there
    // to retry or copy out.
    expect((document.querySelector("#task-title") as HTMLInputElement).value).toBe(
      "Pay the March invoice",
    );
  });

  it("re-enables Save rather than leaving the form stuck busy", async () => {
    const { client } = fakeClient(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    await mount(client);

    const title = document.querySelector("#task-title") as HTMLInputElement;
    await act(async () => {
      setValue(title, "Pay the March invoice");
    });
    await act(async () => {
      button("Save").click();
    });

    expect(toasts.error).toHaveBeenCalledWith("cannot reach the company host");
    expect(button("Save").disabled).toBe(false);
  });
});
