// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OpenCompanyClient } from "@/api/client";
import type { DeskDto, TeamMemberDto } from "@/api/types";

let lastCovered: boolean | undefined;
vi.mock("@/views/overview/kg/KnowledgeGraph", () => ({
  KnowledgeGraph: ({ statusSlot, covered }: { statusSlot?: unknown; covered?: boolean }) => {
    lastCovered = covered;
    return statusSlot ?? null;
  },
}));
const { Overview } = await import("@/views/Overview");

function desk(over: Partial<DeskDto> & Pick<DeskDto, "id" | "name">): DeskDto {
  return { members: [], ...over } as DeskDto;
}
function member(id: string, role = "Analyst"): TeamMemberDto { return { id, role }; }

const HEALTHY_GET: Record<string, unknown> = {
  "/tasks": [], "/users": [],
  "/memory": { items: [], totalContext: 0, contextTruncated: false },
  "/workflows": [],
};

function fakeClient(over?: { desks?: DeskDto[]; team?: TeamMemberDto[] }) {
  const get = vi.fn((path: string) => {
    const suffix = Object.keys(HEALTHY_GET).find((k) => path.endsWith(k));
    return Promise.resolve(suffix ? HEALTHY_GET[suffix] : []);
  });
  const listDesks = vi.fn().mockResolvedValue(over?.desks ?? []);
  const listTeam = vi.fn().mockResolvedValue(over?.team ?? []);
  const client = { scopeFor: () => "/api/v1/company/acme", get, listDesks, listTeam } as unknown as OpenCompanyClient;
  return { client, get, listDesks, listTeam };
}
function goUnreachable(mocks: ReturnType<typeof fakeClient>) {
  const fail = () => Promise.reject(new Error("ERR_CONNECTION_REFUSED"));
  mocks.get.mockImplementation(fail); mocks.listDesks.mockImplementation(fail); mocks.listTeam.mockImplementation(fail);
}
function goHealthy(mocks: ReturnType<typeof fakeClient>) {
  mocks.get.mockImplementation((path: string) => {
    const suffix = Object.keys(HEALTHY_GET).find((k) => path.endsWith(k));
    return Promise.resolve(suffix ? HEALTHY_GET[suffix] : []);
  });
  mocks.listDesks.mockResolvedValue([]); mocks.listTeam.mockResolvedValue([]);
}

let container: HTMLDivElement; let root: Root;
beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div"); document.body.appendChild(container);
  root = createRoot(container); lastCovered = undefined;
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

async function render(host: OpenCompanyClient) {
  await act(async () => { root.render(createElement(Overview, { client: host, company: "acme" })); });
  await act(async () => {}); await act(async () => {});
}
function retryButton(): HTMLButtonElement | undefined {
  return container.querySelector<HTMLButtonElement>('[aria-label="Retry loading the company overview"]') ?? undefined;
}

describe("probe", () => {
  it("focus handoff", async () => {
    const unreachable = fakeClient();
    goUnreachable(unreachable);
    await render(unreachable.client);
    console.log("after render, activeElement:", document.activeElement?.className, document.activeElement?.tagName);
    goHealthy(unreachable);
    await act(async () => { retryButton()?.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    await act(async () => {}); await act(async () => {});
    console.log("after retry, activeElement:", document.activeElement?.tagName, document.activeElement?.getAttribute("aria-label"));
    console.log("refresh button exists:", !!container.querySelector('[aria-label="Refresh the graph"]'));
    console.log("refresh button disabled:", (container.querySelector('[aria-label="Refresh the graph"]') as HTMLButtonElement)?.disabled);
    console.log("graph shell:", !!container.querySelector('[data-graph-shell]'));
    expect(true).toBe(true);
  });
});
