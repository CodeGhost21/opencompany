// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import { ApprovalPayload } from "@/components/approval-card";

(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;

function approval(payload: unknown): ApprovalSummary {
  return {
    id: "a1",
    kind: "http_request",
    amount_usd: null,
    at_millis: 1_000,
    payload,
  };
}

let container: HTMLDivElement;
let root: Root;

async function render(payload: unknown) {
  await act(async () => {
    root.render(createElement(ApprovalPayload, { approval: approval(payload) }));
  });
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the approval payload preview", () => {
  it("clamps a long value at three whole lines, then reveals it on request", async () => {
    await render({ url: `https://example.test/${"a".repeat(180)}` });

    const preview = container.firstElementChild?.firstElementChild;
    expect(preview?.classList).toContain("line-clamp-3");
    expect(preview?.classList).not.toContain("max-h-24");

    const button = container.querySelector("button");
    expect(button?.textContent).toContain("Show everything");
    await act(async () => button?.click());

    expect(preview?.classList).not.toContain("line-clamp-3");
    expect(button?.textContent).toContain("Show less");
  });

  it("keeps the line-count trigger on the same line-boundary clamp", async () => {
    await render({ url: "https://example.test", method: "POST", headers: {}, body: "hello" });

    const preview = container.firstElementChild?.firstElementChild;
    expect(preview?.classList).toContain("line-clamp-3");
    expect(container.querySelector("button")?.textContent).toContain("Show everything");
  });
});
