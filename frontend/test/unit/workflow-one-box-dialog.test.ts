// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { WorkflowGraph } from "@/api/workflows";
import { WorkflowCreateDialog } from "@/views/WorkflowCreateDialog";

/**
 * Creating a workflow is one description box and a Create button: no Name, no
 * Workflow ID, no Description, no Nodes, no Connections, and none of the
 * validation that serves them — including "Give the workflow an id.", which
 * used to fire on a dialog the operator had not finished reading.
 *
 * **On every company and every build.** The copilot's availability used to pick
 * between two dialogs; a host with no model configured got the full graph form,
 * which is the dialog this redesign exists to retire. Now it changes only what
 * Create *does* — draft, or fall back to the operator's own sentence — and what
 * the box says above itself. The `echo` cases below are the ones that prove it,
 * and they are the reason this file renders the component rather than trusting
 * `createSurface` (unit-tested next door): the branch has to reach the DOM.
 *
 * The manual form is unchanged and still reachable, by the one route that
 * needs it — a create the host refused, which names an id there is otherwise no
 * control to obey.
 */

const SCOPE = "/api/v1/companies/acme";

/** The controls that must NOT exist on the one-box dialog. */
const NAME_INPUT = 'input[placeholder="e.g. Campaign pipeline"]';
const ID_INPUT = 'input[placeholder="e.g. campaign_pipeline"]';
const DESCRIPTION_BOX = 'textarea[placeholder="What does this workflow do?"]';

/** A drafted graph the host would answer with. */
const DRAFTED: WorkflowGraph = {
  id: "weekly-digest",
  name: "Weekly digest",
  description: "Every Monday, draft the digest and email it.",
  version: null,
  nodes: [
    { id: "start", kind: "trigger", name: "Start", schedule: "0 9 * * 1" },
    { id: "write", kind: "agent", name: "Draft it", agent: "writer" },
  ],
  edges: [{ from: "start", to: "write" }],
};

interface Stub {
  /** What `POST …/workflows/draft-from-description` answers, or throws. */
  draft?: () => Promise<unknown>;
  /** Counts every draft attempt, so "it never asked" can be asserted. */
  drafts?: { count: number };
  /** What `POST …/workflows` answers, or throws. */
  create?: (body: unknown) => Promise<unknown>;
  /** The company's cognition path. `"hosted"` is a company that can draft. */
  cognition?: string;
  /**
   * The workflows the host has **saved**, keyed by id — what a reconcile read
   * of `GET …/workflows/{id}` finds. An id that is not here answers `404`,
   * which is how "the ambiguous write never landed" is expressed.
   */
  saved?: Record<string, WorkflowGraph>;
  /** Counts reconcile reads, so "it asked before writing again" is provable. */
  reads?: { count: number };
}

/** The prefix a single-workflow read sits under. */
const WORKFLOW_PATH = `${SCOPE}/workflows/`;

/**
 * The GETs that share that prefix without naming a workflow — the picker
 * sources the dialog fetches on mount. Counting one of these as a reconcile
 * read would put `reads.count` at 1 before the operator had done anything.
 */
const PICKER_SUBROUTES = new Set(["tool-slugs", "wired-channels"]);

/**
 * Stubs the verbs the dialog reaches. The GETs other than `/inference` and a
 * single-workflow read are optional picker sources that each degrade on
 * failure, so one rejection stands in for "this host offers none of them".
 */
