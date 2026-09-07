// @vitest-environment jsdom
//
// The reduced Add-teammate dialog, and the redirect that completes it
// (issue #1989).
//
// # Why this is a component test as well as a pure one
//
// `team-add-surface.test.ts` proves the branch decision for every input. What
// it cannot prove is that the component actually *asks* — a dialog that
// hard-rendered the full form would pass every one of those cases. That is
// exactly the silent failure this redesign is exposed to: the full form looks
// precisely like the dialog did before the change, so nothing reports it.
//
// The redirect needs a component test for a second reason. The reduced dialog
// collects a name and a sentence and nothing else, so the description, the
// persona, the budget and the inbox are all still to be written — on the page
// this redirect opens, beside the copilot that drafts two of them. A create
// that lands nowhere is not a smaller dialog, it is a teammate abandoned
// half-written.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";

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

const api = vi.hoisted(() => ({
  listTasks: vi.fn(),
  fetchBoardColumns: vi.fn(),
  fetchMe: vi.fn(),
  listPeople: vi.fn(),
  setInboxEnabled: vi.fn(),
  getInferenceStatus: vi.fn(),
  designTeammate: vi.fn(),
}));

vi.mock("@/api/tasks", () => ({ listTasks: api.listTasks }));
vi.mock("@/lib/board-columns", () => ({
  fetchBoardColumns: api.fetchBoardColumns,
  IN_FLIGHT_COLUMNS: ["planning", "in_progress"],
}));
vi.mock("@/api/auth", () => ({ me: api.fetchMe, listPeople: api.listPeople }));
vi.mock("@/api/inbox", () => ({ setInboxEnabled: api.setInboxEnabled }));
vi.mock("@/api/inference", () => ({ getInferenceStatus: api.getInferenceStatus }));
// Only the design call is stubbed; `refusalNotice` and `draftNewAgentField` stay
// real, so the hand-over notice these tests read is the operator's own sentence.
vi.mock("@/api/agent-copilot", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/agent-copilot")>()),
  designTeammate: api.designTeammate,
}));

const { TeamView } = await import("@/views/TeamView");

const ROSTER: TeamMemberDto[] = [
  { id: "maya", name: "Maya", role: "Research Lead", description: "Tracks competitors." },
];

let container: HTMLDivElement;
let root: Root;
let added: Array<Record<string, unknown>>;
/**
 * What `addTeamMember` throws, or `null` to let it succeed. The failed-write
 * suite at the end sets it to the transient case the dialog must survive.
 */
let addThrows: unknown | null;
/**
 * What the client reports for `cancelsInFlightRequests`. `false` is the desktop
 * app, where an in-flight Tauri `invoke` cannot be cancelled — see the held-open
 * suite at the end.
 */
let cancelsInFlight: boolean;
/** Held by `addTeamMember` before it answers, so the write can be caught mid-flight. */
let stallAdd: Promise<void> | null;
let opened: Array<[string | null, { edit?: boolean } | undefined]>;

function fakeClient(): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get cancelsInFlightRequests() {
      return cancelsInFlight;
    },
    listTeam: async () => ROSTER,
    addTeamMember: async (input: Record<string, unknown>) => {
      if (stallAdd) await stallAdd;
      added.push(input);
      if (addThrows) throw addThrows;
      return { id: "nova", name: "Nova", role: "Runs paid acquisition" } as TeamMemberDto;
    },
  } as unknown as OpenCompanyClient;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  added = [];
  addThrows = null;
  cancelsInFlight = true;
  stallAdd = null;
  opened = [];
  vi.clearAllMocks();
  api.listTasks.mockResolvedValue([]);
  api.fetchBoardColumns.mockResolvedValue([]);
  api.fetchMe.mockResolvedValue({ id: "u1", role: "admin" });
  api.listPeople.mockResolvedValue([]);
  api.getInferenceStatus.mockResolvedValue({ cognition: "harness" });
  api.designTeammate.mockResolvedValue({
    source: "model",
    role: "Growth Marketer",
    description: "Owns paid acquisition and the weekly ROAS report.",
    instructions: "Report ROAS every Monday. Never raise a budget without sign-off.",
  });
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

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

