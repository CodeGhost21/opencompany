import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * What pinning the Room rail costs `ChatView`, and the three things that pay it
 * (issue #2130).
 *
 * The rail is painted in the sidebar on every section now, and it is portalled
 * out of `ChatView` — so the shell keeps that view mounted on every route and
 * hands it `routeOpen`. Three consequences follow, and each one was found the
 * hard way or is one edit from being lost:
 *
 *   1. **A mounted view must not steer the route.** `ChatView` restores the
 *      remembered channel into the hash whenever the hash names no channel.
 *      Mounted everywhere, that fires on `#/workflows` and `#/connections` too
 *      — which name no second segment — and navigates the operator straight
 *      back out of the section they just opened. Found in a browser: clicking
 *      **Flows** landed on `#/chat/main`.
 *   2. **A mounted view must not paint over the page.** The transcript, its
 *      header and the members pane render only when `routeOpen`.
 *   3. **The dialogs must NOT be gated with them.** Their triggers are painted
 *      in the sidebar — the rail's "+" and its "New message" pencil — so they
 *      have to open from Company and Flows exactly as from Room. A portal moves
 *      the DOM node, not the component tree, which is what makes that possible
 *      at all, and putting them inside the `routeOpen` gate would quietly undo
 *      it: the trigger would still be on screen and would open nothing.
 *
 * A source guard rather than a render test, in the idiom of
 * `responsive-two-rail-band`: (1) needs a hash router and a mounted shell to
 * reproduce, and (3) is invisible below the level of the whole shell — the
 * dialogs are correct read on their own and wrong only once you know where
 * their triggers are painted. `section-rail-layout.test.ts` covers what the
 * rail and the content rail render.
 */

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("ChatView, mounted off its own route", () => {
  const chatView = read("views/ChatView.tsx");

  it("refuses to restore the remembered channel while another section is open", () => {
    // The guard, and that it is the FIRST thing the effect does — after the
    // `if (sub)` line it is already too late for a bare `#/workflows`.
    const effect = chatView.slice(chatView.indexOf("const restoredFor = useRef"));
    const guard = effect.indexOf("if (!routeOpen) return;");
    const subCheck = effect.indexOf("if (sub) {");
    expect(guard).toBeGreaterThan(-1);
    expect(subCheck).toBeGreaterThan(-1);
    expect(guard).toBeLessThan(subCheck);
    // And it is a dependency, so arriving on Room re-runs the restore rather
    // than skipping it for the life of the mount.
    expect(effect.slice(0, effect.indexOf("}, [") + 200)).toContain(
      "[company, routeOpen, scope, sub, onNavigate]",
    );
  });

  it("renders the transcript only on its own route", () => {
    expect(chatView).toContain("{routeOpen && (");
  });

  it("keeps both sidebar-triggered dialogs outside that gate", () => {
    // Everything after the gate closes is what stays mounted off Room. Both
    // dialogs have to be in it: their triggers are the rail's "+" and its "New
    // message" pencil, which are painted in the sidebar on every section.
    const gate = chatView.indexOf("{routeOpen && (");
    const gateClose = chatView.indexOf("\n      )}\n", gate);
    const dialog = chatView.search(/<ChannelCreateDialog[\s/>]/);
    expect(gate).toBeGreaterThan(-1);
    expect(gateClose).toBeGreaterThan(gate);
    expect(dialog, "ChannelCreateDialog must mount after the routeOpen gate closes").toBeGreaterThan(
      gateClose,
    );
    // `NewMessageDialog` mounts inside `ChannelRail`, which is the portalled
    // node itself — so it rides along with the rail rather than needing a place
    // in this tail. Asserted from the rail's side so the pairing is stated
    // somewhere rather than assumed.
    expect(read("views/chat/ChannelRail.tsx")).toMatch(/<NewMessageDialog[\s/>]/);
  });

  it("is mounted by the shell unconditionally, with routeOpen as the only gate", () => {
    const shell = read("components/app-shell.tsx");
    // The regression this replaces: `{view === "chat" && <ChatView …/>}`, which
    // unmounted the rail's owner the moment the operator left Room.
    expect(shell).not.toMatch(/\{view === "chat" && \(\s*<ChatView/);
    expect(shell).toContain('routeOpen={view === "chat"}');
    // And it is handed the CHAT segment, not the current view's. On
    // `#/connections/mcp` the live `sub` is `mcp`, which chat would resolve as
    // a channel id.
    expect(shell).toContain('sub={view === "chat" ? sub : chatSub}');
  });
});
