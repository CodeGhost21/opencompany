// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type {
  ApprovalSummary,
  GrantScope,
  StandingGrant,
  Verdict,
} from "@/api/types";
import { money } from "@/lib/language";
import { ApprovalCard, StandingPermissions } from "@/views/ApprovalsView";

/**
 * Issue #1406: Approve and Decline must not sit above the evidence and the
 * scope control that changes what Approve does.
 *
 * This suite is normally for pure functions — see `vitest.config.ts` — and it
 * earns the exception the same way `approval-batch-card` does: the claim is
 * about the DOM the operator's pointer travels through, and only a render can
 * show whether the commit affordance comes before or after the control that
 * redefines it. The old card put Approve in the headline's `actions` slot, level
 * with the title and ~200px above the "If you approve" fieldset; a pure test of
 * any helper cannot see that ordering at all.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

// `broadly_grantable` so the scope control renders — it is the whole point of
// the issue — and a multi-line payload so the card is tall, the condition under
// which the scope control used to fall off-screen below the button.
const APPROVAL: ApprovalSummary = {
  id: "a1",
  kind: "shell",
  amount_usd: null,
  at_millis: T0,
  agent: "ops",
  broadly_grantable: true,
  payload: { command: "rm -rf /tmp/build && make release", cwd: "/srv/app" },
};

let container: HTMLDivElement;
let root: Root;

async function render(approval: ApprovalSummary) {
  await act(async () => {
    root.render(
      createElement(ApprovalCard, {
        approval,
        now: T0 + 60_000,
        askerNames: new Map([["ops", "Ops"]]),
        deciding: null,
        batchIndex: 1,
        batchTotal: 1,
        onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
      }),
    );
  });
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

/** The Approve button, wherever it ended up. */
function approveButton(): HTMLButtonElement {
  const btn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Approve"),
  );
  if (!btn) throw new Error("no Approve button rendered");
  return btn as HTMLButtonElement;
}

const GRANT: StandingGrant = {
  id: "grant-1",
  agent: "ops",
  tool: "web_fetch",
  scope: "https://docs.rs",
  granted_by: { kind: "user", id: "operator" },
  at_millis: T0,
  expires_at_millis: T0 + 60 * 60_000,
  verdict: "approve",
};

