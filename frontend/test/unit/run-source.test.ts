import { describe, expect, it } from "vitest";

import type { RunSummary } from "@/api/runs";
import type { Task } from "@/api/tasks";
import { runSource, type RunSourceIndex } from "@/lib/run-source";

/**
 * Where a run came from (issue #1573).
 *
 * This is the derivation the teammate's run history is built on, and being
 * wrong here looks entirely normal on screen: an attempt filed under the wrong
 * card title, or a nightly workflow's attempts listed as thirty unexplained
 * cards nobody opened, both render as a perfectly tidy list. Nothing throws.
 * Hence unit tests rather than trusting the eye.
 */

function run(over: Partial<RunSummary> = {}): RunSummary {
  return {
    id: "run-1",
    agentId: "engineer",
    attempt: 1,
    status: "succeeded",
    phase: "terminal",
    createdAtMillis: 1_700_000_000_000,
    usage: { input: 0, output: 0, cachedInput: 0, costUsd: 0 },
    stepCount: 3,
    stepCountCapped: false,
    ...over,
  };
}

function task(over: Partial<Task> = {}): Task {
  return {
    id: "card-7",
    title: "Draft the Q3 memo",
    column: "in_progress",
    priority: "medium",
    assignee: "engineer",
    updatedAt: 1_700_000_000_000,
    ...over,
  };
}

const index = (tasks: Task[], workflows: [string, string][] = []): RunSourceIndex => ({
  tasks: new Map(tasks.map((t) => [t.id, t])),
  workflows: new Map(workflows),
});

describe("a run that worked a card", () => {
  it("is named by the card's title, and links to the card", () => {
    const source = runSource(run({ taskId: "card-7" }), index([task()]));
    expect(source.kind).toBe("card");
    expect(source.label).toBe("Draft the Q3 memo");
    expect(source.href).toBe("#/tasks/card-7");
    expect(source.resolved).toBe(true);
  });

  it("says the card is gone rather than inventing a title for it", () => {
    // The board read may have failed, or the card may genuinely have been
    // deleted. Either way the attempt is a real record and still lists — under
    // its id, marked unresolved so the render sets it as an id and not a name.
    const source = runSource(run({ taskId: "card-gone" }), index([]));
    expect(source.kind).toBe("card");
    expect(source.label).toBe("card-gone");
    expect(source.resolved).toBe(false);
    expect(source.href).toBe("#/tasks/card-gone");
  });
});

describe("a run whose card was opened by a workflow", () => {
  it("is filed under the workflow, with the card on the second line", () => {
    // The ordering that matters: the workflow decided the work and the card is
    // the mechanism. Filing these under the card title would bury the schedule
    // that is actually generating them behind thirty cards nobody wrote.
    const source = runSource(
      run({ taskId: "card-7" }),
      index(
        [task({ originWorkflowId: "wf-nightly", originRunId: "wfrun-9" })],
        [["wf-nightly", "Nightly digest"]],
      ),
    );
    expect(source.kind).toBe("workflow");
    expect(source.label).toBe("Nightly digest");
    expect(source.detail).toBe("Draft the Q3 memo");
    expect(source.resolved).toBe(true);
    // Deep-linked to the run that opened the card, not just to the graph.
    expect(source.href).toBe("#/workflows/wf-nightly?run=wfrun-9");
  });

  it("still says 'workflow' when the workflow list could not be read", () => {
    const source = runSource(
      run({ taskId: "card-7" }),
      index([task({ originWorkflowId: "wf-nightly" })]),
    );
    expect(source.kind).toBe("workflow");
    expect(source.label).toBe("wf-nightly");
    expect(source.resolved).toBe(false);
    // No run id stamped: the link is the graph, with no `?run=` claiming an
    // attempt that was never recorded.
    expect(source.href).toBe("#/workflows/wf-nightly");
  });
});

describe("a run that answered a message", () => {
  it("is named by the channel when the name is known, and by its id when not", () => {
    const named = runSource(run({ chatId: "front-desk" }), {
      chats: new Map([["front-desk", "#front-desk"]]),
    });
    expect(named.kind).toBe("chat");
    expect(named.label).toBe("#front-desk");
    expect(named.href).toBe("#/chat/front-desk");
    expect(named.resolved).toBe(true);

    const bare = runSource(run({ chatId: "front-desk" }));
    expect(bare.label).toBe("front-desk");
    expect(bare.resolved).toBe(false);
  });

  it("links a desk-channel run to the desk, not to a DM of the same name", () => {
    // `agentId === chatId` here is a *desk* coincidence, not a DM: the desk's
    // channel id is its thread id, so a turn in #engineering is addressed to
    // the very desk whose id the thread carries. Stamp `dm:` on that and the
    // link is `#/chat/dm:engineering`, which ChatView cannot resolve as the
    // desk and drops as an unknown channel — the bug this PR fixed.
    const source = runSource(
      run({ chatId: "engineering", agentId: "engineering" }),
      { chats: new Map([["engineering", "#engineering"]]) },
    );
    expect(source.href).toBe("#/chat/engineering");
    expect(source.resolved).toBe(true);
  });

  it("links a DM run to the member's DM, which no desk claims", () => {
    // A DM's thread id is the roster member's id, and no desk's id is also a
    // member's id — so "not in the desk index" is the DM signal, and the link
    // needs the `dm:` prefix ChatView resolves those channel ids by.
    const source = runSource(
      run({ chatId: "ada-1f3k", agentId: "ada-1f3k" }),
      { chats: new Map([["engineering", "#engineering"]]) },
    );
    expect(source.href).toBe("#/chat/dm:ada-1f3k");
    expect(source.resolved).toBe(false);
  });
});

describe("a run with neither handle", () => {
  it("is listed as unattributed rather than hidden", () => {
    // A dispatch that died before it could name a card or a conversation still
    // left an honest record of an attempt. Dropping it would hide exactly the
    // failure it is evidence of.
    const source = runSource(run());
    expect(source.kind).toBe("unknown");
    expect(source.resolved).toBe(false);
    expect(source.href).toBeUndefined();
  });
});

describe("resolution is total", () => {
  it("never throws on an empty index, whichever handle the run carries", () => {
    for (const over of [
      { taskId: "card-7" },
      { chatId: "general" },
      {},
    ] as Partial<RunSummary>[]) {
      expect(() => runSource(run(over))).not.toThrow();
      expect(runSource(run(over)).label.length).toBeGreaterThan(0);
    }
  });
});
