// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { Task } from "@/api/tasks";

/**
 * `POST …/tasks` (`create_task`) sits behind `ScopedCompany` — any signed-in
 * company member, the same authority every other task write answers to — not
 * an admin-only guard. `CreateTaskDialog` never asks who the operator is
 * before offering the prompt box, which is the correct answer here: gating
 * task creation on a role would take away a capability this product
 * deliberately gives every member (`scope.rs`'s own words for the family).
 *
 * This is the cell's AUTH axis only. The prompt box's own gap — no length
 * cap, client or server — is reported as a finding rather than pinned here,
 * since inventing a cap this component does not have would pin a lie.
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

const { CreateTaskDialog } = await import("@/views/CreateTaskDialog");

/** A member-shaped client: every read this dialog makes, and no `/auth/me`. */
function fakeClient(post: (path: string, body?: unknown) => Promise<Task>) {
  const paths: string[] = [];
  const client = {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    get: vi.fn(async (path: string) => {
      paths.push(path);
      if (path.includes("/ledgers")) return { ledgers: [] };
      return {};
    }),
    listDesks: vi.fn(async () => []),
    listTeam: vi.fn(async () => []),
    post,
  } as unknown as OpenCompanyClient;
  return { client, paths };
}

let container: HTMLDivElement;
let root: Root;

async function mount(client: OpenCompanyClient) {
  const onClose = vi.fn();
  const onCreated = vi.fn();
  await act(async () => {
    root.render(
      createElement(CreateTaskDialog, {
        open: true,
        onClose,
        onCreated,
        client,
        company: "acme",
      }),
    );
  });
  await act(async () => {});
  await act(async () => {});
  return { onClose, onCreated };
}

function button(label: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLButtonElement | undefined;
}

function setValue(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("New task, by authority (create_task's ScopedCompany write)", () => {
  it("offers a plain member client the prompt box and a live Create, asking no role question first", async () => {
    const post = vi.fn(async () => ({ id: "task-9" }) as unknown as Task);
    const { client, paths } = fakeClient(post);
    await mount(client);

    const prompt = document.querySelector("#new-prompt") as HTMLTextAreaElement;
    expect(prompt).not.toBeNull();
    expect(prompt.disabled).toBe(false);

    await act(async () => {
      setValue(prompt, "reconcile March invoices");
    });
    expect(button("Create")!.disabled).toBe(false);

    expect(paths.some((p) => p.includes("/auth/me"))).toBe(false);
  });

  it("actually creates the card for that member, matching what create_task accepts from any member", async () => {
    const post = vi.fn(async () => ({ id: "task-9" }) as unknown as Task);
    const { client } = fakeClient(post);
    const { onCreated } = await mount(client);

    const prompt = document.querySelector("#new-prompt") as HTMLTextAreaElement;
    await act(async () => {
      setValue(prompt, "reconcile March invoices");
    });
    await act(async () => {
      button("Create")!.click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(onCreated).toHaveBeenCalledTimes(1);
  });
});
