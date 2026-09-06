// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { IrreversibleEffect, Task } from "@/api/tasks";

/**
 * The edit dialog must survive its own screen's poll.
 *
 * The Task Detail screen refetches the card every 4s and hands `TaskEditDialog`
 * a **new `detail.task` object** each tick with the same values. The seeding
 * effect used to be keyed on that object, so it re-ran on every tick and wiped
 * the draft: measured in a browser, a Column select set to Working read back
 * `Pending` 2.3s later and never recovered.
 *
 * The consequence is worse than a lost keystroke, which is why these are here
 * rather than in the gates file. After the reset `computeTaskPatch` returns
 * `{}` — so `confirmDispatch` is false and Save writes nothing at all: no
 * confirmation, no request, no toast. The irreversible-effects gate on the
 * Column select (issue #351, applied to this second dispatch path) was only
 * reachable by picking and saving within the same ~150ms. A guard nobody can
 * reach is not a guard.
 *
 * The last two tests pin the **deliberate cost** of the fix: with the draft
 * seeded on the card's identity, a server-side change to the card while this
 * dialog is open is not reflected here. That is the intended trade — the open
 * draft wins, because nothing else on the screen holds a copy of it — and it is
 * bounded, because the save diffs against the card as it read when the dialog
 * opened, so a field the operator never touched is never in the patch.
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

const PAID: IrreversibleEffect = {
  kind: "payment.send",
  atMillis: 1_699_999_000_000,
  amountUsd: 40,
};

/** The `tasks` ledger, so the Column select offers the real three phases. */
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

function fakeClient() {
  const patch = vi.fn(async (_path: string, body: unknown) => {
    void body;
    return { ...TASK, column: "working" } as Task;
  });
  const client = {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    get: vi.fn(async (path: string) => (path.endsWith("/ledgers") ? LEDGERS : [])),
    patch,
    del: vi.fn(async (_path: string) => undefined),
    listDesks: vi.fn(async () => []),
    listTeam: vi.fn(async () => []),
  } as unknown as OpenCompanyClient;
  return { client, patch };
}

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

/** The dialog and its confirmations render through portals onto `document.body`. */
function buttons(): HTMLButtonElement[] {
  return Array.from(document.querySelectorAll("button"));
}

function button(label: string): HTMLButtonElement {
  const found = buttons().find((b) => b.textContent?.trim() === label);
  if (!found) {
    throw new Error(
      `no “${label}” button; saw: ${buttons()
        .map((b) => b.textContent?.trim())
        .join(" | ")}`,
    );
  }
  return found;
}

function maybeButton(label: string): HTMLButtonElement | undefined {
  return buttons().find((b) => b.textContent?.trim() === label);
}

