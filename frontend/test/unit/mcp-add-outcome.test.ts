// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError, type McpHealth, type McpServer, type McpSource } from "@/api/types";

/**
 * What the add dialog says happened.
 *
 * The add path has two outcomes: the host refusing the write, and the host
 * accepting it and then failing to reach the endpoint. Only the first is a
 * failure. The second leaves a server saved, enabled and attached to every
 * agent that reaches it — so titling it "Couldn't add the server" tells the
 * operator the opposite of what happened and invites a second add of a server
 * that is already there.
 *
 * The outcome lands in the dialog, on the server it is about, and offers the
 * next move. The third thing pinned here is the description field.
 */

const api = vi.hoisted(() => ({
  addMcpServer: vi.fn(),
  updateMcpServer: vi.fn(),
}));

vi.mock("@/api/mcp", () => api);

const { McpAddServerDialog } = await import(
  "@/views/connections/McpAddServerDialog"
);

const client = {} as unknown as OpenCompanyClient;

function server(over: Partial<McpServer> & { source: McpSource }): McpServer {
  return {
    name: "deadsrv",
    endpoint: "https://mcp.example.com/mcp",
    enabled: true,
    allowedTools: [],
    disallowedTools: [],
    readOnlyTools: [],
    timeoutSecs: 30,
    authConfigured: false,
    ...over,
  };
}

const UNREACHABLE: McpHealth = {
  status: "error",
  message: "MCP server 'deadsrv' couldn't be used: mcp transport failure.",
  toolCount: 0,
  checkedAtMillis: 1,
};

let container: HTMLDivElement;
let root: Root;
const opened: string[] = [];

/** The dialog renders into a portal, so its DOM is the document's, not ours. */
function el(testId: string): HTMLElement | null {
  return document.body.querySelector(`[data-testid="${testId}"]`);
}

function text(): string {
  return document.body.textContent ?? "";
}

async function mount() {
  await act(async () => {
    root.render(
      createElement(McpAddServerDialog, {
        client,
        company: "acme",
        open: true,
        bridge: "present" as const,
        onOpenChange: () => {},
        onAdded: () => {},
        onOpenServer: (name: string) => opened.push(name),
      }),
    );
  });
}

/** Type into a controlled field the way React reads it. */
async function type(selector: string, value: string) {
  const node = document.body.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector);
  if (!node) throw new Error(`no ${selector}`);
  const proto =
    node instanceof HTMLTextAreaElement
      ? window.HTMLTextAreaElement.prototype
      : window.HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  await act(async () => {
    setter?.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submit() {
  const button = el("mcp-add-submit");
  if (!button) throw new Error("no submit");
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Fill the two required fields and press the one button that saves. */
async function add() {
  await type('[data-testid="mcp-add-name"]', "deadsrv");
  await type('[data-testid="mcp-add-endpoint"]', "https://mcp.example.com/mcp");
  await submit();
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  opened.length = 0;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("an add the host accepted but could not reach", () => {
  beforeEach(() => {
    api.addMcpServer.mockResolvedValue({
      server: server({ source: "runtime" }),
      note: "Agents pick it up on their next turn.",
      test: UNREACHABLE,
    });
  });

  it("does not call it a failure to add", async () => {
    await mount();
    await add();

    expect(text()).not.toContain("Couldn't add the server");
    expect(text()).toContain("Added, but not answering");
  });

  it("says where it went, so nobody adds it twice", async () => {
    await mount();
    await add();

    expect(el("mcp-add-outcome")?.textContent).toContain(
      "saved and listed with your servers",
    );
  });

  it("still surfaces the probe's own words", async () => {
    await mount();
    await add();

    expect(text()).toContain("mcp transport failure");
  });

  it("offers the next move instead of naming a row to go and find", async () => {
    await mount();
    await add();

    const open = el("mcp-add-open-server");
    expect(open).not.toBeNull();
    await act(async () => {
      open?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(opened).toEqual(["deadsrv"]);
  });
});

describe("an add the host refused", () => {
  it("is the one that reads as a failure to add", async () => {
    api.addMcpServer.mockRejectedValue(
      new ApiError(400, "invalid", "an https endpoint is required", true),
    );

    await mount();
    await add();

    expect(el("mcp-add-error")?.textContent).toContain(
      "an https endpoint is required",
    );
    // Nothing was saved, so there is no server for an outcome to be about.
    expect(el("mcp-add-outcome")).toBeNull();
    expect(el("mcp-add-open-server")).toBeNull();
  });

  it("puts a name already taken on the field it is about", async () => {
    api.addMcpServer.mockRejectedValue(
      new ApiError(409, "conflict", "a server named notion already exists", true),
    );

    await mount();
    await add();

    // A form-level alert makes the operator re-read four fields to find which
    // one the host refused.
    expect(el("mcp-add-name-error")?.textContent).toContain(
      "a server named notion already exists",
    );
    expect(el("mcp-add-error")).toBeNull();
    // Pressing it again would be refused again, which is not validation.
    expect(el("mcp-add-submit")?.getAttribute("disabled")).not.toBeNull();
  });
});

describe("what a server calls itself", () => {
  it("is offered when nothing was typed, rather than left nameless", async () => {
    api.addMcpServer.mockResolvedValue({
      server: server({
        source: "runtime",
        probedDescription: "Search, read and update pages.",
      }),
      note: "Agents pick it up on their next turn.",
      test: { status: "ok", message: "", toolCount: 16, checkedAtMillis: 2 },
    });
    api.updateMcpServer.mockResolvedValue({ server: server({ source: "runtime" }), note: "" });

    await mount();
    await add();

    expect(el("mcp-add-probed-description")?.textContent).toContain(
      "Search, read and update pages.",
    );
    await act(async () => {
      el("mcp-add-use-probed")?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    expect(api.updateMcpServer).toHaveBeenCalledWith(client, "acme", "deadsrv", {
      description: "Search, read and update pages.",
    });
  });

  it("is not offered over a description the operator wrote", async () => {
    api.addMcpServer.mockResolvedValue({
      server: server({
        source: "runtime",
        description: "Our own reporting replica.",
        probedDescription: "Search, read and update pages.",
      }),
      note: "Agents pick it up on their next turn.",
      test: { status: "ok", message: "", toolCount: 16, checkedAtMillis: 2 },
    });

    await mount();
    await type('[data-testid="mcp-add-description"]', "Our own reporting replica.");
    await add();

    expect(el("mcp-add-probed-description")).toBeNull();
    expect(api.addMcpServer).toHaveBeenCalledWith(
      client,
      "acme",
      expect.objectContaining({ description: "Our own reporting replica." }),
    );
  });
});

describe("a build with no MCP bridge", () => {
  it("says the probe cannot be acted on, and still offers the save", async () => {
    await act(async () => {
      root.render(
        createElement(McpAddServerDialog, {
          client,
          company: "acme",
          open: true,
          bridge: "absent" as const,
          onOpenChange: () => {},
          onAdded: () => {},
          onOpenServer: () => {},
        }),
      );
    });

    expect(el("mcp-add-no-bridge")).not.toBeNull();
    // "Save anyway", not "Add": the configuration is valid and survives the
    // rebuild, but nothing is connected.
    expect(el("mcp-add-submit")?.textContent).toContain("Save anyway");
  });
});