describe("ApprovalCard decide ordering (#1406)", () => {
  it("renders the scope control before the decide buttons in DOM order", async () => {
    await render(APPROVAL);

    const scope = container.querySelector("fieldset");
    const approve = approveButton();
    expect(
      scope,
      "the scope control should render for a broadly-grantable card",
    ).not.toBeNull();

    // `DOCUMENT_POSITION_FOLLOWING` on the scope element, tested against the
    // Approve button, means the button comes *after* the scope control — the
    // reading order #1406 requires: evidence and scope first, commit last.
    const position = scope!.compareDocumentPosition(approve);
    expect(position & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("keeps both decide buttons in the footer, below the scope control", async () => {
    await render(APPROVAL);

    const footer = container.querySelector('[data-testid="approval-decide"]');
    expect(footer, "the decide footer should exist").not.toBeNull();
    // Both verbs live in the footer — not one moved and one left behind.
    expect(footer!.textContent).toContain("Approve");
    expect(footer!.textContent).toContain("Decline");

    const scope = container.querySelector("fieldset")!;
    const position = scope.compareDocumentPosition(footer!);
    expect(position & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("names each decision with the request it affects (#1411)", async () => {
    await render(APPROVAL);

    // The action phrase alone ("Run a terminal command") is identical for two
    // same-kind cards, so the label carries the command, the distinguishing
    // follow-ups (here `cwd`), and the asker too. Compared by attribute, not
    // CSS selector: the command's `&&` is legal in an attribute value but
    // trips jsdom's selector engine.
    const labelled = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labelled).toContain(
      `Approve: Run a terminal command — rm -rf /tmp/build && make release — cwd: /srv/app — asked by Ops — request ${T0}`,
    );
    expect(labelled).toContain(
      `Decline: Run a terminal command — rm -rf /tmp/build && make release — cwd: /srv/app — asked by Ops — request ${T0}`,
    );
  });

  it("distinguishes two same-URL http_request cards by method (#1411)", async () => {
    const get: ApprovalSummary = {
      ...APPROVAL,
      id: "get",
      kind: "http_request",
      payload: { url: "https://api.example.com/items", method: "GET" },
    };
    const del: ApprovalSummary = {
      ...APPROVAL,
      id: "del",
      kind: "http_request",
      payload: { url: "https://api.example.com/items", method: "DELETE" },
    };

    await act(async () => {
      root.render(
        createElement(
          "div",
          null,
          createElement(ApprovalCard, {
            approval: get,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 1,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
          createElement(ApprovalCard, {
            approval: del,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 2,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
        ),
      );
    });

    const labelled = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    // Same URL, different method — the accessible names must not collide: the
    // method rides after the URL with its label so the two buttons read apart.
    expect(labelled).toContain(
      `Approve: Make a request to a web address — https://api.example.com/items — method: GET — request ${T0} — approval 1 of 1 — approval 1 of 2 — asked by Ops`,
    );
    expect(labelled).toContain(
      `Approve: Make a request to a web address — https://api.example.com/items — method: DELETE — request ${T0} — approval 1 of 1 — approval 2 of 2 — asked by Ops`,
    );
  });

  it("keeps truncated payload labels distinct with the dropped argument (#1411)", async () => {
    // Same url, method and headers — the only difference is the body, which
    // sits past `MAX_LEAD_LINES` and is dropped from the label. The dropped
    // line's own start must ride along so the two buttons do not read
    // identically — and it must be the argument's words, not the card id, so
    // the button names what the operator can see on the card body.
    const a: ApprovalSummary = {
      ...APPROVAL,
      id: "req-1",
      kind: "http_request",
      payload: {
        url: "https://api.example.com/items",
        method: "POST",
        headers: "content-type: application/json",
        body: '{"q": 1}',
      },
    };
    const b: ApprovalSummary = {
      ...APPROVAL,
      id: "req-2",
      kind: "http_request",
      payload: {
        url: "https://api.example.com/items",
        method: "POST",
        headers: "content-type: application/json",
        body: '{"q": 2}',
      },
    };

    await act(async () => {
      root.render(
        createElement(
          "div",
          null,
          createElement(ApprovalCard, {
            approval: a,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 1,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
          createElement(ApprovalCard, {
            approval: b,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 2,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
        ),
      );
    });

    const labelled = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labelled).toContain(
      `Approve: Make a request to a web address — https://api.example.com/items — method: POST — headers: content-type: application/json — body: {"q": 1} — request ${T0} — approval 1 of 1 — approval 1 of 2 — asked by Ops`,
    );
    expect(labelled).toContain(
      `Approve: Make a request to a web address — https://api.example.com/items — method: POST — headers: content-type: application/json — body: {"q": 2} — request ${T0} — approval 1 of 1 — approval 2 of 2 — asked by Ops`,
    );
  });

  it("distinguishes two same-payload payment cards by amount (#1411)", async () => {
    const small: ApprovalSummary = {
      ...APPROVAL,
      id: "pay-40",
      kind: "payment.send",
      amount_usd: 40,
      payload: { recipient: "acme" },
    };
    const big: ApprovalSummary = {
      ...APPROVAL,
      id: "pay-4000",
      kind: "payment.send",
      amount_usd: 4000,
      payload: { recipient: "acme" },
    };

    await act(async () => {
      root.render(
        createElement(
          "div",
          null,
          createElement(ApprovalCard, {
            approval: small,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 1,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
          createElement(ApprovalCard, {
            approval: big,
            now: T0 + 60_000,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 2,
            batchTotal: 2,
            onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
          }),
        ),
      );
    });

    const labelled = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    // Identical payload and asker — the amount is the only thing that tells
    // the $40 decision from the $4,000 one, so it has to ride in the name.
    // The amount is formatted via `money`, so the expectation uses it rather
    // than a locale-hardcoded literal. The recipient keeps its label because
    // `payment.send` has no predictable first argument name — dropping it
    // would let `{recipient: "acme"}` and `{payee: "acme"}` collide (#1411).
    expect(labelled).toContain(
      `Approve: Send a payment — ${money(40)} — recipient: acme — request ${T0} — approval 1 of 1 — approval 1 of 2 — asked by Ops`,
    );
    expect(labelled).toContain(
      `Approve: Send a payment — ${money(4000)} — recipient: acme — request ${T0} — approval 1 of 1 — approval 2 of 2 — asked by Ops`,
    );
  });

  it("names each identical batched approval by its human-readable position (#1411)", async () => {
    const first = { ...APPROVAL, id: "batch-1" };
    const second = { ...APPROVAL, id: "batch-2" };
    await act(async () => {
      root.render(
        createElement(
          "div",
          null,
          createElement(ApprovalCard, {
            approval: first,
            now: T0,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 1,
            batchTotal: 2,
            onDecide: () => {},
          }),
          createElement(ApprovalCard, {
            approval: second,
            now: T0,
            askerNames: new Map([["ops", "Ops"]]),
            deciding: null,
            batchIndex: 2,
            batchTotal: 2,
            onDecide: () => {},
          }),
        ),
      );
    });
    const labels = Array.from(container.querySelectorAll("button"), (button) =>
      button.getAttribute("aria-label"),
    );
    expect(labels.filter((label) => label?.startsWith("Approve:"))).toEqual([
      expect.stringContaining("approval 1 of 2"),
      expect.stringContaining("approval 2 of 2"),
    ]);
  });

  it("names each permission revocation with the grantee it affects (#1411)", async () => {
    await act(async () => {
      root.render(
        createElement(StandingPermissions, {
          grants: [GRANT],
          now: T0,
          askerNames: new Map([["ops", "Ops"]]),
          granterNames: new Map([["operator", "you"]]),
          onRevoke: async () => {},
        }),
      );
    });

    expect(
      container.querySelector(
        'button[aria-label="Revoke Ops\'s permission: Fetch a web page — https://docs.rs only — expires in 1h — grant 1 of 1"]',
      ),
    ).not.toBeNull();
  });
  it("distinguishes revocations for identical grants with different expirations (#1411)", async () => {
    const first: StandingGrant = {
      ...GRANT,
      id: "grant-short",
      expires_at_millis: T0 + 60 * 60 * 1000,
    };
    const second: StandingGrant = {
      ...GRANT,
      id: "grant-long",
      expires_at_millis: T0 + 7 * 24 * 60 * 60 * 1000,
    };
    await act(async () => {
      root.render(
        createElement(StandingPermissions, {
          grants: [first, second],
          now: T0,
          askerNames: new Map([["ops", "Ops"]]),
          granterNames: new Map([["operator", "you"]]),
          onRevoke: async () => {},
        }),
      );
    });
    const labels = Array.from(container.querySelectorAll("button"), (button) =>
      button.getAttribute("aria-label"),
    );
    expect(labels).toEqual([
      "Revoke Ops's permission: Fetch a web page — https://docs.rs only — expires in 1h — grant 1 of 2",
      "Revoke Ops's permission: Fetch a web page — https://docs.rs only — expires in 7d — grant 2 of 2",
    ]);
  });

  it("names a workflow grant revocation after the workflow (#1411)", async () => {
    // A workflow grant carries no agent (`agent` is empty, issue #1098) — its
    // subject lives in `workflow`, and the revocation label must name that
    // workflow, not the empty string the agent field would yield.
    const workflowGrant: StandingGrant = {
      ...GRANT,
      id: "grant-deploy",
      agent: "",
      workflow: "deploy",
    };

    await act(async () => {
      root.render(
        createElement(StandingPermissions, {
          grants: [workflowGrant],
          now: T0,
          askerNames: new Map([["ops", "Ops"]]),
          granterNames: new Map([["operator", "you"]]),
          onRevoke: async () => {},
        }),
      );
    });

    expect(
      container.querySelector(
        'button[aria-label="Revoke the deploy workflow\'s permission: Fetch a web page — https://docs.rs only — expires in 1h — grant 1 of 1"]',
      ),
    ).not.toBeNull();
  });
});
