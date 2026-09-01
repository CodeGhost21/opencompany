// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { ChatView } from "@/views/ChatView";

/**
 * The channel composer answers a read-only channel by not existing, and the
 * echo-brain notice sits next to the control it qualifies.
 *
 * # Why a render test and not a source scan
 *
 * Both facts are about what is *on screen*, and both were previously "true"
 * in a form that read as fixed and was not. The composer was `disabled`, which
 * is still a claim that the action exists: under a notice reading "There is
 * nothing to reply to here", `#Operator` drew a text input, three intent
 * chips, a mention button, a paperclip, a formatting toggle, a Send button and
 * an "Enter to send" hint. And the notice explaining that replies come from
 * the offline echo brain sat above the transcript, at the far end of the page
 * from the Send that provokes one.
 *
 * A grep cannot tell a rendered control from a removed one, so this mounts the
 * real `ChatView` against a stub client and asks the DOM.
 *
 * # The writable half is not optional
 *
 * Every read-only assertion here is an assertion of absence, and absence is
 * also what a `ChatView` that failed to mount produces. The writable cases
 * pin the same queries finding everything, off the same fixture — so a
 * mount that silently renders nothing fails rather than passing twice.
 */

const OPERATOR_DTO = {
  id: "operator",
  name: "Operator",
  description: "Workflow reports and notifications",
};

const DESK_DTO = {
  id: "main",
  name: "main",
  description: "The main channel",
  members: [] as string[],
};

function stubClient(cognition: string | null): OpenCompanyClient {
  return {
    listDesks: vi.fn(async () => [DESK_DTO]),
    listTeam: vi.fn(async () => []),
    mentionables: vi.fn(async () => []),
    getOperatorChannel: vi.fn(async () => OPERATOR_DTO),
    capabilityStatus: vi.fn(async () => ({ cognition })),
    chat: vi.fn(),
    reactToMessage: vi.fn(),
    getBudgetPause: vi.fn(async () => null),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // `useIsDesktop` reads `matchMedia`, which jsdom does not implement. A
  // desktop viewport keeps both panes mounted, which is the case under test.
  window.matchMedia = ((query: string) => ({
    matches: query.includes("min-width"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  Object.defineProperty(window, "innerWidth", { value: 1440, writable: true });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(sub: string, cognition: string | null = null) {
  const client = stubClient(cognition);
  const view = createElement(ChatView, {
    client,
    company: "acme",
    sub,
    onNavigate: vi.fn(),
    transcripts: {},
    setTranscripts: vi.fn(),
    // The live-scope escape hatch `send` reads to decide whether a reply still
    // belongs to the company on screen. Nothing here sends.
    scopeRef: { current: { connection: "local", company: "acme", client } },
  });
  const node: ReactNode = createElement(ConnectionScopeProvider, {
    scope: { connection: "local", company: "acme" },
    children: view,
  });
  await act(async () => {
    root.render(node);
  });
  // Let the desks / operator / capability reads settle.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** The main channel composer's textarea — `MessageComposer` labels it. */
function composerInput() {
  return container.querySelector('textarea[aria-label^="Message "]');
}

function readOnlyComposerInput() {
  return container.querySelector('textarea[aria-label="This channel is read-only"]');
}

function banner() {
  return container.querySelector('[data-testid="chat-cognition-banner"]');
}

describe("a read-only channel renders no composer", () => {
  it("draws neither the composer nor its placeholder", async () => {
    await mount("operator");

    expect(composerInput()).toBeNull();
    expect(readOnlyComposerInput()).toBeNull();
    expect(container.querySelector("textarea")).toBeNull();
  });

  it("draws no Send button and no intent chips", async () => {
    await mount("operator");

    expect(container.querySelector('[aria-label="Send"]')).toBeNull();
    expect(container.querySelector('[aria-label="What this message is for"]')).toBeNull();
    for (const chip of ["Just chatting", "Do it once", "Build me the workflow"]) {
      expect(container.textContent).not.toContain(chip);
    }
  });

  it("draws none of the mention, attach or formatting controls", async () => {
    await mount("operator");

    for (const label of ["Mention someone", "Attach a file", "Formatting"]) {
      expect(container.querySelector(`[aria-label="${label}"]`)).toBeNull();
    }
  });

  it("drops the keyboard hint, which describes a send that cannot happen", async () => {
    await mount("operator");

    expect(container.textContent).not.toContain("to send");
    expect(container.textContent).not.toContain("for a new line");
  });

  it("keeps the notice that explains why", async () => {
    await mount("operator");

    expect(container.textContent).toContain("There is nothing to reply to here");
  });

  it("offers neither empty-state card, since neither action exists here", async () => {
    await mount("operator");

    // "Give the team a brief" prefills a composer this channel does not
    // render; "Add people" opens a members pane `ChatView` gates off on the
    // same flag. Both were dead controls under the notice.
    expect(container.textContent).not.toContain("Give the team a brief");
    expect(container.textContent).not.toContain("Add people");
  });
});

describe("a writable channel still renders the whole composer", () => {
  it("draws the input, the Send button, the chips and the controls", async () => {
    await mount("main");

    expect(composerInput()).not.toBeNull();
    expect(container.querySelector('[aria-label="Send"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="What this message is for"]')).not.toBeNull();
    for (const label of ["Mention someone", "Formatting"]) {
      expect(container.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    }
    expect(container.textContent).toContain("to send");
    expect(container.textContent).not.toContain("There is nothing to reply to here");
  });

  it("still offers the empty-state cards", async () => {
    await mount("main");

    expect(container.textContent).toContain("Give the team a brief");
    expect(container.textContent).toContain("Add people");
  });
});

describe("the harness-unavailable notice sits next to the composer", () => {
  it("renders the notice, unchanged, on a writable channel", async () => {
    await mount("main", "unavailable");

    const strip = banner();
    expect(strip).not.toBeNull();
    expect(strip?.textContent).toContain(
      "This host cannot reach a model — no agent harness is available.",
    );
    expect(strip?.textContent).toContain(
      "The replies below come from the offline echo brain rather than the teammate they " +
        "appear under. No setting changes that: it takes a host built and started with the " +
        "harness.",
    );
  });

  it("places it below the transcript and above the composer", async () => {
    await mount("main", "unavailable");

    const strip = banner()!;
    const input = composerInput()!;
    expect(strip).not.toBeNull();
    expect(input).not.toBeNull();

    // `MessageTimeline`'s root is the scrolling viewport. The notice, the
    // scroller and the composer are all direct children of the same flex
    // column, so their order in that column is the order on screen — which is
    // the entire claim being made: the notice qualifies the Send below it, not
    // the transcript above it.
    const column = strip.parentElement!;
    const kids = Array.from(column.children);
    const scroller = column.querySelector(":scope > div.overflow-y-auto")!;
    const composerRoot = kids.find((el) => el.contains(input))!;

    expect(scroller).not.toBeNull();
    expect(composerRoot).not.toBeUndefined();
    expect(kids.indexOf(scroller)).toBeLessThan(kids.indexOf(strip));
    expect(kids.indexOf(strip)).toBeLessThan(kids.indexOf(composerRoot));
  });

  it("is suppressed on a read-only channel, where nothing can be sent", async () => {
    await mount("operator", "unavailable");

    expect(banner()).toBeNull();
    expect(container.textContent).toContain("There is nothing to reply to here");
  });
});
