// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

const toasts = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({
  toast: { success: toasts.success, error: toasts.error, warning: vi.fn(), info: vi.fn() },
}));

/**
 * `renameMoveNode` (`PATCH …/workspace/{id}`) carries no version token at
 * all, so two operators moving the same note from two tabs is resolved
 * entirely by whichever PATCH the host answers last — there is nothing
 * client-side to arbitrate. What a frontend test CAN prove about that race
 * is the losing tab's half: `move()` applies the host's answer only after
 * `await`ing it (`WorkspaceView.tsx`), never optimistically, so a PATCH the
 * host refuses — the ordinary shape of losing a race, since the node the
 * losing tab named may already be gone or already moved — must leave the
 * tree exactly as it was and say so, not silently believe the move landed.
 */

function node(over: { id: string; name: string; kind: "folder" | "file"; parentId?: string }) {
  return { ...over, updatedAt: 1 };
}

const TREE = [
  node({ id: "product", name: "Product", kind: "folder" }),
  node({ id: "archive", name: "Archive", kind: "folder" }),
  node({ id: "note", name: "Plan.md", kind: "file" }),
];

let container: HTMLDivElement;
let root: Root;
let client: {
  scopeFor: () => string;
  get: ReturnType<typeof vi.fn>;
  patch: ReturnType<typeof vi.fn>;
  listTeam: ReturnType<typeof vi.fn>;
};

function host(team: TeamMemberDto[], patch: ReturnType<typeof vi.fn>) {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue(TREE),
    patch,
    listTeam: vi.fn().mockResolvedValue(team),
  };
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollIntoView = vi.fn();
  localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  toasts.success.mockClear();
  toasts.error.mockClear();
});

function menuItem(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="menuitem"]')).find(
    (el) => el.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no "${label}" menu item in:\n${document.body.innerHTML}`);
  return found as HTMLElement;
}

function actionsButtonFor(name: string): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll("div.group")).find((d) =>
    d.textContent?.includes(name),
  );
  const found = row?.querySelector('[aria-label="Actions"]');
  if (!found) throw new Error(`no Actions button for "${name}"`);
  return found as HTMLButtonElement;
}

async function openMoveDialog(patch: ReturnType<typeof vi.fn>) {
  client = host([], patch);
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(WorkspaceView, {
          client: client as unknown as OpenCompanyClient,
          company: "acme",
        }),
      }),
    );
  });
  await act(async () => {
    actionsButtonFor("Plan").click();
  });
  await act(async () => {
    menuItem("Move to…").click();
  });
  await act(async () => {
    (
      Array.from(
        document.querySelectorAll<HTMLButtonElement>('[data-testid="workspace-move-dest"]'),
      ).find((d) => d.textContent?.includes("Archive")) ?? null
    )?.click();
  });
}

describe("losing a move race: the host's PATCH is refused", () => {
  it("leaves the note exactly where it was and reports the refusal, rather than believing the move landed", async () => {
    const patch = vi.fn(() =>
      Promise.reject(
        new ApiError(404, "not_found", "this item no longer exists — it may have just been moved or deleted"),
      ),
    );
    await openMoveDialog(patch);

    await act(async () => {
      (
        document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement
      ).click();
    });

    expect(patch).toHaveBeenCalledTimes(1);
    // Still at the workspace root's own depth, not nested one level in under
    // Archive — a client that applied the move optimistically would indent
    // this row regardless of what the host answered.
    const planName = container.querySelector('[data-testid="workspace-tree-name"][title="Plan"]');
    const planRow = planName?.closest<HTMLElement>('[style*="padding-left"]');
    expect(planRow).toBeTruthy();
    expect(planRow!.style.paddingLeft).toBe("6px");
    expect(toasts.error).toHaveBeenCalledWith(
      "this item no longer exists — it may have just been moved or deleted",
    );
  });
});