function option(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="option"]')).find(
    (o) => o.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no “${label}” option in the open popup`);
  return found as HTMLElement;
}

const titleInput = () => document.querySelector("#task-title") as HTMLInputElement;
const noteInput = () => document.querySelector("#task-note") as HTMLTextAreaElement;
/**
 * What a select trigger is showing. The trigger also renders a decorative
 * chevron glyph, so strip everything outside printable ASCII rather than
 * assert against a character the icon owns.
 */
function triggerText(id: string): string {
  return (document.querySelector(id)?.textContent ?? "").replace(/[^\x20-\x7E]/g, "").trim();
}

/** The Column trigger shows the label, not the id. */
const columnText = () => triggerText("#task-column");
const priorityText = () => triggerText("#task-priority");

function mounter(client: OpenCompanyClient, irreversible?: IrreversibleEffect[]) {
  /** Render (or re-render) with `task` — pass null for a closed dialog. */
  return async function show(task: Task | null) {
    await act(async () => {
      root.render(
        createElement(TaskEditDialog, {
          task,
          onClose: vi.fn(),
          onSaved: vi.fn(),
          onDeleted: vi.fn(),
          client,
          company: "acme",
          irreversible: irreversible ?? [],
          historyIncomplete: false,
        }),
      );
    });
    // The ledger and roster reads settle in microtasks and the state they set
    // lands a tick later; give React those ticks rather than assuming one flush
    // covers all three.
    await act(async () => {});
    await act(async () => {});
  };
}

/**
 * One poll tick: the same card, same values, **new object identity** — exactly
 * what `getTaskDetail` produces every `POLL_MS` on the Task Detail screen.
 */
async function pollTick(show: (t: Task | null) => Promise<void>, ticks = 1) {
  for (let i = 0; i < ticks; i += 1) await show({ ...TASK });
}

async function chooseColumn(label: string) {
  await act(async () => {
    (document.querySelector("#task-column") as HTMLElement).click();
  });
  await act(async () => {
    option(label).click();
  });
}

/** React installs its own value setter, so assign through the prototype. */
function setValue(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("an open edit dialog under the Task Detail poll", () => {
  it("keeps a Column choice when the poll returns the same card", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await chooseColumn("Working");
    expect(columnText()).toBe("Working");

    await pollTick(show, 3);

    // The measured symptom, inverted: it read “Pending” from 2.3s onward.
    expect(columnText()).toBe("Working");
  });

  it("keeps typed text in the title and note when the poll returns the same card", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => setValue(titleInput(), "Pay the invoice twice"));
    await act(async () => setValue(noteInput(), "half an hour of typing"));

    await pollTick(show, 3);

    expect(titleInput().value).toBe("Pay the invoice twice");
    expect(noteInput().value).toBe("half an hour of typing");
  });

  it("keeps a Priority choice when the poll returns the same card", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => {
      (document.querySelector("#task-priority") as HTMLElement).click();
    });
    await act(async () => {
      option("high").click();
    });
    expect(priorityText()).toBe("high");

    await pollTick(show, 3);

    expect(priorityText()).toBe("high");
  });

  /**
   * The load-bearing one. This is the whole reason the reset is urgent rather
   * than merely old: once the draft is wiped the patch is `{}`, so Save is a
   * silent no-op and the confirmation it gates never opens.
   */
  it("still reaches the dispatch confirmation after the poll has ticked", async () => {
    const { client, patch } = fakeClient();
    const show = mounter(client, [PAID]);
    await show(TASK);

    await chooseColumn("Working");
    await pollTick(show, 3);

    await act(async () => {
      button("Save").click();
    });

    expect(maybeButton("Save anyway")).toBeDefined();
    expect(patch).not.toHaveBeenCalled();

    await act(async () => {
      button("Save anyway").click();
    });
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ column: "working" });
  });

  it("still writes an edit that survived the poll", async () => {
    const { client, patch } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => setValue(noteInput(), "a rewritten note"));
    await pollTick(show, 3);

    await act(async () => {
      button("Save").click();
    });
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ note: "a rewritten note" });
  });

  /**
   * The poll must not manufacture unsaved changes either. An untouched dialog
   * that has sat through a few ticks still dismisses on Cancel without the
   * discard confirmation — which is only true because the diff is taken against
   * the card as it read when the dialog opened.
   */
  it("does not become dirty just because the poll ticked", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await pollTick(show, 3);

    await act(async () => {
      button("Cancel").click();
    });
    expect(document.body.textContent).not.toContain("Discard this edit?");
  });
});

describe("what seeding on the card's identity gives up", () => {
  /**
   * Deliberate, not an oversight: a second operator editing the same card
   * cannot retype the first one's open draft out from under them. A
   * half-written note exists nowhere else — nothing on the screen holds a copy,
   * which is why Cancel asks before discarding it — so the open draft wins and
   * the server-side change lands on the screen behind instead.
   */
  it("does not adopt a server-side change to the card while the dialog is open", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await show({ ...TASK, title: "renamed by somebody else", priority: "high" });

    expect(titleInput().value).toBe("Pay the invoice");
    expect(priorityText()).toBe("medium");

    // And it is not reported as an unsaved edit either. Diffing the untouched
    // draft against the *live* card would call this dirty and make Cancel stop
    // to ask about changes the operator never made.
    await act(async () => {
      button("Cancel").click();
    });
    expect(document.body.textContent).not.toContain("Discard this edit?");
  });

  /**
   * …and the cost stops there. The save diffs against the card as it read when
   * the dialog opened, so the stale title the operator never touched is not in
   * the patch. Diffing a frozen draft against the *live* task would resubmit
   * it — reverting somebody else's rename as a side effect of editing a note,
   * and re-validating an assignee nobody touched, which is the failure issue
   * #263's diff exists to prevent.
   */
  it("still sends only the fields the operator actually changed", async () => {
    const { client, patch } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => setValue(noteInput(), "a rewritten note"));
    await show({ ...TASK, title: "renamed by somebody else", column: "done" });

    await act(async () => {
      button("Save").click();
    });
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ note: "a rewritten note" });
  });
});

describe("when the draft is re-seeded", () => {
  it("re-seeds when the dialog is closed and opened again", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => setValue(noteInput(), "abandoned typing"));
    // Close (the detail screen passes null), then reopen on the current card.
    await show(null);
    await show({ ...TASK, note: "the note as the host now has it" });

    expect(noteInput().value).toBe("the note as the host now has it");
  });

  it("re-seeds when the screen swaps in a different card", async () => {
    const { client } = fakeClient();
    const show = mounter(client);
    await show(TASK);

    await act(async () => setValue(noteInput(), "typing about task-1"));
    // A lineage hop replaces the card without closing the dialog.
    await show({ ...TASK, id: "task-2", title: "Chase the refund", note: "a different note" });

    expect(titleInput().value).toBe("Chase the refund");
    expect(noteInput().value).toBe("a different note");
  });
});
