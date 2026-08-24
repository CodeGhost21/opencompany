// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { DeskDto, TeamMemberDto } from "@/api/types";

/**
 * Issue #1219: an unreachable host used to draw an empty company.
 *
 * `Overview` reads six independent sources in one `Promise.all`, and each was
 * individually `.catch()`-ed to an empty fallback. When the host cannot be
 * reached at all, every one of the six "fails" into the same empty value the
 * component also uses for a company that genuinely has nothing in it — so a
 * total outage and an empty company were indistinguishable, and the snapshot
 * clock re-stamped itself over a fetch that never actually landed.
 *
 * `KnowledgeGraph` — the lazy-loaded graph itself — is mocked out below. It
 * drives a force simulation off `requestAnimationFrame` and reads
 * `window.matchMedia`, neither of which jsdom provides, and none of that
 * machinery is under test here: what is under test is the outage state
 * `Overview` renders over it.
 */

vi.mock("@/views/overview/kg/KnowledgeGraph", () => ({
  // The snapshot corner is a slot the graph's shell positions (issue #1307),
  // because only the shell knows how wide the detail rail is. The stand-in
  // therefore has to render that slot; the outage itself is owned by Overview
  // and deliberately covers that chrome (issue #1314).
  KnowledgeGraph: ({ statusSlot }: { statusSlot?: unknown }) => statusSlot ?? null,
}));

const { Overview } = await import("@/views/Overview");

function desk(over: Partial<DeskDto> & Pick<DeskDto, "id" | "name">): DeskDto {
  return { members: [], ...over } as DeskDto;
}

function member(id: string, role = "Analyst"): TeamMemberDto {
  return { id, role };
}

/** Every `client.get` path this component reads, keyed by its suffix. */
const HEALTHY_GET: Record<string, unknown> = {
  "/tasks": [],
  "/users": [],
  // `GET /memory` answers with `{ items, totalContext, contextTruncated }` —
  // the truncation metadata rides beside the rows from one read.
  "/memory": { items: [], totalContext: 0, contextTruncated: false },
  "/workflows": [],
};

/**
 * A fake host, built from three raw mocks so a test can reconfigure them
 * later (a company switching from healthy to unreachable, say) without losing
 * the mock identity a plain object literal cast to `OpenCompanyClient` would.
 *
 * `get` is one mock dispatched by path suffix, since `listTasks`,
 * `listPeople`, `listMemory` and `listWorkflows` all route through it rather
 * than through their own client method.
 */
function fakeClient(over?: { desks?: DeskDto[]; team?: TeamMemberDto[] }) {
  const get = vi.fn((path: string) => {
    const suffix = Object.keys(HEALTHY_GET).find((k) => path.endsWith(k));
    return Promise.resolve(suffix ? HEALTHY_GET[suffix] : []);
  });
  const listDesks = vi.fn().mockResolvedValue(over?.desks ?? []);
  const listTeam = vi.fn().mockResolvedValue(over?.team ?? []);
  const client = {
    scopeFor: () => "/api/v1/company/acme",
    get,
    listDesks,
    listTeam,
  } as unknown as OpenCompanyClient;
  return { client, get, listDesks, listTeam };
}

/** Makes every one of the six reads fail at the transport, in place. */
function goUnreachable(mocks: ReturnType<typeof fakeClient>) {
  const fail = () => Promise.reject(new Error("ERR_CONNECTION_REFUSED"));
  mocks.get.mockImplementation(fail);
  mocks.listDesks.mockImplementation(fail);
  mocks.listTeam.mockImplementation(fail);
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(host: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(Overview, { client: host, company: "acme" }));
  });
  // The six sources resolve as one `Promise.all`(-Settled), but the state it
  // sets lands a tick later; give React that tick rather than assuming one
  // flush covers it.
  await act(async () => {});
  await act(async () => {});
}

function snapshotText(): string {
  return (
    [...container.querySelectorAll(".text-2xs")]
      .map((el) => el.textContent ?? "")
      .find((t) => t.includes("Snapshot") || t.includes("No snapshot") || t.includes("Loading")) ?? ""
  );
}

