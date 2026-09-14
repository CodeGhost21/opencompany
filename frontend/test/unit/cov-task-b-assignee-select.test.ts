// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { AssigneeSelect } from "@/views/AssigneeSelect";

/**
 * `AssigneeSelect` carries no role check of its own — every call site
 * (`TaskDetailView`'s reassign row, `TaskEditDialog`, `CreateTaskDialog`)
 * drives its `disabled` prop off a local save-in-flight flag, never off a
 * viewer's role. That matches the host: `PATCH …/tasks/{id}`, which is where
 * an `assignee` write lands, is `ScopedCompany` (`src/server/ops/tasks.rs`),
 * open to any member. `assignee-roster-gap.test.ts` already pins the roster
 * read's own honesty (the gap notice, and the off-roster flag); it never
 * checks `disabled` itself, and never drives a pick through past a failed
 * half of the roster — both closed here.
 */

const DESKS = [{ id: "engineering", name: "Engineering", members: ["eng-lead"] }];
const TEAM = [{ id: "eng-lead", name: "Eng Lead", role: "engineer" }];

function fakeClient(
  over: { desks?: () => Promise<unknown>; team?: () => Promise<unknown> } = {},
): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    listDesks: vi.fn(over.desks ?? (async () => DESKS)),
    listTeam: vi.fn(over.team ?? (async () => TEAM)),
  } as unknown as OpenCompanyClient;
}

const fails = () => Promise.reject(new Error("host unreachable"));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

async function mount(
  client: OpenCompanyClient,
  opts: { value?: string; disabled?: boolean; onChange?: (next: string) => void } = {},
) {
  const onChange = opts.onChange ?? vi.fn();
  await act(async () => {
    root.render(
      createElement(AssigneeSelect, {
        id: "assignee",
        client,
        company: "acme",
        value: opts.value ?? "",
        disabled: opts.disabled,
        onChange,
      }),
    );
  });
  // The two roster reads settle in microtasks, and the state they set lands a
  // tick later — the same double flush `assignee-roster-gap.test.ts` uses.
  await act(async () => {});
  await act(async () => {});
  return onChange;
}

function trigger(): HTMLElement {
  return document.querySelector("#assignee") as HTMLElement;
}

async function open() {
  await act(async () => {
    trigger().click();
  });
}

function option(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="option"]')).find(
    (o) => o.textContent?.trim().startsWith(label),
  );
  if (!found) throw new Error(`no “${label}” option; saw: ${document.body.innerHTML}`);
  return found as HTMLElement;
}

describe("AssigneeSelect — reassignment offered to every member, not just an admin", () => {
  it("is not disabled and reaches the whole roster when the caller does not lock it", async () => {
    await mount(fakeClient());
    expect(trigger().getAttribute("aria-disabled")).not.toBe("true");
    expect(trigger().hasAttribute("disabled")).toBe(false);

    await open();
    // Both the desk and the teammate the roster carries are pickable — a
    // member gets the same full picker an admin would, since neither role is
    // ever asked here.
    expect(document.body.textContent).toContain("Engineering");
    expect(document.body.textContent).toContain("Eng Lead");
  });

  it("only ever disables through the prop the caller passes it (busy, never role)", async () => {
    await mount(fakeClient(), { disabled: true });
    expect(trigger().getAttribute("data-disabled")).not.toBeNull();
  });
});

describe("AssigneeSelect — a roster read that failed", () => {
  it("still lets Unassigned go through, rather than a stuck or dead picker", async () => {
    const onChange = await mount(fakeClient({ team: fails }));
    await open();

    expect(document.querySelector('[data-testid="assignee-roster-gap"]')).not.toBeNull();

    await act(async () => {
      option("Unassigned").click();
    });
    expect(onChange).toHaveBeenCalledWith("");
  });

  it("still lets a pick from the half that did arrive go through", async () => {
    const onChange = await mount(fakeClient({ team: fails }));
    await open();

    await act(async () => {
      option("Engineering").click();
    });
    expect(onChange).toHaveBeenCalledWith("engineering");
  });
});
