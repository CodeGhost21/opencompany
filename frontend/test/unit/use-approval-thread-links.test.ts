// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ApprovalSummary } from "@/api/types";
import { useApprovalThreadLinks, type ApprovalThreadLink } from "@/components/approval-card";

const T0 = new Date("2026-08-20T20:00:00Z").getTime();

function fakeClient(): OpenCompanyClient {
  return {
    listDesks: vi.fn(async () => [{ id: "engineering", name: "Engineering", members: [] }]),
    listTeam: vi.fn(async () => []),
  } as unknown as OpenCompanyClient;
}

function approval(id: string, thread?: string): ApprovalSummary {
  return {
    id,
    kind: "runtime.unlabelled_effect",
    amount_usd: null,
    at_millis: T0,
    agent: null,
    ...(thread ? { thread } : {}),
  };
}

let container: HTMLDivElement;
let root: Root;
let lastLinks: Map<string, ApprovalThreadLink> | null;

function Probe({
  client,
  approvals,
}: {
  client: OpenCompanyClient;
  approvals: ApprovalSummary[];
}) {
  lastLinks = useApprovalThreadLinks(client, "acme", approvals);
  return null;
}

async function render(client: OpenCompanyClient, approvals: ApprovalSummary[]) {
  await act(async () => {
    root.render(createElement(Probe, { client, approvals }));
  });
  // The effect resolves `listDesks`/`listTeam` in a microtask; let the
  // topology land before asserting on the derived links.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  lastLinks = null;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("useApprovalThreadLinks", () => {
  it("links a pending request whose thread resolves to a desk", async () => {
    const client = fakeClient();
    await render(client, [approval("a1", "engineering")]);

    expect(lastLinks?.get("a1")).toEqual({ channelId: "engineering", label: "#engineering" });
  });

  it("derives links for a newly arrived approval on an already-known thread", async () => {
    const client = fakeClient();
    await render(client, [approval("a1", "engineering")]);
    expect(lastLinks?.get("a1")).toEqual({ channelId: "engineering", label: "#engineering" });

    // The second request shares a1's thread, so the set of distinct thread ids
    // — the key the topology fetch is keyed on — does not change. The link map
    // must still pick up a2, or its card would omit the "Asked in" line until
    // some later thread change rebuilt the map.
    await render(client, [approval("a1", "engineering"), approval("a2", "engineering")]);

    expect(lastLinks?.get("a1")).toEqual({ channelId: "engineering", label: "#engineering" });
    expect(lastLinks?.get("a2")).toEqual({ channelId: "engineering", label: "#engineering" });
  });

  it("leaves an unresolvable thread unlinked", async () => {
    const client = fakeClient();
    await render(client, [approval("a1", "someone-else")]);

    expect(lastLinks?.has("a1")).toBe(false);
  });

  it("falls back to the default desks when /desks comes back empty", async () => {
    // A company with no declared `[[group_chat]]` entries gets `[]` from
    // /desks, yet ChatView and AppShell still show the default desks. An
    // approval raised in one of those (the `main` thread) must resolve too,
    // or its "Asked in" link would silently disappear.
    const client = {
      listDesks: vi.fn(async () => []),
      listTeam: vi.fn(async () => []),
    } as unknown as OpenCompanyClient;
    await render(client, [approval("a1", "main")]);

    expect(lastLinks?.get("a1")).toEqual({ channelId: "main", label: "#general" });
  });
});
