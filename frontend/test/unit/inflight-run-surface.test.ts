import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { steerActionsFor, supportsSteerAction, type InflightRun } from "@/api/tasks";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

const RUN: InflightRun = {
  taskId: null,
  key: "run_1",
  kind: "delegation",
  title: "t",
  agentId: "a",
  startedAt: 0,
  pendingAction: null,
};

/**
 * What a run supports is a property of the run.
 *
 * The host rejects `pause` and `redirect` on a delegation with `400 "this run
 * only supports cancel"`. That rule lived inline on each surface as a
 * `run.kind === "task"` branch, which is how it came to be spelled once per
 * surface and enforced on none of them from a single place.
 */
describe("steer verbs are derived from the run", () => {
  it("gives a delegation cancel and nothing else", () => {
    expect(steerActionsFor(RUN)).toEqual(["cancel"]);
    expect(supportsSteerAction(RUN, "cancel")).toBe(true);
    expect(supportsSteerAction(RUN, "pause")).toBe(false);
    expect(supportsSteerAction(RUN, "redirect")).toBe(false);
  });

  it("gives a dispatched card all three", () => {
    const card: InflightRun = { ...RUN, kind: "task", taskId: "t-1", key: "t-1" };
    expect([...steerActionsFor(card)].sort()).toEqual(["cancel", "pause", "redirect"]);
  });

  it("does not consult taskId", () => {
    // A delegation that has acquired a card is still cancel-only, and a card
    // run with a null taskId would still be a card run. `kind` is the axis.
    expect(steerActionsFor({ ...RUN, taskId: "t-9" })).toEqual(["cancel"]);
  });
});

/**
 * The surface that carries the control.
 *
 * The retired `#/conversation` route held the only strip that listed runs by
 * key; the Room that replaced it offered no control over a run at all. These
 * pin the replacement to Room, and pin the shell to keeping the whole in-flight
 * read rather than only its card-keyed projection — a delegation has no card,
 * so that projection is precisely where it was being dropped.
 */
describe("the Room carries the in-flight control", () => {
  const chatView = read("views/ChatView.tsx");
  const appShell = read("components/app-shell.tsx");
  const tasksApi = read("api/tasks.ts");

  it("ChatView renders the in-flight bar", () => {
    expect(chatView).toContain('import { InflightRunBar } from "./chat/InflightRunBar";');
    expect(chatView).toContain("<InflightRunBar");
  });

  it("the bar is fed the runs and a refresh from the shell", () => {
    expect(appShell).toContain("inflightRuns={inflightRuns}");
    expect(appShell).toContain("onInflightSteered={refreshTaskStatuses}");
  });

  it("the shell keeps the in-flight rows, not only the card-keyed map", () => {
    expect(appShell).toContain(
      "const [inflightRuns, setInflightRuns] = useState<readonly InflightRun[]>([]);",
    );
    expect(appShell).toContain(
      'if (inflight.status === "fulfilled") setInflightRuns(inflight.value);',
    );
  });

  it("the shell drops in-flight rows when the company changes", () => {
    // A steer key is company-scoped: a row surviving a switch would offer a
    // cancel that addresses the previous company's registry.
    const idx = appShell.indexOf("setTaskStatusByTaskId({});");
    expect(idx).toBeGreaterThan(-1);
    expect(appShell.slice(idx, idx + 400)).toContain("setInflightRuns([]);");
  });

  it("the card-keyed projection still says it cannot carry a cardless run", () => {
    // `taskStatusesById` is allowed to drop them — it is a board decoration —
    // but the contract has to be written down, because reading it as "what is
    // running" is what left a delegation with no surface.
    const idx = tasksApi.indexOf("export function taskStatusesById");
    expect(tasksApi.slice(Math.max(0, idx - 400), idx)).toContain("Card-keyed");
  });

  it("does not reach back into the retired conversation surface", () => {
    expect(chatView).not.toContain("views/conversation/");
    expect(chatView).not.toContain("InflightStrip");
  });
});