function stubClient(opts: Stub): OpenCompanyClient {
  return {
    scopeFor: () => SCOPE,
    listTeam: () => Promise.reject(new Error("not offered by this host")),
    get: (path: string) => {
      if (path.endsWith("/inference")) {
        return Promise.resolve({ cognition: opts.cognition ?? "hosted" });
      }
      // `${SCOPE}/workflows/{id}` only — the trailing slash keeps the LIST read
      // (`${SCOPE}/workflows`) out, and `PICKER_SUBROUTES` the two named GETs
      // that share the prefix. Everything else degrades below.
      if (path.startsWith(WORKFLOW_PATH)) {
        const wid = decodeURIComponent(path.slice(WORKFLOW_PATH.length));
        if (!PICKER_SUBROUTES.has(wid)) {
          if (opts.reads) opts.reads.count += 1;
          const found = opts.saved?.[wid];
          return found
            ? Promise.resolve(found)
            : Promise.reject(new ApiError(404, "not_found", `no workflow \`${wid}\``, true));
        }
      }
      return Promise.reject(new Error("not offered by this host"));
    },
    post: (path: string, body?: unknown) => {
      if (path.endsWith("/workflows/draft-from-description")) {
        if (opts.drafts) opts.drafts.count += 1;
        return (
          opts.draft?.() ??
          Promise.resolve({ automatable: true, summary: "a digest", workflow: DRAFTED })
        );
      }
      if (path.endsWith("/workflows/validate")) return Promise.resolve({ valid: true });
      if (path.endsWith("/workflows")) {
        return opts.create?.(body) ?? Promise.resolve(body as WorkflowGraph);
      }
      return Promise.reject(new Error(`unexpected POST ${path}`));
    },
    put: () => Promise.reject(new Error("no put expected")),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let onOpenChange: ReturnType<typeof vi.fn>;
let onCreated: ReturnType<typeof vi.fn>;

function inDialog<T extends Element>(selector: string): T | null {
  return document.querySelector<T>(`[data-slot="dialog-content"] ${selector}`);
}

function submitButton(): HTMLButtonElement {
  const el = inDialog<HTMLButtonElement>('[data-testid="workflow-dialog-submit"]');
  if (!el) throw new Error(`no submit button in:\n${document.body.innerHTML}`);
  return el;
}

function describeBox(): HTMLTextAreaElement | null {
  return inDialog<HTMLTextAreaElement>('[data-testid="workflow-describe-box"]');
}

/**
 * A `workflow_invalid` refusal: a 400 that carries per-node complaints.
 *
 * `problems` is a field rather than a constructor argument, so it is assigned
 * here rather than passed — the fourth positional argument is `fromHost`, and a
 * refusal built by passing the array there is one that silently has none.
 */
function perNodeRefusal(): ApiError {
  const err = new ApiError(400, "workflow_invalid", "the graph was refused", true);
  err.problems = [
    { node_id: "write", field: "config.agent", message: "no such teammate" },
  ];
  return err;
}

/**
 * The id the confirm is about to make permanent (issue #1808).
 *
 * Read from the document rather than through `inDialog`: the confirm is
 * portalled onto `document.body`, and a dialog-scoped lookup misses it — which
 * reads as "the confirm did not open" rather than "it opened elsewhere".
 */
function confirmedId(): string {
  return (
    document.querySelector('[data-testid="workflow-id-confirm-value"]')?.textContent ?? ""
  );
}

/** Presses the confirm's own Create, which is what actually writes. */
async function confirmCreate() {
  const btn = document.querySelector<HTMLButtonElement>(
    '[data-testid="workflow-id-confirm-create"]',
  );
  expect(btn, `no id confirm on screen in:\n${document.body.innerHTML}`).toBeTruthy();
  await act(async () => {
    btn!.click();
  });
}

/** Sets a controlled textarea the way a keystroke would. */
function typeDescription(value: string) {
  const box = describeBox();
  expect(box, "the one-box dialog should have a description box").toBeTruthy();
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )!.set!;
  setter.call(box, value);
  box!.dispatchEvent(new Event("input", { bubbles: true }));
}

/** The visible label text of every section heading and control label. */
function dialogText(): string {
  return document.querySelector('[data-slot="dialog-content"]')?.textContent ?? "";
}

/**
 * Renders the dialog with `open` as given.
 *
 * Separate from {@link open} because closing and reopening is the only way to
 * bump the draft epoch from outside, and that is exactly what the late-rejection
 * case below needs to reproduce.
 */
async function setOpen(client: OpenCompanyClient, isOpen: boolean) {
  await act(async () => {
    root.render(
      createElement(WorkflowCreateDialog, {
        open: isOpen,
        onOpenChange,
        onCreated,
        client,
        company: "acme",
      }),
    );
  });
}

async function open(client: OpenCompanyClient) {
  await setOpen(client, true);
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  Element.prototype.scrollIntoView = vi.fn();
  onOpenChange = vi.fn();
  onCreated = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the New-workflow dialog when the copilot can draft", () => {
  it("is one description box — no Name, ID, Description, Nodes or Connections", async () => {
    await open(stubClient({ cognition: "hosted" }));

    expect(describeBox(), "the description box is the whole dialog").toBeTruthy();
    expect(inDialog(NAME_INPUT), "Name must not render").toBeNull();
    expect(inDialog(ID_INPUT), "Workflow ID must not render").toBeNull();
    expect(inDialog(DESCRIPTION_BOX), "the second Description box must not render").toBeNull();
    // The section headings, not just their controls: a heading with no rows
    // under it is the same clutter the redesign removes.
    expect(dialogText()).not.toContain("Nodes");
    expect(dialogText()).not.toContain("Connections");
    expect(dialogText()).not.toContain("Add node");
    expect(dialogText()).not.toContain("Add edge");
    // …and the separate "Draft it" button is gone with the two-step it served.
    expect(inDialog('[data-testid="workflow-copilot-draft"]')).toBeNull();
  });

  it("never raises the id complaint on a dialog that asks for no id", async () => {
    await open(stubClient({ cognition: "hosted" }));

    // Create with an empty box does nothing at all — the button is dead rather
    // than answering with a rule about a field nobody was shown.
    expect(submitButton().disabled, "Create is dead with an empty box").toBe(true);
    await act(async () => {
      submitButton().click();
    });
    expect(dialogText()).not.toContain("Give the workflow an id.");
    expect(inDialog('[data-testid="create-error"]')).toBeNull();
  });

  it("drafts, saves, and hands the canvas the graph and the host's notes", async () => {
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: true,
            summary: "a weekly digest",
            workflow: DRAFTED,
            notes: ["Matched “the writer” to teammate `writer`.", "   "],
          }),
        create: (body) => {
          posted.push(body);
          return Promise.resolve({ ...(body as WorkflowGraph), version: "v1" });
        },
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    // One write, of the host's own drafted graph — not a round trip through
    // form state the dialog is no longer rendering.
    expect(posted).toHaveLength(1);
    expect((posted[0] as WorkflowGraph).id).toBe("weekly-digest");
    expect((posted[0] as WorkflowGraph).nodes).toHaveLength(2);
    // The canvas is where review happens now, so it is handed both the saved
    // graph and the corrections the host made on the way to it — blank notes
    // dropped, because an empty bullet is not a correction.
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onCreated.mock.calls[0]![0].version).toBe("v1");
    expect(onCreated.mock.calls[0]![1]).toEqual([
      "Matched “the writer” to teammate `writer`.",
    ]);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("shows a decline as advice, with a way past it", async () => {
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: false,
            reason: "This is a one-off — just do it once rather than building it.",
          }),
        create: (body) => {
          posted.push(body);
          return Promise.resolve(body as WorkflowGraph);
        },
      }),
    );

    await act(async () => {
      typeDescription("Email Priya the Q3 numbers, once.");
    });
    await act(async () => {
      submitButton().click();
    });

    // The reason is shown, and nothing was written.
    const declined = inDialog('[data-testid="workflow-draft-declined"]');
    expect(declined, "a decline must be shown, not swallowed").toBeTruthy();
    expect(declined!.textContent).toContain("This is a one-off");
    expect(posted, "a decline writes nothing on its own").toHaveLength(0);

    // …and the operator who disagrees is not blocked. The reason is advice.
    const anyway = inDialog<HTMLButtonElement>('[data-testid="workflow-create-anyway"]');
    expect(anyway, "a decline must offer a way past it").toBeTruthy();
    await act(async () => {
      anyway!.click();
    });
    // Issue #1808: the console derives this id from a clause the operator wrote
    // as prose, so it is said out loud before it is permanent.
    expect(confirmedId(), "a console-derived id is confirmed, not minted in silence").toBe(
      "email-priya-the-q3-numbers",
    );
    expect(posted, "nothing is written until the id is confirmed").toHaveLength(0);
    await confirmCreate();
    expect(posted, "Create it anyway must actually create").toHaveLength(1);
    const graph = posted[0] as WorkflowGraph;
    // Named and described from the operator's own sentence, with the same
    // single trigger the blank form has always started from.
    expect(graph.name).toBe("Email Priya the Q3 numbers");
    expect(graph.id).toBe("email-priya-the-q3-numbers");
    expect(graph.description).toBe("Email Priya the Q3 numbers, once.");
    expect(graph.nodes.map((n) => n.kind)).toEqual(["trigger"]);
    expect(onCreated).toHaveBeenCalledTimes(1);
  });

  it("hands over the fields when the host refuses the minted id", async () => {
    // The refusal that actually happens: the host mints ids by deduping against
    // SAVED workflows only, so two similar descriptions drafted before either
    // is created mint the same id — and the second Create is told to pick a
    // different one, by a dialog with no id field.
    await open(
      stubClient({
        cognition: "hosted",
        create: () =>
          Promise.reject(
            new ApiError(
              409,
              "conflict",
              "A workflow with id `weekly-digest` already exists. Pick a different id.",
              // The host's own envelope. A 409 the client synthesised from a
              // proxy's status line is not an instruction and must not hand
              // over the form — see the `writeRefusalHandsOverForm` unit tests.
              true,
            ),
          ),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    // The fields come back, carrying the graph that was refused, so the
    // instruction in the message is one the operator can actually follow.
    expect(inDialog(ID_INPUT), "the id field must come back").toBeTruthy();
    expect(inDialog<HTMLInputElement>(ID_INPUT)!.value).toBe("weekly-digest");
    expect(inDialog<HTMLInputElement>(NAME_INPUT)!.value).toBe("Weekly digest");
    expect(dialogText()).toContain("Nodes");
    expect(inDialog('[data-testid="create-error"]')!.textContent).toContain(
      "Pick a different id",
    );
    expect(onCreated, "a refused write creates nothing").not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it("keeps the box when the build turns out to have no copilot, and still creates", async () => {
    // A capability gap is a fact about the deployment, not about the sentence,
    // so it retires DRAFTING — not the dialog. The fields do not come back;
    // the notice above the box changes to the host's own words and Create
    // builds the workflow from what the operator wrote.
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.reject(new ApiError(404, "not_wired", "This build has no copilot wired.")),
        create: (body) => {
          posted.push(body);
          return Promise.resolve(body as WorkflowGraph);
        },
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest.");
    });
    await act(async () => {
      submitButton().click();
    });

    // The one box is still the whole dialog — this is the reversal.
    expect(describeBox(), "the box must survive a capability gap").toBeTruthy();
    expect(inDialog(NAME_INPUT), "the manual form must NOT come back").toBeNull();
    expect(inDialog(ID_INPUT)).toBeNull();
    expect(dialogText()).not.toContain("Nodes");
    // …saying what happened, in the host's own words, and what Create does now.
    const notice = inDialog('[data-testid="workflow-draft-unavailable"]');
    expect(notice, "the operator must be told the copilot could not draft").toBeTruthy();
    expect(notice!.textContent).toContain("This build has no copilot wired.");
    expect(notice!.textContent).toContain("empty canvas");
    expect(posted, "the failed draft writes nothing on its own").toHaveLength(0);

    // …and Create is not a dead button. The second press builds the workflow
    // from the sentence and lands on the canvas.
    await act(async () => {
      submitButton().click();
    });
    await confirmCreate();
    expect(posted, "Create must still create with no copilot").toHaveLength(1);
    const graph = posted[0] as WorkflowGraph;
    expect(graph.name).toBe("Every Monday");
    expect(graph.description).toBe("Every Monday, draft the digest.");
    expect(graph.nodes.map((n) => n.kind)).toEqual(["trigger"]);
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe("the New-workflow dialog on a company with no model configured", () => {
  it("is the same one box — the graph form does not come back for `echo`", async () => {
    await open(stubClient({ cognition: "echo" }));

    // The reversal, asserted where the operator sees it. Every field the
    // one-box dialog removes stays removed on the offline brain, because a
    // company with no model is the LAST one to hand a graph editor to.
    expect(describeBox(), "the description box is the whole dialog").toBeTruthy();
    expect(inDialog(NAME_INPUT), "Name must not render").toBeNull();
    expect(inDialog(ID_INPUT), "Workflow ID must not render").toBeNull();
    expect(inDialog(DESCRIPTION_BOX), "the second Description box must not render").toBeNull();
    expect(dialogText()).not.toContain("Nodes");
    expect(dialogText()).not.toContain("Connections");
    expect(dialogText()).not.toContain("Add node");
    expect(dialogText()).not.toContain("Add edge");
    expect(inDialog('[data-testid="workflow-copilot-draft"]')).toBeNull();
  });

  it("says what Create will do before it is pressed, and where to fix it", async () => {
    await open(stubClient({ cognition: "echo" }));

    const notice = inDialog('[data-testid="workflow-draft-unavailable"]');
    expect(notice, "an operator must not be promised a draft that cannot happen").toBeTruthy();
    expect(notice!.textContent).toContain("no model configured");
    expect(notice!.textContent).toContain("Settings → Inference");
    expect(notice!.textContent).toContain("empty canvas");
    // NOT the copy this path used to carry, which pointed at a form that is no
    // longer under it.
    expect(dialogText()).not.toContain("build the graph by hand below");
  });

  it("creates from the sentence and lands on the canvas, without a doomed draft", async () => {
    const drafts = { count: 0 };
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "echo",
        drafts,
        create: (body) => {
          posted.push(body);
          return Promise.resolve({ ...(body as WorkflowGraph), version: "v1" });
        },
      }),
    );

    await act(async () => {
      typeDescription("Chase overdue invoices every Friday.");
    });
    await act(async () => {
      submitButton().click();
    });

    // No round trip that is already known to fail — the cognition read has
    // settled on the offline brain, so there is nothing to ask.
    expect(drafts.count, "the copilot must not be asked on the offline brain").toBe(0);
    // Issue #1808 again, and this is the case it was written about: a company
    // with no copilot slugs the operator's first clause into a permanent backend
    // join key, and the one-box dialog has no field that would ever show it.
    expect(confirmedId()).toBe("chase-overdue-invoices-every-friday");
    expect(posted, "nothing is written before the id is confirmed").toHaveLength(0);
    await confirmCreate();
    // …and the same route "Create it anyway" takes: the sentence names it and
    // describes it, over the single trigger the blank form has always started
    // from.
    expect(posted, "Create must never be a button that cannot create").toHaveLength(1);
    const graph = posted[0] as WorkflowGraph;
    expect(graph.name).toBe("Chase overdue invoices every Friday");
    expect(graph.id).toBe("chase-overdue-invoices-every-friday");
    expect(graph.description).toBe("Chase overdue invoices every Friday.");
    expect(graph.nodes.map((n) => n.kind)).toEqual(["trigger"]);
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onCreated.mock.calls[0]![0].version).toBe("v1");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("hands over the fields when the sentence names nothing, rather than minting an empty id", async () => {
    // The id is the permanent join key and it is derived from the name, so a
    // sentence with no words in it has to be asked about — never guessed.
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "echo",
        create: (body) => {
          posted.push(body);
          return Promise.resolve(body as WorkflowGraph);
        },
      }),
    );

    await act(async () => {
      typeDescription("...");
    });
    await act(async () => {
      submitButton().click();
    });

    expect(posted, "an empty name must not be written").toHaveLength(0);
    expect(inDialog(NAME_INPUT), "the fields come back to be filled in").toBeTruthy();
    expect(inDialog<HTMLInputElement>(NAME_INPUT)!.value).toBe("");
    expect(inDialog<HTMLInputElement>(ID_INPUT)!.value).toBe("");
    expect(inDialog('[data-testid="create-error"]')!.textContent).toContain(
      "Give this workflow a name",
    );
  });
});

