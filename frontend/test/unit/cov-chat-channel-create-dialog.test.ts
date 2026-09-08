// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMember } from "@/lib/team";
import { ChannelCreateDialog } from "@/views/chat/ChannelCreateDialog";

/**
 * The channel-create dialog's own field validation, independently traced.
 *
 * An empty-member channel is unroutable on the host by construction (the
 * responder selector has no candidate) — the dialog's own doc says a control
 * that will be refused is not offered a hopeful submit, so a member-less
 * submit must never reach `client.createDesk` at all. What it does reach —
 * a name the host still refuses for some other reason — must land as an
 * honest, retryable message rather than a stuck "Creating…" button.
 */

const MEMBERS: TeamMember[] = [
  {
    id: "m1",
    name: "Ada",
    role: "engineer",
    description: "",
    tone: "blue",
    avatar: "ada",
    inboxEnabled: false,
    effectiveTools: [],
    desks: [],
  },
];

function clientAs(createDesk: ReturnType<typeof vi.fn>): OpenCompanyClient {
  return { createDesk } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let onCreated: ReturnType<typeof vi.fn>;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom does not implement scrollIntoView; the blank-name refusal focuses
  // and scrolls the name field to it.
  Element.prototype.scrollIntoView = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  onCreated = vi.fn();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function render(client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(ChannelCreateDialog, {
        client,
        company: "acme",
        members: MEMBERS,
        open: true,
        onOpenChange: vi.fn(),
        onCreated,
      }),
    );
  });
}

function at(label: string): HTMLElement {
  const el = [...document.body.querySelectorAll("label")].find((l) => l.textContent === label);
  const id = el?.getAttribute("for");
  const input = id ? document.getElementById(id) : null;
  if (!input) throw new Error(`no field labelled "${label}"`);
  return input;
}

function submitButton(): HTMLButtonElement {
  return [...document.body.querySelectorAll("button")].find((b) => (b.textContent ?? "").includes("Create channel"))
    ?.closest("button") as HTMLButtonElement;
}

async function type(input: HTMLElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submit() {
  await act(async () => submitButton().click());
}

describe("submitting with nobody picked (AUTH — a submit the host would refuse as unroutable)", () => {
  it("shows the members error and never calls client.createDesk", async () => {
    const createDesk = vi.fn(async () => ({ id: "d1" }));
    await render(clientAs(createDesk));

    await type(at("Name"), "Launch week");
    await submit();

    expect(document.body.textContent).toContain("Pick at least one member");
    expect(createDesk).not.toHaveBeenCalled();
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("clears the members error the moment a member is picked", async () => {
    const createDesk = vi.fn(async () => ({ id: "d1" }));
    await render(clientAs(createDesk));

    await type(at("Name"), "Launch week");
    await submit();
    expect(document.body.textContent).toContain("Pick at least one member");

    await act(async () => {
      (document.body.querySelector('button[aria-pressed="false"]') as HTMLButtonElement)?.click();
    });

    expect(document.body.textContent).not.toContain("Pick at least one member");
  });
});

describe("submitting with a blank name", () => {
  it("shows the name error and never calls client.createDesk", async () => {
    const createDesk = vi.fn(async () => ({ id: "d1" }));
    await render(clientAs(createDesk));

    await submit();

    expect(document.body.textContent).toContain("Give the channel a name");
    expect(createDesk).not.toHaveBeenCalled();
  });
});

describe("a name and a member the host still refuses (FAIL)", () => {
  async function fillValidForm() {
    await type(at("Name"), "Launch week");
    await act(async () => {
      (document.body.querySelector('button[aria-pressed="false"]') as HTMLButtonElement)?.click();
    });
  }

  it("shows the host's own refusal and leaves Create pressable again", async () => {
    const createDesk = vi.fn(async () => {
      throw new Error("a channel named Launch week already exists");
    });
    await render(clientAs(createDesk));
    await fillValidForm();
    await submit();

    expect(createDesk).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("a channel named Launch week already exists");
    expect(submitButton().disabled).toBe(false);
    expect(submitButton().textContent).toBe("Create channel");
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("falls back to a generic message for a non-Error rejection", async () => {
    const createDesk = vi.fn(async () => {
      throw { status: 500 };
    });
    await render(clientAs(createDesk));
    await fillValidForm();
    await submit();

    expect(document.body.textContent).toContain("could not create the channel");
  });
});
