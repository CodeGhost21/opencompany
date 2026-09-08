// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { DeskDto } from "@/api/types";
import type { TeamMember } from "@/lib/team";
import { ChannelCreateDialog } from "@/views/chat/ChannelCreateDialog";

/**
 * `POST {scope}/desks` (`create_desk`, `operator.rs`) is `scoped(…)` — any
 * company member, not admin-only — and `ChannelCreateDialog` carries no role
 * check of its own; `ChatView`'s own trigger (`onAddChannel`) is likewise
 * gated only on `fromHost && members.length > 0`, never on `isAdmin`. This
 * pins the dialog completes end to end for a plain member, and that a
 * refused create shows an honest, retryable error rather than closing on a
 * write that never landed.
 */

const MEMBER: TeamMember = {
  id: "m1",
  name: "Ada",
  role: "engineer",
  description: "",
  tone: "blue",
  avatar: "ada",
  inboxEnabled: false,
  effectiveTools: [],
  desks: [],
};

const CREATED: DeskDto = {
  id: "launch-week",
  name: "Launch week",
  description: "",
  members: ["m1"],
};

function clientAs(createDesk: () => Promise<DeskDto>): OpenCompanyClient {
  return {
    createDesk: vi.fn(createDesk),
  } as unknown as OpenCompanyClient;
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
  vi.restoreAllMocks();
});

async function render(client: OpenCompanyClient, onCreated = vi.fn()) {
  await act(async () => {
    root.render(
      createElement(ChannelCreateDialog, {
        client,
        company: "acme",
        members: [MEMBER],
        open: true,
        onOpenChange: vi.fn(),
        onCreated,
      }),
    );
  });
  return onCreated;
}

function nameInput(): HTMLInputElement {
  return document.body.querySelector('input[placeholder="e.g. Launch week"]') as HTMLInputElement;
}

function memberToggle(): HTMLButtonElement {
  return Array.from(document.body.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Ada"),
  ) as HTMLButtonElement;
}

function createButton(): HTMLButtonElement {
  return Array.from(document.body.querySelectorAll("button")).find(
    (b) => b.textContent === "Create channel" || b.textContent === "Creating…",
  ) as HTMLButtonElement;
}

function setInput(el: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("creating a channel, for a plain member (no role check in this dialog)", () => {
  it("completes end to end with no admin gate anywhere in the flow", async () => {
    const client = clientAs(() => Promise.resolve(CREATED));
    const onCreated = await render(client);

    setInput(nameInput(), "Launch week");
    await act(async () => memberToggle().click());
    await act(async () => createButton().click());
    await flush();

    expect(client.createDesk).toHaveBeenCalledWith(
      { name: "Launch week", description: undefined, members: ["m1"], responder: "auto" },
      "acme",
    );
    expect(onCreated).toHaveBeenCalledWith(CREATED);
  });
});

describe("creating a channel the host refuses", () => {
  it("shows an honest error and keeps the form open for a retry", async () => {
    const client = clientAs(() => Promise.reject(new Error("that name is already taken")));
    const onCreated = await render(client);

    setInput(nameInput(), "Launch week");
    await act(async () => memberToggle().click());
    await act(async () => createButton().click());
    await flush();

    expect(document.body.textContent).toContain("that name is already taken");
    expect(onCreated).not.toHaveBeenCalled();
    // Not stuck disabled — the operator can fix the name and try again.
    expect(createButton().disabled).toBe(false);
    expect(createButton().textContent).toBe("Create channel");
  });
});
