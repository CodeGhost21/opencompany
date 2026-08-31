import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * A completed background task must not have its toast suppressed by a
 * transcript the operator cannot actually see (#1768 codex review).
 *
 * Below `lg`, selecting the channel rail hides `ChatView`'s transcript
 * (`chatPaneVisible === false`) while leaving it mounted — but
 * `activeChatChannelRef` in the shell only updates from `onChannelViewed`,
 * which stops firing the moment the pane hides. So the ref keeps naming
 * whichever channel was on screen right before the rail was opened, and a
 * `desk_task_completed` event from that channel makes `isViewingTaskOrigin`
 * report "still watching it" even though the inline marker it is deferring
 * to is off screen. The operator gets no toast and no visible marker.
 *
 * A jsdom render of `app-shell` cannot prove this — it needs the whole
 * client and every hook, the same reason `chat-rail-focus.test.ts` and
 * `responsive-two-rail-band.test.ts` fall back to a source-contract check.
 * This guards the wiring the fix rests on: ChatView reports pane visibility
 * on its own channel, separate from `onChannelViewed`'s channel-identity
 * report, and the shell's origin check consults it before trusting the
 * remembered channel id.
 */

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("a hidden mobile chat pane cannot suppress a completion toast (#1768)", () => {
  const chatView = read("views/ChatView.tsx");
  const appShell = read("components/app-shell.tsx");

  it("ChatView reports chatPaneVisible on its own dedicated channel", () => {
    // Not folded into `onChannelViewed`, which only fires — and only ever
    // fired — while the pane is visible, so it cannot report the hide edge.
    expect(chatView).toContain("onChatPaneVisibilityChange?.(chatPaneVisible);");
  });

  it("the shell tracks pane visibility separately from the remembered channel", () => {
    // Distinct from `activeChatChannelRef`: that ref has a second job
    // (addressing an unaddressed system line after a walk to Approvals) that
    // must keep using the last-known channel even while the rail is showing,
    // so visibility cannot be folded into clearing it.
    expect(appShell).toContain("const chatPaneVisibleRef = useRef(true);");
    expect(appShell).toContain(
      "const onChatPaneVisibilityChange = useCallback((visible: boolean) => {\n    chatPaneVisibleRef.current = visible;\n  }, []);",
    );
  });

  it("wires the shell's tracker to ChatView's report", () => {
    expect(appShell).toContain("onChatPaneVisibilityChange={onChatPaneVisibilityChange}");
  });

  it("isViewingTaskOrigin refuses to claim visibility while the pane is hidden", () => {
    const idx = appShell.indexOf("isViewingTaskOrigin: useCallback(");
    expect(idx).toBeGreaterThan(-1);
    const body = appShell.slice(idx, idx + 900);
    expect(body).toContain('if (!chatPaneVisibleRef.current) return false;');
    // The visibility check must come before the origin channel is trusted.
    expect(body.indexOf("chatPaneVisibleRef.current")).toBeLessThan(
      body.indexOf("activeChatChannelRef.current !== origin"),
    );
  });
});

/**
 * The same rule, one level down (#1890 B).
 *
 * A card now records the thread it was raised in, so a threaded settle's marker
 * carries a `parentId` — and `buildTimeline` folds every parented line into its
 * root's replies rather than rendering it in the channel timeline. The channel
 * being open is therefore no longer proof the operator can see the marker: with
 * the thread panel closed, or open on a sibling thread, the marker is nowhere
 * on screen while `isViewingTaskOrigin` would still report "watching it" on the
 * channel match alone. That is exactly the defect #1768's review established
 * the rule against, reintroduced by a change one layer away from it.
 *
 * A source-contract check for the same reason the block above is one.
 */
describe("a closed thread panel cannot suppress a completion toast (#1890)", () => {
  const chatView = read("views/ChatView.tsx");
  const appShell = read("components/app-shell.tsx");

  it("the shell tracks which thread panel is open", () => {
    expect(appShell).toContain("const openThreadRootRef = useRef<string | null>(null);");
    // Fed from `onChannelViewed`, whose effect in ChatView lists `openThreadId`
    // — so the ref tracks the panel rather than lagging one open behind it.
    expect(appShell).toContain("openThreadRootRef.current = openThreadId ?? null;");
    expect(chatView).toContain("openThreadId,");
  });

  it("clears the open thread when the company changes", () => {
    // Another company's thread roots are another namespace: a stale root left
    // behind by a switch could match an incoming marker by coincidence.
    expect(appShell).toContain(
      "activeChatChannelRef.current = null;\n    openThreadRootRef.current = null;",
    );
  });

  it("isViewingTaskOrigin defers to the thread only for a threaded marker", () => {
    const idx = appShell.indexOf("isViewingTaskOrigin: useCallback(");
    expect(idx).toBeGreaterThan(-1);
    const body = appShell.slice(idx, idx + 1600);
    // An unparented marker still renders inline, so the channel check alone
    // stays right for it — `root == null ||` is what keeps that case working.
    expect(body).toContain("return root == null || openThreadRootRef.current === root;");
    // And the channel must still be established first: a matching thread root
    // in the wrong channel proves nothing.
    expect(body.indexOf("activeChatChannelRef.current !== origin")).toBeLessThan(
      body.indexOf("openThreadRootRef.current === root"),
    );
  });
});
