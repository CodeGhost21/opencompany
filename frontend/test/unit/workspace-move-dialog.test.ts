// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { WorkspaceView } from "@/views/WorkspaceView";

/**
 * Issue #1381: the Move dialog listed every folder by bare name, unsorted,
 * unpathed, offered `derived/`, and committed on the first click.
 *
 * Four defects each sufficient on its own to silently re-file a note: two
 * `Drafts` under different parents were identical rows; the order was the
 * host's unspecified `tree()` order beside a tree that was sorted; a roster
 * folder was a raw ULID; `derived/` was offered although the host refuses every
 * write under it (#1222); and one click moved the note with no Move button, no
 * Cancel and no undo.
 */

function node(over: { id: string; name: string; kind: "folder" | "file"; parentId?: string }) {
  return { ...over, updatedAt: 1 };
}

const TREE = [
  node({ id: "secrets", name: "secrets", kind: "folder" }),
  node({ id: "standards", name: "Standards", kind: "folder" }),
  node({
    id: "s-drafts",
    name: "Drafts",
    kind: "folder",
    parentId: "standards",
  }),
  node({ id: "product", name: "Product", kind: "folder" }),
  node({ id: "p-drafts", name: "Drafts", kind: "folder", parentId: "product" }),
  node({ id: "derived", name: "derived", kind: "folder" }),
  node({ id: "d-goals", name: "Goals", kind: "folder", parentId: "derived" }),
  node({ id: "agents", name: "Agents", kind: "folder" }),
  node({
    id: "roster",
    name: "01JQZY8T7K",
    kind: "folder",
    parentId: "agents",
  }),
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

function host(team: TeamMemberDto[]) {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn().mockResolvedValue(TREE),
    patch: vi.fn(async () =>
      node({ id: "note", name: "Plan.md", kind: "file", parentId: "product" }),
    ),
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
});

