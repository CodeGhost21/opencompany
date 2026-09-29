// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError, type McpHealth, type McpServer, type McpSource } from "@/api/types";

/**
 * The directory, folded into the one list.
 *
 * Discover was a second screen answering a question the first screen already had
 * to answer: an operator types a name to find out whether this company has that
 * server. Splitting the answer across two tabs is what let a second copy of an
 * installed server be installed at all — so the property that matters is that a
 * directory entry this company already holds is rendered one row from its own
 * copy and offers no install. The other two are cost and failure: nothing calls
 * the directory until something is typed, and a directory outage leaves the
 * company's half of the answer on screen.
 */

const api = vi.hoisted(() => ({
  listMcpServers: vi.fn(),
  testMcpServer: vi.fn(),
  discoverMcpTools: vi.fn(),
  addMcpServer: vi.fn(),
  removeMcpServer: vi.fn(),
  updateMcpServer: vi.fn(),
  startMcpOAuth: vi.fn(),
}));

const registryApi = vi.hoisted(() => ({
  connectMcpRegistryServer: vi.fn(),
  disconnectMcpRegistryServer: vi.fn(),
  getMcpRegistryEntry: vi.fn(),
  installMcpRegistryEntry: vi.fn(),
  searchMcpRegistry: vi.fn(),
  uninstallMcpRegistryServer: vi.fn(),
  updateMcpRegistryEnv: vi.fn(),
}));

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  message: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("@/api/mcp", () => api);
vi.mock("@/api/mcp-registry", () => registryApi);
vi.mock("sonner", () => ({
  toast: Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    message: toasts.message,
    warning: toasts.warning,
    info: toasts.info,
  }),
}));

const { McpServersSection } = await import(
  "@/views/connections/McpServersSection"
);

function row(over: Partial<McpServer> & { source: McpSource }): McpServer {
  return {
    name: "linear",
    endpoint: "https://mcp.linear.app/mcp",
    enabled: true,
    allowedTools: [],
    disallowedTools: [],
    readOnlyTools: [],
    timeoutSecs: 30,
    authConfigured: false,
    ...over,
  };
}

const OK: McpHealth = {
  status: "ok",
  message: "",
  toolCount: 9,
  checkedAtMillis: 1,
};

function entry(over: { qualifiedName: string; displayName: string } & Record<string, unknown>) {
  return {
    description: "Read and update issues.",
    source: "mcp_official",
    official: true,
    useCount: 0,
    ...over,
  };
}

const client = {
  capabilityStatus: () => Promise.resolve({ mcpInBuild: true }),
} as unknown as OpenCompanyClient;

let container: HTMLDivElement;
let root: Root;

function all(selector: string): HTMLElement[] {
  return [...document.body.querySelectorAll<HTMLElement>(selector)];
}

async function mount(servers: McpServer[]) {
  api.listMcpServers.mockResolvedValue(servers);
  await act(async () => {
    root.render(
      createElement(McpServersSection, {
        client,
        company: "acme",
        canManage: true,
        chrome: "standalone" as const,
      }),
    );
  });
}

/** Type into the one search field, then let the debounce elapse. */
async function search(term: string) {
  const field = document.body.querySelector<HTMLInputElement>(
    '[data-testid="mcp-search"]',
  );
  if (!field) throw new Error("no search field");
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )?.set;
  await act(async () => {
    setter?.call(field, term);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    vi.advanceTimersByTime(400);
    await Promise.resolve();
  });
  await act(async () => {
    await Promise.resolve();
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  vi.useFakeTimers({ shouldAdvanceTime: true });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("what opening the page costs", () => {
  it("calls the directory for nothing until something is typed", async () => {
    await mount([{ ...row({ source: "registry", serverId: "srv_1" }), health: OK }]);

    expect(registryApi.searchMcpRegistry).not.toHaveBeenCalled();
    // And there are no group headings, because there is one list and no second
    // half to distinguish it from.
    expect(all('[data-testid="mcp-group-row"]')).toHaveLength(0);
  });
});

describe("a directory entry this company already has", () => {
  it("sits one row from its own copy and offers no install", async () => {
    registryApi.searchMcpRegistry.mockResolvedValue({
      page: 1,
      totalPages: 1,
      servers: [
        entry({ qualifiedName: "@linear/mcp", displayName: "Linear" }),
        entry({
          qualifiedName: "@modelcontextprotocol/github",
          displayName: "GitHub",
        }),
      ],
    });

    await mount([
      {
        ...row({ source: "registry", serverId: "srv_1", qualifiedName: "@linear/mcp" }),
        health: OK,
      },
    ]);
    await search("linear");

    // Two headings carry the distinction the two tabs used to.
    const groups = all('[data-testid="mcp-group-row"]');
    expect(groups).toHaveLength(2);
    expect(groups[0]?.textContent).toContain("In this company");
    expect(groups[1]?.textContent).toContain("from the public directory");

    // The copy this company holds is matched by the qualified name, so the
    // directory row reports itself installed and offers nothing to press.
    const installed = all('[data-testid="mcp-directory-installed"]');
    expect(installed).toHaveLength(1);
    const offers = all('[data-testid="mcp-directory-install"]');
    expect(offers).toHaveLength(1);
    expect(offers[0]?.getAttribute("aria-label")).toBe("Install GitHub");
  });
});

describe("a directory outage", () => {
  it("leaves this company's own half of the answer on screen", async () => {
    registryApi.searchMcpRegistry.mockRejectedValue(
      new ApiError(502, "upstream", "The registry returned 502.", true),
    );

    await mount([{ ...row({ source: "runtime", name: "linear" }), health: OK }]);
    await search("linear");

    expect(all('[data-testid="mcp-registry-error"]')).toHaveLength(1);
    // The company's row is still there. A dead directory is not a broken page.
    expect(all('[data-testid="mcp-server-row"]')).toHaveLength(1);
    expect(all('[data-testid="mcp-load-error"]')).toHaveLength(0);
  });

  it("reads a build without the feature as a missing feature, not an error", async () => {
    registryApi.searchMcpRegistry.mockRejectedValue(
      new ApiError(404, "not_wired", "", true),
    );

    await mount([{ ...row({ source: "runtime" }), health: OK }]);
    await search("linear");

    expect(all('[data-testid="mcp-registry-unwired"]')).toHaveLength(1);
    expect(all('[data-testid="mcp-registry-error"]')).toHaveLength(0);
  });
});

describe("a search that matches nothing anywhere", () => {
  it("reports both halves and offers the route a directory never covers", async () => {
    registryApi.searchMcpRegistry.mockResolvedValue({
      page: 1,
      totalPages: 0,
      servers: [],
    });

    await mount([{ ...row({ source: "runtime", name: "linear" }), health: OK }]);
    await search("internal-reports");

    const card = document.body.querySelector('[data-testid="mcp-search-nothing"]');
    expect(card?.textContent).toContain("Nothing in this company matches");
    expect(card?.textContent).toContain("the directory has no listing for it");
    // An internal server will never be in a public directory, which is the
    // normal case for the URL flow rather than a failure.
    expect(card?.textContent).toContain("inside your own network");
  });
});