async function mount() {
  await act(async () => {
    root.render(
      createElement(TeamView, {
        client: fakeClient(),
        company: "acme",
        sub: null,
        onOpenAgent: (agentId: string | null, options?: { edit?: boolean }) => {
          opened.push([agentId, options]);
        },
        refreshKey: 0,
        onRunSetup: vi.fn(),
        onManageDesks: vi.fn(),
        onNavigateToDesk: vi.fn(),
      }),
    );
  });
}

/** Opens the dialog and lets its cognition read land. */
async function openDialog() {
  await act(async () => {
    byText("button", "Add teammate")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await act(async () => {});
}

/** The footer's Add teammate — the dialog is open, so it is the last one. */
async function pressCreate() {
  const buttons = Array.from(document.querySelectorAll<HTMLElement>("button")).filter(
    (el) => el.textContent?.trim() === "Add teammate",
  );
  await act(async () => {
    buttons[buttons.length - 1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

const box = '[data-testid="team-describe-box"]';
const roleField = '[data-testid="agent-field-role"]';

describe("the reduced Add-teammate dialog (issue #1989)", () => {
  it("renders one box and no Role, Instructions, budget or inbox field", async () => {
    await mount();
    await openDialog();

    expect(document.querySelector(box), "the description box must be on screen").not.toBeNull();
    expect(
      document.querySelector('[data-testid="team-describe-name"]'),
      "a name is asked for, because nothing can derive one",
    ).not.toBeNull();

    // The fields the reduction removes. Each is either derived, or waiting on
    // the detail page this create lands on.
    expect(document.querySelector(roleField), "Role is derived, not asked for").toBeNull();
    expect(
      document.querySelector('[data-testid="agent-field-instructions"]'),
      "the persona is drafted by the copilot on the detail page",
    ).toBeNull();
    expect(
      document.querySelector('[data-testid="agent-field-description"]'),
      "the description IS the box",
    ).toBeNull();
    expect(
      document.querySelector('[data-testid="team-add-budget"]'),
      "the budget is set on the detail page",
    ).toBeNull();
    expect(
      byText("span", "Give this teammate an inbox"),
      "the inbox is toggled on the detail page",
    ).toBeUndefined();
  });

  it("writes the teammate the host designed, and lands on its edit form", async () => {
    await mount();
    await openDialog();

    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition, and reports on ROAS weekly.");
    await pressCreate();

    // The sentence reached the host whole. Nothing on this side split it.
    expect(api.designTeammate.mock.calls[0][2]).toEqual({
      name: "Nova",
      description: "Runs paid acquisition, and reports on ROAS weekly.",
    });

    expect(added).toHaveLength(1);
    expect(added[0].name).toBe("Nova");
    // A job title. The clause split that used to run here answered "Runs paid
    // acquisition" for this sentence, dropped the rest of the job on the floor,
    // and stored the fragment as the teammate's permanent role.
    expect(added[0].role).toBe("Growth Marketer");
    expect(added[0].description).toBe("Owns paid acquisition and the weekly ROAS report.");
    // And a persona at birth, rather than an empty field and a promise. This is
    // the assertion that would have caught the state the operator found: a
    // teammate holding one sentence three times over and no instructions.
    expect(added[0].instructions).toBe(
      "Report ROAS every Monday. Never raise a budget without sign-off.",
    );
    expect(added[0].instructions).not.toBe(added[0].description);

    // The redirect is what puts the three designed fields in front of the
    // operator, editable, so a role a model wrote is read before it matters.
    expect(opened).toEqual([["nova", { edit: true }]]);
  });

  it("hands over the full form when the host cannot design the teammate", async () => {
    api.designTeammate.mockResolvedValue({ source: "unavailable", reason: "no_model" });
    await mount();
    await openDialog();

    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");
    await pressCreate();

    // Nothing was written. A teammate is created only from a design that came
    // back whole — never from a fragment of the operator's own sentence, and
    // never with a blank role, which every other write path in the repository
    // refuses and which `POST /team` now refuses too.
    expect(added).toHaveLength(0);
    expect(opened).toHaveLength(0);

    // The full form instead, carrying what was typed, so the operator can name
    // the role themselves rather than meeting a Create that cannot work.
    expect(document.querySelector(roleField), "the full form must be on screen").not.toBeNull();
    expect(document.querySelector(box), "the reduced dialog is retired").toBeNull();
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="agent-field-name"]')!.value,
    ).toBe("Nova");
    expect(
      document.querySelector<HTMLTextAreaElement>('[data-testid="agent-field-description"]')!.value,
    ).toBe("Runs paid acquisition.");
    expect(document.querySelector('[data-testid="team-add-handover"]')).not.toBeNull();
  });

  it("refuses to create until both the name and the box hold something", async () => {
    await mount();
    await openDialog();

    type("team-describe-box", "Runs paid acquisition.");
    await pressCreate();
    expect(added, "a nameless teammate has no id to mint").toHaveLength(0);

    type("team-describe-name", "Nova");
    type("team-describe-box", "");
    await pressCreate();
    expect(added, "an empty box derives no role").toHaveLength(0);
  });
});

describe("closing the Add-teammate dialog (issue #1989)", () => {
  // The hand-over to the full form is meant to last for one open — the module
  // says so in `reset`'s own comment. It did not. `reset` hung off the wrapper
  // passed to Radix's `onOpenChange`, which Radix invokes for Escape and the
  // overlay but which Cancel bypassed by calling the raw `onOpenChange(false)`
  // prop. So one operator who tried a description the dialog could not read a
  // role out of, then pressed Cancel, got the six-field form back on every
  // subsequent add for the life of the page — still carrying the abandoned
  // attempt's name and sentence, still showing the notice explaining a
  // hand-over that had happened minutes ago. Escape, on the identical state,
  // cleared everything. That asymmetry is what these two tests pin.

  async function handOver() {
    api.designTeammate.mockResolvedValue({ source: "unavailable", reason: "no_model" });
    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");
    await pressCreate();
    expect(document.querySelector(roleField), "the hand-over must have happened").not.toBeNull();
  }

  async function pressCancel() {
    await act(async () => {
      byText("button", "Cancel")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await act(async () => {});
  }

  it("Cancel clears the hand-over, so the next add starts reduced again", async () => {
    await handOver();
    await pressCancel();
    await openDialog();

    expect(document.querySelector(box), "the reduced dialog must be back").not.toBeNull();
    expect(document.querySelector(roleField), "the full form must be gone").toBeNull();
    expect(
      document.querySelector('[data-testid="team-add-handover"]'),
      "and the notice about a hand-over that is over",
    ).toBeNull();
  });

  it("Cancel clears what was typed, so nothing leaks into the next add", async () => {
    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");
    await pressCancel();
    await openDialog();

    expect(
      document.querySelector<HTMLInputElement>('[data-testid="team-describe-name"]')!.value,
    ).toBe("");
    expect(
      document.querySelector<HTMLTextAreaElement>('[data-testid="team-describe-box"]')!.value,
    ).toBe("");
    expect(added, "and Cancel writes nothing").toHaveLength(0);
  });
});

describe("the full Add-teammate form on a company that cannot draft", () => {
  beforeEach(() => {
    // The operator's screenshot: "No model is configured, so the copilot can't
    // draft yet." That path keeps today's form, unchanged — hidden, never
    // deleted — so a company with no model is not locked out of writing the
    // fields nothing downstream could draft for it.
    api.getInferenceStatus.mockResolvedValue({ cognition: "echo" });
  });

  it("renders every field the dialog has always had", async () => {
    await mount();
    await openDialog();

    expect(document.querySelector(box), "the reduced dialog must NOT be on screen").toBeNull();
    for (const field of ["name", "role", "description", "instructions"]) {
      expect(
        document.querySelector(`[data-testid="agent-field-${field}"]`),
        `the full form must still render ${field}`,
      ).not.toBeNull();
    }
    expect(document.querySelector('[data-testid="team-add-budget"]')).not.toBeNull();
    expect(byText("span", "Give this teammate an inbox")).toBeDefined();
    // The hand-over note belongs to the reduced dialog's dead end. This form is
    // simply what the dialog IS here, so there is nothing to explain.
    expect(document.querySelector('[data-testid="team-add-handover"]')).toBeNull();
  });

  it("creates from the typed fields and stays on the roster", async () => {
    await mount();
    await openDialog();

    type("agent-field-name", "Nova");
    type("agent-field-role", "Growth Marketer");
    await pressCreate();

    expect(added).toHaveLength(1);
    expect(added[0].role).toBe("Growth Marketer");
    // No redirect on this path: the operator filled the fields in here, so
    // there is nothing waiting for them on the detail page.
    expect(opened).toHaveLength(0);
  });
});

describe("a design the operator walks away from (issue #1989)", () => {
  // `attempt` already made a late answer harmless. It did nothing about the
  // cost: a design pass runs a model for up to ninety seconds and is metered
  // against the company's plan, and the dialog has four ways out — Cancel,
  // Escape, the backdrop and the header's close icon. Three of them reached
  // `close()` while Cancel was disabled, so the control that said what it would
  // do was the only one that would not do it, and every one of them left the
  // host running a pass whose answer is thrown away.

  /** A design that never answers, so the dialog is still waiting when it shuts. */
  function hangingDesign(): { signals: AbortSignal[] } {
    const signals: AbortSignal[] = [];
    api.designTeammate.mockImplementation(
      (
        _client: unknown,
        _company: unknown,
        _teammate: unknown,
        signal?: AbortSignal,
      ) =>
        new Promise((_resolve, reject) => {
          if (signal) {
            signals.push(signal);
            signal.addEventListener("abort", () => reject(new DOMException("", "AbortError")));
          }
        }),
    );
    return { signals };
  }

  async function startDesigning() {
    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");
    await pressCreate();
  }

  it("Cancel is live while a design is running, and aborts it", async () => {
    const { signals } = hangingDesign();
    await startDesigning();

    const cancel = byText("button", "Cancel") as HTMLButtonElement;
    expect(cancel, "Cancel must still be on screen").toBeDefined();
    expect(
      cancel.disabled,
      "a disabled Cancel beside a live Escape is the inconsistency, not the fix",
    ).toBe(false);

    expect(signals).toHaveLength(1);
    expect(signals[0].aborted, "nothing is aborted while the operator is waiting").toBe(false);

    await act(async () => {
      cancel.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await act(async () => {});

    expect(signals[0].aborted, "closing must tear the design down, not just ignore it").toBe(true);
    expect(added, "and nothing is written").toHaveLength(0);
  });

  it("Escape aborts the design too, so no exit spends tokens silently", async () => {
    const { signals } = hangingDesign();
    await startDesigning();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await act(async () => {});

    expect(signals[0].aborted).toBe(true);
    expect(added).toHaveLength(0);
  });

  it("an aborted design never hands over, because its dialog is already gone", async () => {
    // The abort rejects the same promise a transport failure rejects. Without
    // the `attempt` guard in front of it, closing mid-design would reopen onto
    // the full form carrying a hand-over notice for a refusal that never
    // happened.
    const { signals } = hangingDesign();
    await startDesigning();

    await act(async () => {
      (byText("button", "Cancel") as HTMLButtonElement).dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await act(async () => {});
    expect(signals[0].aborted).toBe(true);

    await openDialog();
    expect(document.querySelector(box), "the next add starts reduced again").not.toBeNull();
    expect(document.querySelector('[data-testid="team-add-handover"]')).toBeNull();
  });
});

describe("a cognition read that lands after the operator starts typing", () => {
  // The reduced dialog renders while `/inference` is in flight, because
  // `cognition` is `null` and `addTeammateSurface` deliberately reads that as
  // "can draft". On an `echo` company the answer then arrives and swaps the
  // form. The two shapes hold separate state, so the name and the sentence were
  // simply gone: no error, nothing to retry, and it reads as the console eating
  // the input.

  it("carries the typed name and sentence into the form it swaps to", async () => {
    let settle: (status: { cognition: string }) => void = () => {};
    api.getInferenceStatus.mockReturnValue(
      new Promise((resolve) => {
        settle = resolve;
      }),
    );

    await mount();
    await openDialog();
    expect(
      document.querySelector(box),
      "an unsettled check shows the reduced dialog — that is the documented choice",
    ).not.toBeNull();

    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");

    await act(async () => {
      settle({ cognition: "echo" });
    });
    await act(async () => {});

    expect(document.querySelector(roleField), "the full form must have taken over").not.toBeNull();
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="agent-field-name"]')!.value,
      "the name the operator typed must survive the swap",
    ).toBe("Nova");
    expect(
      document.querySelector<HTMLTextAreaElement>('[data-testid="agent-field-description"]')!.value,
      "and so must the sentence",
    ).toBe("Runs paid acquisition.");
    // Nothing failed, so there is no hand-over to explain.
    expect(document.querySelector('[data-testid="team-add-handover"]')).toBeNull();
  });

  it("does not overwrite the form once the operator has edited it", async () => {
    // Guards the carry against re-running over a later edit: it is a starting
    // point for a form nobody has touched, never a correction to one they have.
    let settle: (status: { cognition: string }) => void = () => {};
    api.getInferenceStatus.mockReturnValue(
      new Promise((resolve) => {
        settle = resolve;
      }),
    );

    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    await act(async () => {
      settle({ cognition: "echo" });
    });
    await act(async () => {});

    type("agent-field-name", "Atlas");
    await act(async () => {});
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="agent-field-name"]')!.value,
    ).toBe("Atlas");
  });
});

describe("a write that does not land (issue #1989)", () => {
  // `onAdd` used to be `void` and called fire-and-forget: the dialog called it
  // and cleared itself on the very next line, while `POST {scope}/team` was
  // still in flight. A 5xx or a dropped connection then left the dialog open,
  // blank and enabled, having thrown away three things at once — the name, the
  // sentence, and a design the company had already been billed a model call
  // for. Pressing Create again bought the same design a second time.

  async function createTeammate() {
    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition, and reports on ROAS weekly.");
    await pressCreate();
  }

  it("keeps the name and the sentence when the create fails", async () => {
    addThrows = new Error("the company host is unreachable");
    await createTeammate();

    expect(added, "the write was attempted").toHaveLength(1);
    expect(opened, "and it did not land, so nobody is redirected").toHaveLength(0);
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="team-describe-name"]')!.value,
      "the name must survive a failed write",
    ).toBe("Nova");
    expect(
      document.querySelector<HTMLTextAreaElement>('[data-testid="team-describe-box"]')!.value,
      "and so must the sentence",
    ).toBe("Runs paid acquisition, and reports on ROAS weekly.");
    // Not a hand-over: the design worked. Saying "no model is configured" over
    // a write that 5xx'd would send the operator after the wrong problem.
    expect(document.querySelector('[data-testid="team-add-handover"]')).toBeNull();
  });

  it("retries without paying for a second design pass", async () => {
    addThrows = new Error("the company host is unreachable");
    await createTeammate();
    expect(api.designTeammate).toHaveBeenCalledTimes(1);

    addThrows = null;
    await pressCreate();

    expect(
      api.designTeammate,
      "the held design still answers this exact sentence, so no second model call",
    ).toHaveBeenCalledTimes(1);
    expect(added).toHaveLength(2);
    expect(added[1].role).toBe("Growth Marketer");
    expect(opened).toEqual([["nova", { edit: true }]]);
  });

  it("designs again once the sentence has been edited", async () => {
    // A design belongs to the sentence it was written from, so an edited box
    // must not be written from the answer to the old one.
    addThrows = new Error("nope");
    await createTeammate();
    expect(api.designTeammate).toHaveBeenCalledTimes(1);

    addThrows = null;
    type("team-describe-box", "Runs wholesale outreach to boutique retailers.");
    await pressCreate();

    expect(api.designTeammate).toHaveBeenCalledTimes(2);
    expect(added).toHaveLength(2);
  });

  it("clears the dialog once the write lands", async () => {
    await createTeammate();
    expect(added).toHaveLength(1);
    expect(opened).toEqual([["nova", { edit: true }]]);

    await openDialog();
    expect(
      document.querySelector<HTMLInputElement>('[data-testid="team-describe-name"]')!.value,
      "a landed write clears the box for the next add",
    ).toBe("");
  });
});

describe("a host that says it cannot design a teammate", () => {
  // The console used to answer this itself, as `cognition !== "echo"`. The host
  // reports the capability now, and this is the path the guess got wrong: a
  // `hosted` company has no profile drafter either, so the reduced dialog could
  // only ever spend a Create on a `no_model` refusal.
  it("renders the full form up front on a non-echo path with no drafter", async () => {
    api.getInferenceStatus.mockResolvedValue({ cognition: "hosted", designsProfiles: false });
    await mount();
    await openDialog();

    expect(document.querySelector(box), "the reduced dialog must NOT be offered").toBeNull();
    expect(document.querySelector(roleField), "the full form is what this company gets").not.toBeNull();
    // Not a hand-over — nothing was attempted and nothing refused.
    expect(document.querySelector('[data-testid="team-add-handover"]')).toBeNull();
    expect(api.designTeammate, "and no design pass is ever asked for").not.toHaveBeenCalled();
  });

  it("still offers it when the host says a design pass can run", async () => {
    api.getInferenceStatus.mockResolvedValue({ cognition: "hosted", designsProfiles: true });
    await mount();
    await openDialog();
    expect(document.querySelector(box)).not.toBeNull();
  });
});

describe("holding the dialog open while leaving would not stop anything", () => {
  // One rule behind four controls: offer the way out only when taking it does
  // something. Two cases where it does not — and the first shipped as a
  // *cancel* that spent the tokens anyway on the desktop app.

  function hangingDesign(): { signals: AbortSignal[] } {
    const signals: AbortSignal[] = [];
    api.designTeammate.mockImplementation(
      (_c: unknown, _co: unknown, _t: unknown, signal?: AbortSignal) =>
        new Promise((_res, rej) => {
          if (signal) {
            signals.push(signal);
            signal.addEventListener("abort", () => rej(new DOMException("", "AbortError")));
          }
        }),
    );
    return { signals };
  }

  async function fillAndCreate() {
    await mount();
    await openDialog();
    type("team-describe-name", "Nova");
    type("team-describe-box", "Runs paid acquisition.");
    await pressCreate();
  }

  it("holds itself open during a design on a transport that cannot cancel", async () => {
    // `ProxyTransport` cannot abort an in-flight Tauri `invoke`, so the pass
    // runs to completion inside the app's core and is metered no matter what
    // the operator does. Offering Cancel there is a gesture that spends the
    // tokens and throws away the answer — the exact behaviour the signal was
    // added to remove, wearing the label of the fix.
    cancelsInFlight = false;
    hangingDesign();
    await fillAndCreate();

    const cancel = byText("button", "Cancel") as HTMLButtonElement;
    expect(cancel.disabled, "Cancel must be held while the design cannot be stopped").toBe(true);
    expect(
      byText("button", "Close"),
      "and the header icon is gone rather than dead — a present control that does nothing reads as broken",
    ).toBeUndefined();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await act(async () => {});
    expect(
      document.querySelector(box),
      "Escape must not shut it either — every exit goes through the same guard",
    ).not.toBeNull();
  });

  it("still offers the way out during a design on a transport that can cancel", async () => {
    cancelsInFlight = true;
    const { signals } = hangingDesign();
    await fillAndCreate();

    const cancel = byText("button", "Cancel") as HTMLButtonElement;
    expect(cancel.disabled).toBe(false);
    expect(byText("button", "Close"), "and the header icon stays").toBeDefined();
    await act(async () => {
      cancel.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await act(async () => {});
    expect(signals[0].aborted).toBe(true);
  });

  it("holds itself open while the write is running, on any transport", async () => {
    // `POST {scope}/team` is not cancellable at all. Closing during it left the
    // create running: on success the parent still navigated to the new
    // teammate's page — pulling the operator somewhere they had just declined
    // to go — and a reopen-and-submit in the gap made a second teammate.
    let release: () => void = () => {};
    api.designTeammate.mockResolvedValue({
      source: "model",
      role: "Growth Marketer",
      description: "Owns paid acquisition.",
      instructions: "Report ROAS every Monday.",
    });
    const gate = new Promise<void>((r) => {
      release = r;
    });
    // Hold the write open, so the dialog is caught saying "Adding…".
    stallAdd = gate;

    await fillAndCreate();

    const cancel = byText("button", "Cancel") as HTMLButtonElement;
    expect(cancel.disabled, "the write cannot be cancelled, so neither can the dialog").toBe(true);
    expect(byText("button", "Close")).toBeUndefined();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await act(async () => {});
    expect(document.querySelector(box), "Escape is refused too").not.toBeNull();

    stallAdd = null;
    await act(async () => {
      release();
      await gate;
    });
    await act(async () => {});
    expect(added, "and the write it was holding for did land").toHaveLength(1);
  });
});
