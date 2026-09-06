// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InflightRun, IrreversibleEffect, Task } from "@/api/tasks";

/**
 * The four states the edit dialog used to get wrong, all of them claims about
 * what the DOM does across a click sequence rather than anything a pure helper
 * can pin on its own — so this renders the dialog, the way
 * `ledger-retire-confirm.test.ts` does for the same reason.
 *
 *   1. **Column → Working is a dispatch.** It sends the identical
 *      `PATCH { column: "working" }` the Task Detail Retry button sends, and
 *      Retry has been gated behind issue #351's irreversible-effects
 *      confirmation since that issue shipped. Save was a plain button.
 *   2. **Delete is refused for an in-flight card.** `delete_task` answers `409`
 *      on purpose (issue #984), so offering the button makes the operator
 *      discover the rule as a failed click.
 *   3. **Cancel discards an unsaved edit.** The note textarea holds text
 *      nothing else on the screen has a copy of.
 *   4. **The assignee picker stayed live during a save**, unlike its twin in
 *      the detail screen's reassign row.
 *
 * The load-bearing assertion in (1) is the *negative* one: `client.patch` must
 * not have been called when Save was pressed. A confirmation that opens and
 * dispatches anyway is worse than none, because it reads as a guard.
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

const PAID: IrreversibleEffect = { kind: "payment.send", atMillis: 1_699_999_000_000, amountUsd: 40 };

const RUNNING: InflightRun = {
  taskId: "task-1",
  key: "task-1",
  kind: "task",
  title: "Pay the invoice",
  agentId: "finance",
  startedAt: 1_700_000_000_000,
  pendingAction: null,
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
  // Typed with the arguments `patchTask` actually passes, so `mock.calls[0][1]`
  // is the request body rather than a hole in an empty tuple — the drift
  // `tsconfig.unit.json` exists to catch.
  const patch = vi.fn(async (_path: string, body: unknown) => {
    void body;
    return { ...TASK, column: "working" } as Task;
  });
  const del = vi.fn(async (_path: string) => undefined);
  const client = {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    get: vi.fn(async (path: string) => (path.endsWith("/ledgers") ? LEDGERS : [])),
    patch,
    del,
    listDesks: vi.fn(async () => []),
    listTeam: vi.fn(async () => []),
  } as unknown as OpenCompanyClient;
  return { client, patch, del };
}

let container: HTMLDivElement;
let root: Root;

/**
 * Both the edit dialog and the confirmations inside it render through portals
 * onto `document.body`, so every lookup searches the whole document rather than
 * `container`.
 */
function buttons(): HTMLButtonElement[] {
  return Array.from(document.querySelectorAll("button"));
}

function button(label: string): HTMLButtonElement {
  const found = buttons().find((b) => b.textContent?.trim() === label);
  if (!found) throw new Error(`no “${label}” button; saw: ${buttons().map((b) => b.textContent?.trim()).join(" | ")}`);
  return found;
}

function maybeButton(label: string): HTMLButtonElement | undefined {
  return buttons().find((b) => b.textContent?.trim() === label);
}

