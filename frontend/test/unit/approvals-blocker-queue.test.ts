// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ApprovalSummary, ChatResponse, Verdict } from "@/api/types";
import type { CompanyFeed } from "@/hooks/use-company";
import { ApprovalsView } from "@/views/ApprovalsView";

/**
 * **Issue #2028, at the queue rather than the card.** Two rules the page owes
 * once a blocker has four verdicts instead of two:
 *
 * * a bulk Approve can only send the two-value verdict, so sweeping a question
 *   into it would silently re-run every stopped step — the same flattening the
 *   card just stopped doing, at scale;
 * * a blocker settles its whole root-cause group, so every sibling the host
 *   names is this tab's decision too, or its resolution echo lands as a second
 *   toast for one click.
 */

const NOW = new Date("2026-08-23T10:00:00Z").getTime();

function gated(id: string): ApprovalSummary {
  return { id, kind: "web_fetch", amount_usd: null, at_millis: NOW };
}

function blocker(id: string): ApprovalSummary {
  return {
    id,
    kind: "blocker.infrastructure",
    amount_usd: null,
    at_millis: NOW,
    payload: { reason: "the model id `gpt-nope` was rejected" },
  };
}

interface Sent {
  id: string;
  verdict: Verdict;
  options: { blocker?: { verdict: string; answer?: string } };
}

function stubClient(sent: Sent[], answer: ChatResponse): OpenCompanyClient {
  return {
    get: async <T>(path: string): Promise<T> =>
      (path.endsWith("/users") ? [] : null) as T,
    listGrants: async () => [],
    listTeam: async () => [],
    revokeGrant: async () => undefined,
    scopeFor: () => "/api/v1/company",
    resolveApproval: async (
      id: string,
      verdict: Verdict,
      _note: undefined,
      _company: string | null,
      options: Sent["options"],
    ) => {
      sent.push({ id, verdict, options });
      return answer;
    },
  } as unknown as OpenCompanyClient;
}

function feedOf(approvals: ApprovalSummary[]): CompanyFeed {
  return {
    status: {} as CompanyFeed["status"],
    approvals,
    queue: "ready",
    now: NOW,
    refresh: async () => undefined,
  };
}

let container: HTMLDivElement;
let root: Root;

async function render(
  approvals: ApprovalSummary[],
  client: OpenCompanyClient,
  onDecideStart?: (id: string) => void,
) {
  await act(async () => {
    root.render(
      createElement(ApprovalsView, {
        client,
        company: null,
        feed: feedOf(approvals),
        onResolved: () => {},
        onGoToConversation: () => {},
        onDecideStart,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });
}

function byText(label: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLButtonElement | undefined;
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("bulk decisions leave blockers alone", () => {
  it("does not offer a bulk verdict over a queue of questions alone", async () => {
    const sent: Sent[] = [];
    await render(
      [blocker("b1"), blocker("b2")],
      stubClient(sent, { responses: [] }),
    );
    expect(byText("Approve all")).toBeUndefined();
    expect(byText("Decline all")).toBeUndefined();
  });

  it("sweeps only the gated calls when the queue holds both", async () => {
    const sent: Sent[] = [];
    // Three rows, two of them questions: the bulk control appears for the one
    // pair of gated calls and must not touch the blockers between them.
    await render(
      [gated("g1"), blocker("b1"), gated("g2"), blocker("b2")],
      stubClient(sent, { responses: [] }),
    );
    const approveAll = byText("Approve all");
    expect(approveAll, "two gated calls still earn a bulk control").toBeDefined();
    // `window.confirm` is the bulk gate; accept it for this run.
    const confirm = window.confirm;
    window.confirm = () => true;
    try {
      await act(async () => {
        approveAll?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
        await Promise.resolve();
        await Promise.resolve();
      });
    } finally {
      window.confirm = confirm;
    }
    expect(sent.map((s) => s.id)).toEqual(["g1", "g2"]);
    for (const call of sent) {
      expect(call.options.blocker).toBeUndefined();
    }
  });
});

describe("a blocker's root-cause siblings", () => {
  it("are claimed as this tab's decision when the host names them", async () => {
    const sent: Sent[] = [];
    const claimed: string[] = [];
    await render(
      [blocker("b1"), blocker("b2")],
      stubClient(sent, { responses: [], settledIds: ["b1", "b2"] }),
      (id) => claimed.push(id),
    );
    const skip = byText("Skip");
    expect(skip).toBeDefined();
    await act(async () => {
      skip?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(sent).toHaveLength(1);
    expect(sent[0].options.blocker?.verdict).toBe("skip");
    // Both: the clicked card before the request, and its sibling once the host
    // says it settled with it. Without the second, one click raises two toasts.
    expect(claimed).toEqual(["b1", "b2"]);
  });

  it("claims nothing extra on an ordinary resolve", async () => {
    const sent: Sent[] = [];
    const claimed: string[] = [];
    await render([gated("g1")], stubClient(sent, { responses: [] }), (id) =>
      claimed.push(id),
    );
    await act(async () => {
      byText("Approve")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(claimed).toEqual(["g1"]);
  });
});