function menuItem(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="menuitem"]')).find(
    (el) => el.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no “${label}” menu item in:\n${document.body.innerHTML}`);
  return found as HTMLElement;
}

function actionsButtonFor(name: string): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll("div.group")).find((d) =>
    d.textContent?.includes(name),
  );
  const found = row?.querySelector('[aria-label="Actions"]');
  if (!found) throw new Error(`no Actions button for “${name}”`);
  return found as HTMLButtonElement;
}

function destinations(): HTMLButtonElement[] {
  return Array.from(
    document.querySelectorAll('[data-testid="workspace-move-dest"]'),
  ) as HTMLButtonElement[];
}

function labels(): string[] {
  return destinations().map(
    (d) =>
      d.textContent
        ?.replace("Here", "")
        // The audience marker (#1465) rides on the row; the label is the path.
        .replace(/Hides it|Shares it/, "")
        .trim() ?? "",
  );
}

async function openMove(
  team: TeamMemberDto[] = [{ id: "01JQZY8T7K", name: "Nadia", role: "Nadia" }],
) {
  client = host(team);
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
}

describe("the Move dialog says where each destination is (issue #1381)", () => {
  it("tells two same-named folders apart by path", async () => {
    await openMove();

    expect(labels()).toContain("Standards / Drafts");
    expect(labels()).toContain("Product / Drafts");
    // The defect: both rows read "Drafts".
    expect(labels().filter((l) => l === "Drafts")).toHaveLength(0);
  });

  it("lists them in the order the explorer draws them", async () => {
    await openMove();
    const folders = labels().filter((l) => l !== "Workspace root");

    expect(folders).toEqual([
      "Agents",
      "Agents / Nadia",
      "Product",
      "Product / Drafts",
      "secrets",
      "Standards",
      "Standards / Drafts",
    ]);
  });

  it("resolves a roster folder rather than showing its id", async () => {
    await openMove();
    expect(labels()).toContain("Agents / Nadia");
    expect(labels().join(" ")).not.toContain("01JQZY8T7K");
  });

  it("does not offer derived/, where the host refuses every write", async () => {
    await openMove();
    const joined = labels().join(" ");

    expect(joined).not.toContain("derived");
    // Nor anything beneath it.
    expect(joined).not.toContain("Goals");
  });
});

describe("picking a destination is not the same as moving (issue #1381)", () => {
  it("does not move on the first click", async () => {
    await openMove();
    const target = destinations().find((d) => d.textContent?.includes("Product / Drafts"));

    await act(async () => {
      target?.click();
    });

    // The defect: this click was the move.
    expect(client.patch).not.toHaveBeenCalled();
    expect(target?.getAttribute("aria-pressed")).toBe("true");
  });

  it("keeps Move disabled until something is picked", async () => {
    await openMove();
    const confirm = document.querySelector(
      '[data-testid="workspace-move-confirm"]',
    ) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    await act(async () => {
      destinations()[1]?.click();
    });
    expect(
      (document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("moves to the picked folder only when Move is pressed", async () => {
    await openMove();

    await act(async () => {
      destinations()
        .find((d) => d.textContent?.includes("Product / Drafts"))
        ?.click();
    });
    await act(async () => {
      (
        document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement
      ).click();
    });

    expect(client.patch).toHaveBeenCalledTimes(1);
    const [, body] = client.patch.mock.calls[0] as [string, { parentId: string }];
    expect(body.parentId).toBe("p-drafts");
  });

  it("Cancel closes without moving", async () => {
    await openMove();

    await act(async () => {
      destinations()[1]?.click();
    });
    await act(async () => {
      (
        Array.from(document.querySelectorAll("button")).find(
          (b) => b.textContent?.trim() === "Cancel",
        ) as HTMLButtonElement
      ).click();
    });

    expect(client.patch).not.toHaveBeenCalled();
    expect(destinations()).toHaveLength(0);
  });
});

describe("a long destination list can be narrowed (issue #1381)", () => {
  it("filters on the path label", async () => {
    await openMove();
    const box = document.querySelector('[data-testid="workspace-move-filter"]') as HTMLInputElement;

    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(box, "product /");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(labels()).toEqual(["Product / Drafts"]);
  });
});

/**
 * The audience half (issue #1465) meeting select-then-confirm (issue #1381).
 *
 * #1465 shipped the warning on a dialog where an ordinary destination committed
 * on one click, so the warning *was* the confirm step. Now every destination
 * has a Move button — so the warning has to be what that button leads to for a
 * boundary-crossing destination, rather than being skipped because a confirm
 * already happened.
 */
describe("moving across the secrets boundary still asks (issues #1381, #1465)", () => {
  it("marks the row before it is picked", async () => {
    await openMove();
    const secrets = destinations().find((d) => d.textContent?.startsWith("secrets"));

    expect(secrets?.getAttribute("data-audience-change")).toBe("hidden");
    expect(secrets?.textContent).toContain("Hides it");
  });

  it("does not move when Move is pressed — it names the consequence first", async () => {
    await openMove();

    await act(async () => {
      destinations()
        .find((d) => d.textContent?.startsWith("secrets"))
        ?.click();
    });
    await act(async () => {
      (
        document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement
      ).click();
    });

    expect(client.patch).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("This changes who can read it.");
  });

  it("moves once the consequence is acknowledged", async () => {
    await openMove();

    await act(async () => {
      destinations()
        .find((d) => d.textContent?.startsWith("secrets"))
        ?.click();
    });
    await act(async () => {
      (
        document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement
      ).click();
    });
    const confirm = Array.from(document.querySelectorAll("button")).find((b) =>
      /hide|move/i.test(b.textContent ?? ""),
    );
    await act(async () => {
      confirm?.click();
    });

    expect(client.patch).toHaveBeenCalledTimes(1);
    expect((client.patch.mock.calls[0] as [string, { parentId: string }])[1].parentId).toBe(
      "secrets",
    );
  });

  it("leaves an ordinary destination one press away, as before", async () => {
    await openMove();

    await act(async () => {
      destinations()
        .find((d) => d.textContent?.includes("Product / Drafts"))
        ?.click();
    });
    await act(async () => {
      (
        document.querySelector('[data-testid="workspace-move-confirm"]') as HTMLButtonElement
      ).click();
    });

    expect(client.patch).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).not.toContain("This changes who can read it.");
  });
});