/** The Column select's option row, once the popup is open. */
function option(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="option"]')).find(
    (o) => o.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no “${label}” option in the open popup`);
  return found as HTMLElement;
}

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

interface Options {
  inflight?: InflightRun | null;
  irreversible?: IrreversibleEffect[];
  historyIncomplete?: boolean;
  task?: Task;
}

async function mount(client: OpenCompanyClient, opts: Options = {}) {
  const onClose = vi.fn();
  const onSaved = vi.fn();
  const onDeleted = vi.fn();
  await act(async () => {
    root.render(
      createElement(TaskEditDialog, {
        task: opts.task ?? TASK,
        onClose,
        onSaved,
        onDeleted,
        client,
        company: "acme",
        inflight: opts.inflight,
        irreversible: opts.irreversible,
        historyIncomplete: opts.historyIncomplete,
      }),
    );
  });
  // The ledger read and the roster reads settle in microtasks, and the state
  // they set lands a tick later; give React those ticks rather than assuming
  // one flush covers all three.
  await act(async () => {});
  await act(async () => {});
  return { onClose, onSaved, onDeleted };
}

/** Open the Column select and pick `label`. */
async function chooseColumn(label: string) {
  await act(async () => {
    (document.querySelector("#task-column") as HTMLElement).click();
  });
  await act(async () => {
    option(label).click();
  });
}

describe("moving a card into Working from the edit dialog (issue #351)", () => {
  it("does not dispatch on Save until the confirmation is accepted", async () => {
    const { client, patch } = fakeClient();
    await mount(client, { irreversible: [PAID], historyIncomplete: false });

    await chooseColumn("Working");
    await act(async () => {
      button("Save").click();
    });

    // The whole point. The confirmation is open and nothing has been written.
    expect(patch).not.toHaveBeenCalled();
    expect(maybeButton("Save anyway")).toBeDefined();

    await act(async () => {
      button("Save anyway").click();
    });
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ column: "working" });
  });

  it("names what the card already did, and how long ago", async () => {
    const { client } = fakeClient();
    await mount(client, { irreversible: [PAID], historyIncomplete: false });

    await chooseColumn("Working");
    await act(async () => {
      button("Save").click();
    });

    const text = document.body.textContent ?? "";
    // The mechanism first: nothing about a Column select says "dispatch".
    expect(text).toContain("Moving this card into Working runs it again");
    expect(text).toContain("cannot be undone");
    // Rendered through `effectDone`, so the kind is never shown raw.
    expect(text).not.toContain("payment.send");
  });

  it("says a gap is a gap when the journal cannot describe its own history", async () => {
    const { client, patch } = fakeClient();
    await mount(client, { irreversible: [], historyIncomplete: true });

    await chooseColumn("Working");
    await act(async () => {
      button("Save").click();
    });

    expect(patch).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("nothing here can be listed");
  });

  /**
   * The other half of the fix. A confirmation on every dispatch would train the
   * operator to click through it, and it would then be gone on the card that
   * had actually spent money.
   */
  it("saves in one click on a card whose journal is clean", async () => {
    const { client, patch } = fakeClient();
    await mount(client, { irreversible: [], historyIncomplete: false });

    await chooseColumn("Working");
    await act(async () => {
      button("Save").click();
    });

    expect(maybeButton("Save anyway")).toBeUndefined();
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ column: "working" });
  });

  it("leaves a save that is not a dispatch alone, however dirty the card's past", async () => {
    const { client, patch } = fakeClient();
    await mount(client, { irreversible: [PAID], historyIncomplete: true });

    const note = document.querySelector("#task-note") as HTMLTextAreaElement;
    await act(async () => {
      setValue(note, "a rewritten note");
    });
    await act(async () => {
      button("Save").click();
    });

    expect(maybeButton("Save anyway")).toBeUndefined();
    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][1]).toEqual({ note: "a rewritten note" });
  });

  /**
   * A caller that has not wired the read cannot claim the card is clean, so it
   * gets the confirmation. Without this the guard would sit silently dead in
   * any caller that forgot the props — indistinguishable from working.
   */
  it("confirms when the caller passed no effect history at all", async () => {
    const { client, patch } = fakeClient();
    await mount(client);

    await chooseColumn("Working");
    await act(async () => {
      button("Save").click();
    });

    expect(patch).not.toHaveBeenCalled();
    expect(maybeButton("Save anyway")).toBeDefined();
  });
});

describe("Delete on a card the host will refuse (issue #984)", () => {
  it("is disabled while a run holds the card, and says what to do instead", async () => {
    const { client, del } = fakeClient();
    await mount(client, { inflight: RUNNING, irreversible: [], historyIncomplete: false });

    const remove = button("Delete");
    expect(remove.disabled).toBe(true);
    // Matched to the host's own refusal rather than invented here: it names
    // cancelling first and says why deleting now would not remove the card.
    expect(remove.title).toContain("cancel the run first");
    expect(remove.title).toContain("writes the card back when it settles");

    await act(async () => {
      remove.click();
    });
    // No confirm opened, and certainly no request.
    expect(maybeButton("Delete task")).toBeUndefined();
    expect(del).not.toHaveBeenCalled();
  });

  it("still deletes a card nothing is running", async () => {
    const { client, del } = fakeClient();
    const { onDeleted } = await mount(client, { irreversible: [], historyIncomplete: false });

    const remove = button("Delete");
    expect(remove.disabled).toBe(false);
    await act(async () => {
      remove.click();
    });
    await act(async () => {
      button("Delete task").click();
    });
    expect(del).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith("task-1");
  });
});

describe("Cancel on an unsaved edit", () => {
  it("asks before throwing the edit away", async () => {
    const { client } = fakeClient();
    const { onClose } = await mount(client, { irreversible: [], historyIncomplete: false });

    const note = document.querySelector("#task-note") as HTMLTextAreaElement;
    await act(async () => {
      setValue(note, "half an hour of typing");
    });
    await act(async () => {
      button("Cancel").click();
    });

    expect(onClose).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Discard this edit?");

    await act(async () => {
      button("Discard").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("dismisses straight off when nothing was touched", async () => {
    const { client } = fakeClient();
    const { onClose } = await mount(client, { irreversible: [], historyIncomplete: false });

    await act(async () => {
      button("Cancel").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).not.toContain("Discard this edit?");
  });
});

describe("the assignee picker during a save", () => {
  /**
   * Its twin in the detail screen's reassign row is disabled while the write is
   * out. A picker that still moves invites a second choice the in-flight PATCH
   * will not carry, so the row would read as the new assignee while the host
   * stored the old one.
   */
  it("is disabled while the save is in flight", async () => {
    const { client } = fakeClient();
    let release: (t: Task) => void = () => {};
    (client.patch as unknown as ReturnType<typeof vi.fn>).mockImplementation(
      () => new Promise<Task>((resolve) => (release = resolve)),
    );
    await mount(client, { irreversible: [], historyIncomplete: false });

    const picker = () => document.querySelector("#task-assignee") as HTMLButtonElement;
    expect(picker().disabled).toBe(false);

    const note = document.querySelector("#task-note") as HTMLTextAreaElement;
    await act(async () => {
      setValue(note, "something to save");
    });
    await act(async () => {
      button("Save").click();
    });

    expect(picker().disabled).toBe(true);

    await act(async () => {
      release(TASK);
    });
  });
});

/**
 * React installs its own value setter on the element, so assigning `.value`
 * directly is invisible to it. Go through the prototype descriptor and then
 * fire the event React actually listens for.
 */
function setValue(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}