describe("the New-workflow dialog when the write itself fails", () => {
  /**
   * The hand-over is a **one-way door**: it retires the box for the rest of the
   * open. So it has to fire on a refusal the operator can act on, and only on
   * one — a dropped connection or a 500 that collapsed the dialog would leave
   * them hand-authoring a graph on a host that would have written theirs a
   * second later, which is the exact outcome this redesign exists to prevent.
   */
  it("keeps the box when the write fails for a reason nobody can act on", async () => {
    await open(
      stubClient({
        cognition: "hosted",
        create: () => Promise.reject(new ApiError(500, "internal", "the host fell over")),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    // The box is still the whole dialog, and the sentence is still in it.
    expect(describeBox(), "a 500 must not retire the one-box dialog").toBeTruthy();
    expect(describeBox()!.value).toBe("Every Monday, draft the digest and email it.");
    expect(inDialog(NAME_INPUT), "the manual form must NOT come back").toBeNull();
    expect(inDialog(ID_INPUT)).toBeNull();
    expect(dialogText()).not.toContain("Nodes");
    // …and the failure is still reported, so Create never reads as dead.
    expect(inDialog('[data-testid="create-error"]')!.textContent).toContain(
      "the host fell over",
    );
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it("keeps the box when the write never reached the host at all", async () => {
    // A `TypeError` is what `fetch` throws on a dropped connection. It is not an
    // `ApiError`, so there is nothing the host asked for and nothing to obey.
    await open(
      stubClient({
        cognition: "hosted",
        create: () => Promise.reject(new TypeError("Failed to fetch")),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    expect(describeBox(), "a dropped connection must not retire the dialog").toBeTruthy();
    expect(inDialog(ID_INPUT)).toBeNull();
    expect(inDialog('[data-testid="create-error"]')!.textContent).toContain(
      "Failed to fetch",
    );
  });

  it("hands over the fields for a per-node refusal, which names controls", async () => {
    // The other half of the gate: `workflow_invalid` carries per-node problems,
    // and each one wants a control to land on. Those the box does not have.
    await open(
      stubClient({
        cognition: "hosted",
        create: () => Promise.reject(perNodeRefusal()),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    expect(inDialog(ID_INPUT), "a per-node refusal must show the nodes").toBeTruthy();
    expect(dialogText()).toContain("Nodes");
  });
});

describe("the New-workflow dialog when the copilot declines", () => {
  /**
   * `automatable: false` covers two different events, and only one of them is
   * advice. A failed draft used to arrive as advice **verbatim**, which meant
   * the operator was shown the gate diagnostics — `trigger` nodes, node ids —
   * by the one dialog built to stop mentioning them, and offered "Create it
   * anyway" as if there were an opinion to overrule.
   */
  it("does not dress a failed draft as advice, or quote the gates at the operator", async () => {
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: false,
            reason:
              "the described workflow could not be drafted into one that would be accepted: " +
              "invalid request: a workflow needs exactly one `trigger` node to say what " +
              "starts it (found 0).",
          }),
      }),
    );

    await act(async () => {
      typeDescription("Every Friday, email the sales digest and file it in Dropbox.");
    });
    await act(async () => {
      submitButton().click();
    });

    const declined = inDialog('[data-testid="workflow-draft-declined"]');
    expect(declined, "a failed draft must still be reported").toBeTruthy();
    expect(declined!.getAttribute("data-decline-kind")).toBe("failure");
    // The vocabulary the one box exists to retire never reaches the operator.
    expect(declined!.textContent).not.toContain("trigger");
    expect(declined!.textContent).not.toContain("invalid request");
    // What is said instead is true, and says what to do next.
    expect(declined!.textContent).toContain("could not turn that into a workflow");
    expect(declined!.textContent).toContain("start it on the canvas");
    // And the action offered is the canvas, not the overruling of an opinion.
    const action = inDialog<HTMLButtonElement>('[data-testid="workflow-create-anyway"]');
    expect(action!.textContent).toContain("Start it on the canvas");
    expect(action!.textContent).not.toContain("anyway");
  });

  it("still shows a real judgement in the copilot's own words", async () => {
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: false,
            reason: "This is a one-off — just do it once rather than building it.",
          }),
      }),
    );

    await act(async () => {
      typeDescription("Email Priya the Q3 numbers, once.");
    });
    await act(async () => {
      submitButton().click();
    });

    const declined = inDialog('[data-testid="workflow-draft-declined"]');
    expect(declined!.getAttribute("data-decline-kind")).toBe("judgment");
    expect(declined!.textContent).toContain("This is a one-off");
    expect(
      inDialog<HTMLButtonElement>('[data-testid="workflow-create-anyway"]')!.textContent,
    ).toContain("Create it anyway");
  });
});

/**
 * **A write that fails ambiguously must not become a second workflow.**
 *
 * The one box was hardened so a `500` or a dropped connection keeps the box up
 * with the sentence in it, rather than collapsing into the graph form. That is
 * right, and it opened a worse hole one press later: the commit can land and
 * only the *response* be lost, and the next Create used to draft from scratch.
 * The host mints a draft's id by deduping against the workflows it has SAVED
 * (`safe_workflow_id`, `src/harness/built_in/workflow_build/tools.rs`), so the
 * second draft of the same sentence sees the first one already stored and mints
 * `weekly-digest-2` — a permanent duplicate, plus a second billed model call,
 * from an operator who pressed the same button twice on the same sentence.
 */
describe("the New-workflow dialog after a write that may have landed", () => {
  /** The graph the host would answer a reconcile read with. */
  const SAVED: WorkflowGraph = { ...DRAFTED, version: "v1" };

  /** Fails the first `POST …/workflows` and lets every later one through. */
  function failFirstCreate(posted: unknown[]) {
    return (body: unknown) => {
      posted.push(body);
      return posted.length === 1
        ? Promise.reject(new ApiError(500, "internal", "the host fell over", true))
        : Promise.resolve({ ...(body as WorkflowGraph), version: "v1" });
    };
  }

  it("reads the id back and lands on the workflow that already exists", async () => {
    const posted: unknown[] = [];
    const drafts = { count: 0 };
    const reads = { count: 0 };
    await open(
      stubClient({
        cognition: "hosted",
        drafts,
        reads,
        // The write committed; only the answer was lost. So the host HAS it.
        saved: { "weekly-digest": SAVED },
        draft: () =>
          Promise.resolve({
            automatable: true,
            summary: "a digest",
            workflow: DRAFTED,
            notes: ["Matched “the writer” to teammate `writer`."],
          }),
        create: failFirstCreate(posted),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(posted, "the first Create writes").toHaveLength(1);
    expect(onCreated, "…and is told it failed").not.toHaveBeenCalled();
    expect(describeBox(), "a 500 keeps the box").toBeTruthy();

    // The operator presses Create again, on the same sentence.
    await act(async () => {
      submitButton().click();
    });

    // No second model call: the graph from the first press is still the answer
    // to this sentence, and asking again is what mints the duplicate id.
    expect(drafts.count, "the copilot must not be asked twice").toBe(1);
    // The id was read back before anything was written…
    expect(reads.count, "the retry must ask whether the write landed").toBe(1);
    // …and, finding it, nothing was written at all.
    expect(posted, "a committed write must not be written a second time").toHaveLength(1);
    // The operator lands on the workflow they already have, with the host's
    // corrections still attached — not on a duplicate beside it.
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onCreated.mock.calls[0]![0].id).toBe("weekly-digest");
    expect(onCreated.mock.calls[0]![0].version).toBe("v1");
    expect(onCreated.mock.calls[0]![1]).toEqual([
      "Matched “the writer” to teammate `writer`.",
    ]);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("writes the same graph again when the failed write never landed", async () => {
    const posted: unknown[] = [];
    const drafts = { count: 0 };
    const reads = { count: 0 };
    await open(
      stubClient({
        cognition: "hosted",
        drafts,
        reads,
        // Nothing saved: the 500 was a genuine failure, so the retry must write.
        saved: {},
        create: failFirstCreate(posted),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });
    await act(async () => {
      submitButton().click();
    });

    expect(drafts.count, "the copilot must not be asked twice").toBe(1);
    expect(reads.count).toBe(1);
    // The SAME graph, under the SAME id — not a fresh draft that would have
    // been deduped into `weekly-digest-2` had the first write in fact landed.
    expect(posted).toHaveLength(2);
    expect((posted[0] as WorkflowGraph).id).toBe("weekly-digest");
    expect((posted[1] as WorkflowGraph).id).toBe("weekly-digest");
    expect(posted[1]).toEqual(posted[0]);
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("drafts afresh once the operator rewords the box", async () => {
    // The held graph is an answer to a SENTENCE. Reword it and the operator is
    // asking about something else, so the copilot is asked about it — the held
    // graph must not be written for a description nobody typed.
    const posted: unknown[] = [];
    const drafts = { count: 0 };
    await open(
      stubClient({ cognition: "hosted", drafts, saved: {}, create: failFirstCreate(posted) }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });
    await act(async () => {
      typeDescription("Every Friday, chase the overdue invoices.");
    });
    await act(async () => {
      submitButton().click();
    });

    expect(drafts.count, "a new sentence earns a new draft").toBe(2);
  });

  it("does not hold a graph the host actually refused", async () => {
    // A `409` is not ambiguous — the host considered it and said no — and it
    // hands over the form. Holding the graph as well would mean the form's
    // Create raced a retry of the very write that was refused.
    const drafts = { count: 0 };
    const reads = { count: 0 };
    await open(
      stubClient({
        cognition: "hosted",
        drafts,
        reads,
        create: () =>
          Promise.reject(
            new ApiError(409, "conflict", "A workflow with id `x` already exists.", true),
          ),
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    expect(inDialog(ID_INPUT), "a 409 hands over the form").toBeTruthy();
    expect(reads.count, "a refusal is not a question about whether it landed").toBe(0);
  });
});

/**
 * **A draft that rejects after the dialog moved on belongs to nobody.**
 *
 * The success path has checked the epoch since issue #1052; the failure path did
 * not, and its failure is worse than a stale banner. A capability gap latches
 * `draftGap`, which retires drafting for the whole open — so a rejection landing
 * on a REOPENED dialog silently sent the next Create down the `createAnyway()`
 * fallback, building an empty canvas from a sentence the copilot was never asked
 * about, on a host that could draft perfectly well.
 */
describe("the New-workflow dialog when a draft rejects late", () => {
  it("does not retire drafting on the dialog that replaced it", async () => {
    let rejectDraft: (e: unknown) => void = () => {};
    const drafts = { count: 0 };
    const client = stubClient({
      cognition: "hosted",
      drafts,
      draft: () =>
        new Promise((_resolve, reject) => {
          rejectDraft = reject;
        }),
    });

    await open(client);
    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(drafts.count).toBe(1);

    // The operator gives up on it and opens the dialog again.
    await setOpen(client, false);
    await setOpen(client, true);

    // …and only now does the abandoned request answer, with the one code that
    // would otherwise retire drafting for good.
    await act(async () => {
      rejectDraft(
        new ApiError(409, "inference_required", "no model is configured", true),
      );
      await Promise.resolve();
    });

    expect(
      inDialog('[data-testid="workflow-draft-unavailable"]'),
      "a dead request must not tell the new dialog the copilot is gone",
    ).toBeNull();
    // Proof it is not merely invisible: the next Create still asks the copilot,
    // rather than falling through to the sentence-only fallback.
    await act(async () => {
      typeDescription("Every Friday, chase the overdue invoices.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(drafts.count, "the new dialog can still draft").toBe(2);
  });
});

/**
 * **A decline is an argument about one sentence.**
 *
 * "Create it anyway" is a bypass, and what authorises it is that the copilot
 * argued against *this* description. The banner did not clear when the box did,
 * so after rewording A into B it still sat there offering the bypass — and
 * `createAnyway()` reads the CURRENT box. One click created B unexamined, on a
 * justification that was only ever about A.
 */
describe("the New-workflow dialog after the sentence changes", () => {
  it("clears a decline the new sentence never earned", async () => {
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: false,
            reason: "This is a one-off — just do it once rather than building it.",
          }),
      }),
    );

    await act(async () => {
      typeDescription("Email Priya the Q3 numbers, once.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(inDialog('[data-testid="workflow-draft-declined"]')).toBeTruthy();

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });

    expect(
      inDialog('[data-testid="workflow-draft-declined"]'),
      "the decline was about the sentence that is no longer in the box",
    ).toBeNull();
    expect(
      inDialog('[data-testid="workflow-create-anyway"]'),
      "and the bypass goes with the argument that authorised it",
    ).toBeNull();
  });

  it("clears a draft error the new sentence never earned", async () => {
    await open(
      stubClient({
        cognition: "hosted",
        draft: () => Promise.reject(new ApiError(500, "internal", "the copilot fell over", true)),
      }),
    );

    await act(async () => {
      typeDescription("Email Priya the Q3 numbers, once.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(dialogText()).toContain("the copilot fell over");

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    expect(dialogText()).not.toContain("the copilot fell over");
  });

  it("keeps a capability gap, which is not about the sentence at all", async () => {
    // The complement, and the reason this is a per-banner rule rather than
    // "clear everything": no rewording wires a model into the build.
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.reject(
            new ApiError(404, "not_wired", "no copilot is wired into this build", true),
          ),
      }),
    );

    await act(async () => {
      typeDescription("Email Priya the Q3 numbers, once.");
    });
    await act(async () => {
      submitButton().click();
    });
    expect(inDialog('[data-testid="workflow-draft-unavailable"]')).toBeTruthy();

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    expect(
      inDialog('[data-testid="workflow-draft-unavailable"]'),
      "a build with no copilot still has no copilot",
    ).toBeTruthy();
  });
});

/**
 * **The host's corrections have to survive the hand-over.**
 *
 * A drafted graph can carry `notes` (issue #813) — "matched “the writer” to
 * teammate `writer`" — and the canvas is where they are read. The drafted path
 * passed them to `onCreated`; the REFUSAL path hydrated the form and dropped
 * them, and the form's own Create then called `onCreated` with nothing. So on
 * the one route where the operator has least context — a refusal, a form they
 * did not ask for — the saved graph had corrections nobody was ever shown.
 */
describe("the New-workflow dialog's corrections across a refusal", () => {
  const NOTE = "Matched “the writer” to teammate `writer`.";

  it("shows them on the handed-over form and carries them to the canvas", async () => {
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "hosted",
        draft: () =>
          Promise.resolve({
            automatable: true,
            summary: "a digest",
            workflow: DRAFTED,
            notes: [NOTE],
          }),
        create: (body) => {
          posted.push(body);
          return posted.length === 1
            ? Promise.reject(
                new ApiError(
                  409,
                  "conflict",
                  "A workflow with id `weekly-digest` already exists. Pick a different id.",
                  true,
                ),
              )
            : Promise.resolve({ ...(body as WorkflowGraph), version: "v1" });
        },
      }),
    );

    await act(async () => {
      typeDescription("Every Monday, draft the digest and email it.");
    });
    await act(async () => {
      submitButton().click();
    });

    // The form came back, and the corrections came with it — the operator is
    // being asked to fix an id on a graph that was rewritten under them.
    expect(inDialog(ID_INPUT), "the refusal hands over the form").toBeTruthy();
    const notes = inDialog('[data-testid="workflow-copilot-notes"]');
    expect(notes, "the host's corrections must survive the hand-over").toBeTruthy();
    expect(notes!.textContent).toContain("Matched");

    // Pick a different id, as the host asked, and create.
    const idInput = inDialog<HTMLInputElement>(ID_INPUT)!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    await act(async () => {
      setter.call(idInput, "weekly-digest-monday");
      idInput.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      submitButton().click();
    });
    await confirmCreate();

    expect(posted).toHaveLength(2);
    expect((posted[1] as WorkflowGraph).id).toBe("weekly-digest-monday");
    // …and the canvas is told what the copilot changed, which is the whole
    // point of a note: the saved graph does not say “the writer” anywhere.
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onCreated.mock.calls[0]![1]).toEqual([NOTE]);
  });

  it("says nothing on a graph the operator wrote themselves", async () => {
    // The complement: a hand-authored create has no corrections, so `onCreated`
    // must not be handed a stale list from somewhere else.
    const posted: unknown[] = [];
    await open(
      stubClient({
        cognition: "echo",
        create: (body) => {
          posted.push(body);
          return Promise.resolve({ ...(body as WorkflowGraph), version: "v1" });
        },
      }),
    );

    await act(async () => {
      typeDescription("Chase the overdue invoices every Friday.");
    });
    await act(async () => {
      submitButton().click();
    });
    await confirmCreate();

    expect(posted).toHaveLength(1);
    expect(onCreated).toHaveBeenCalledTimes(1);
    expect(onCreated.mock.calls[0]![1]).toEqual([]);
  });
});
