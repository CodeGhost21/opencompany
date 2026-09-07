// @vitest-environment jsdom
//
// The OTHER Add-teammate dialog — `views/chat/AddMemberDialog`, reached from
// chat's member pane, chat's empty state and the org chart's desk cards (issue
// #1989).
//
// # Why this file exists beside `team-add-one-box.test.ts`
//
// That file mounts `TeamView`, which has a dialog of its own. Three of the four
// entry points to Add teammate use this one instead, and it had no component
// test at all — so every claim about "the reduced dialog" was proved on the
// surface a minority of operators actually meet. The two dialogs share
// `addTeammateSurface`, `describedTeammateFields` and `DescribeTeammate`, and
// nothing else: the branch, the reset and the footer are duplicated in both, and
// duplicated code is exactly what drifts.
//
// The Cancel test below is the case in point. The bug it pins was present in
// both dialogs, in the same shape, and fixing one would have left the other.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { NewMemberFields } from "@/views/chat/AddMemberDialog";

const api = vi.hoisted(() => ({ getInferenceStatus: vi.fn() }));
vi.mock("@/api/inference", () => ({ getInferenceStatus: api.getInferenceStatus }));

const { AddMemberDialog } = await import("@/views/chat/AddMemberDialog");

let container: HTMLDivElement;
let root: Root;
let added: NewMemberFields[];
/** Every `onOpenChange` the dialog reported, so a Cancel that never closed is visible. */
let openChanges: boolean[];
let open: boolean;

const client = { scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}` };

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  added = [];
  openChanges = [];
  open = false;
  vi.clearAllMocks();
  api.getInferenceStatus.mockResolvedValue({ cognition: "harness" });
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(
      createElement(AddMemberDialog, {
        open,
        onOpenChange: (next: boolean) => {
          openChanges.push(next);
          open = next;
          void render();
        },
        onAdd: (fields: NewMemberFields) => {
          added.push(fields);
        },
        client: client as unknown as OpenCompanyClient,
        company: "acme",
      }),
    );
  });
  await act(async () => {});
}

/** Opens the dialog and lets its cognition read land. */
async function openDialog() {
  open = true;
  await render();
}

function byText(tag: string, text: string): HTMLElement | undefined {
  return Array.from(document.querySelectorAll<HTMLElement>(tag)).find(
    (el) => el.textContent?.trim() === text,
  );
}

/** Types into a controlled input/textarea the way React sees it. */
function type(testId: string, value: string) {
  const el = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(
    `[data-testid="${testId}"]`,
  );
  if (!el) throw new Error(`no field [data-testid="${testId}"]`);
  const proto =
    el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** The footer's Add teammate — the dialog is open, so it is the last one. */
async function pressCreate() {
  const buttons = Array.from(document.querySelectorAll<HTMLElement>("button")).filter(
    (el) => el.textContent?.trim() === "Add teammate",
  );
  await act(async () => {
    buttons[buttons.length - 1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await act(async () => {});
}

async function pressCancel() {
  await act(async () => {
    byText("button", "Cancel")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await act(async () => {});
}

const box = '[data-testid="team-describe-box"]';
const roleField = "#member-role";

describe("chat's reduced Add-teammate dialog (issue #1989)", () => {
  it("renders one box, and creates with the sentence as the role", async () => {
    await openDialog();

    expect(document.querySelector(box), "the description box must be on screen").not.toBeNull();
    expect(document.querySelector(roleField), "Role is derived, not asked for").toBeNull();

    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs wholesale outreach and keeps the stockist pipeline warm.");
    await pressCreate();

    expect(added).toHaveLength(1);
    // Whole, un-truncated, and no ellipsis: this description has no clause
    // break, which is the case the old first-clause-then-cut derivation turned
    // into "Runs wholesale outreach and keeps the…" and stored as a job title.
    expect(added[0].role).toBe("Runs wholesale outreach and keeps the stockist pipeline warm");
    expect(added[0].landOnProfile).toBe(true);
  });

  it("hands over the full form rather than truncating a long description", async () => {
    await openDialog();

    type("team-describe-name", "Sable");
    type(
      "team-describe-box",
      "Runs wholesale outreach to boutique retailers and keeps the stockist pipeline " +
        "warm. Reports on reorder rates every month, by account.",
    );
    await pressCreate();

    // Nothing written. The operator is asked for a role rather than given one
    // cut out of the middle of their own sentence.
    expect(added).toHaveLength(0);
    expect(document.querySelector(roleField), "the full form must be on screen").not.toBeNull();
    expect(document.querySelector('[data-testid="chat-add-handover"]')).not.toBeNull();
  });

  it("Cancel clears the hand-over and what was typed", async () => {
    // The same bug this dialog's sibling had: `reset` hung off the wrapper
    // Radix calls, and Cancel called the raw `onOpenChange(false)` prop past
    // it. So Escape cleared the dialog and Cancel did not, and one hand-over
    // cancelled rather than escaped retired the reduced dialog for the rest of
    // the page's life.
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "...");
    await pressCreate();
    expect(document.querySelector(roleField), "the hand-over must have happened").not.toBeNull();

    await pressCancel();
    expect(openChanges).toContain(false);
    await openDialog();

    expect(document.querySelector(box), "the reduced dialog must be back").not.toBeNull();
    expect(document.querySelector(roleField), "the full form must be gone").toBeNull();
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="team-describe-name"]')!.value,
      "and nothing typed into the abandoned attempt survives",
    ).toBe("");
  });
});

describe("chat's full Add-teammate form on a company that cannot draft", () => {
  it("keeps every field, because nothing downstream could draft them", async () => {
    api.getInferenceStatus.mockResolvedValue({ cognition: "echo" });
    await openDialog();

    // The reduced dialog is a handoff to a page whose copilot is switched off
    // on this path (`AgentDetailView`'s `cognition === "echo"` guard), so the
    // fields it stops asking for would be askable nowhere.
    expect(document.querySelector(box)).toBeNull();
    expect(document.querySelector(roleField)).not.toBeNull();
    expect(byText("span", "Give this teammate an inbox")).not.toBeUndefined();
  });
});