function alertText(): string | undefined {
  return container.querySelector('[role="alert"]')?.textContent ?? undefined;
}

function clickRefresh() {
  const button = [...container.querySelectorAll("button")].find((b) =>
    b.textContent?.includes("Refresh"),
  );
  button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

function retryButton(): HTMLButtonElement | undefined {
  return (
    container.querySelector<HTMLButtonElement>('[aria-label="Retry loading the company overview"]') ??
    undefined
  );
}

describe("a host that cannot be reached at all", () => {
  it("keeps Refresh at a 24px touch target below the desktop breakpoint", async () => {
    await render(fakeClient().client);

    const refresh = container.querySelector('[aria-label="Refresh the graph"]');
    expect(refresh).not.toBeNull();
    expect(refresh?.className).toContain("min-h-6");
    expect(refresh?.className).toContain("md:min-h-0");
  });

  it("covers the empty graph with a centered, actionable outage state", async () => {
    const unreachable = fakeClient();
    goUnreachable(unreachable);
    await render(unreachable.client);

    expect(alertText()).toContain("Could not reach the company");
    expect(container.querySelector('[data-testid="overview-outage"]')).not.toBeNull();
    expect(retryButton()?.textContent).toContain("Try again");
    // Never a company with nothing in it: the corner says there was no
    // snapshot to draw, not that one was taken and came back empty.
    expect(snapshotText()).not.toContain("Snapshot");
    expect(snapshotText()).toContain("No snapshot yet");
  });

  it("makes the covered graph inert while the outage shows", async () => {
    const unreachable = fakeClient();
    goUnreachable(unreachable);
    await render(unreachable.client);

    // The overlay exists and the covered shell — the wrapper around the
    // graph and its status slot — is inert, so a keyboard user cannot tab
    // into invisible Refresh / pillar / detail controls underneath (issue
    // #1314), and a screen reader does not expose the covered graph.
    expect(container.querySelector('[data-testid="overview-outage"]')).not.toBeNull();
    const shell = container.querySelector('[data-graph-shell]');
    expect(shell).not.toBeNull();
    expect(shell?.hasAttribute("inert")).toBe(true);
  });

  it("retries from the outage state", async () => {
    const unreachable = fakeClient();
    goUnreachable(unreachable);
    await render(unreachable.client);
    const readsBeforeRetry = unreachable.get.mock.calls.length;

    await act(async () => {
      retryButton()?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await act(async () => {});
    await act(async () => {});

    expect(unreachable.get.mock.calls.length).toBeGreaterThan(readsBeforeRetry);
  });

  it("keeps the previous snapshot's time rather than re-stamping it", async () => {
    const mocks = fakeClient({
      desks: [desk({ id: "research", name: "Research Desk", members: ["maya"] })],
      team: [member("maya")],
    });
    await render(mocks.client);
    expect(alertText()).toBeUndefined();
    const firstSnapshot = snapshotText();
    expect(firstSnapshot).toContain("Snapshot");

    // The same outage the issue reproduces: every one of the six reads now
    // fails at the transport. Reconfigure the mocks in place and press the
    // console's own Refresh control, rather than reaching into internals.
    goUnreachable(mocks);
    await act(async () => {
      clickRefresh();
    });
    await act(async () => {});
    await act(async () => {});

    expect(alertText()).toContain("Could not reach the company");
    expect(alertText()).toContain("Showing the last snapshot");
    // The whole point: the time on screen is the one the healthy load
    // produced, not a new one stamped over a graph that never actually
    // re-read anything.
    expect(snapshotText()).toBe(firstSnapshot);
  });
});

describe("a host that answers some sources and not others", () => {
  it("keeps drawing what it has, and raises no outage notice", async () => {
    const mocks = fakeClient({
      desks: [desk({ id: "research", name: "Research Desk", members: ["maya"] })],
      team: [member("maya")],
    });
    // A single failed source (desks) must not read as a total outage — the
    // other five still answered.
    mocks.listDesks.mockRejectedValue(new Error("desks unavailable"));

    await render(mocks.client);

    expect(alertText()).toBeUndefined();
    expect(snapshotText()).toContain("Snapshot");
  });
});
