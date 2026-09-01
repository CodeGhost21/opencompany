import { BOARD_LEDGER } from "@/lib/board-columns";
import { type View, VIEWS } from "@/lib/console-routes";
import { taskIdFromSegment } from "@/lib/task-route";
import { isConnectionPage } from "@/views/connection-pages";
import { isSettingsPage } from "@/views/settings-pages";

/**
 * Resolves retired and unknown top-level addresses before the generic router
 * validates them.
 *
 * Retired routes have a real replacement. An address with no known route gets
 * a named explanation instead of silently looking like Overview (issue #1417).
 */
export const REWRITE_RETIRED = (
  head: string,
  sub: string | null,
): [View, string | null] | null => {
  if (head === "tasks" && taskIdFromSegment(sub) === null) return ["ledgers", BOARD_LEDGER];
  // `#/work` is where the "Work" nav row points in shared and bookmarked links;
  // the board itself lives at `#/ledgers/tasks`, so send the alias there rather
  // than let it fall through to not-found (issue #1797). Bare only: the Work
  // surface's real sub-pages are addressed under `#/ledgers/...` (for example
  // `#/ledgers/manage`), never under `#/work/...`, so a `sub` here names
  // nothing this alias can answer — swallowing it would silently show the
  // board for an address that meant something else. Leave it to fall through
  // to the same not-found handling any other unrecognized head gets.
  if (head === "work" && !sub) return ["ledgers", BOARD_LEDGER];
  // Both of the brain's old addresses. `#/memory` was the surface's first name;
  // `#/settings/brain` is where it lived while it was a settings sub-page. It
  // has its own nav row now, so both are rewritten onto it rather than left to
  // render a settings rail around a page that is no longer in one.
  if (head === "memory") return ["brain", null];
  if (head === "settings" && sub === "brain") return ["brain", null];
  // Apps and MCP Servers left the settings rail for the Connections section.
  // Both of their settings addresses are rewritten onto it, and so is
  // `#/settings/connections` — the address the pre-split Connections page had,
  // which named no `SETTINGS_PAGES` id and therefore landed an operator on
  // General while looking like a link that worked. It was still live in four
  // places in the setup flow when this section was built.
  if (head === "settings" && sub === "oauth") return ["connections", "apps"];
  if (head === "settings" && sub === "mcp") return ["connections", "mcp"];
  if (head === "settings" && sub === "connections") return ["connections", null];
  // Settings owns a fixed table of sub-pages, unlike the entity ids beneath
  // Team and Workspace. Do not render General under an address that names no
  // page: a bookmark or shared link must say where it actually lands.
  if (head === "settings" && sub !== null && !isSettingsPage(sub)) return ["settings", "general"];
  // Bare `#/team` is the Company page now (issue #1141). It rendered the
  // teammate card grid from a route with no nav entry, so nobody arrived at it;
  // the grid is Company's Cards half, and leaving `#/team` answering as well
  // would leave two live addresses drawing one grid with no relationship
  // between them. A named teammate is untouched — `#/team/<agentId>` is the
  // detail sub-page (issue #264), it is what the org chart's rows and the chat
  // pane's chips link to, and it is deliberately a page so it can be linked.
  if (head === "team" && !sub) return ["company", null];
  // `#/connections` is a real address again. It predates the split into OAuth /
  // MCP / Inference, then spent that time rewritten onto the OAuth page; it now
  // names the section those two pages live in, which is closer to what it
  // always meant. Bare is left alone — the section defaults to Apps the way
  // `#/finances` defaults to its Overview. A segment that names no sub-page is
  // repaired rather than swallowed, for the reason the Settings branch above
  // gives: an address that quietly shows a different page looks like a link
  // that worked.
  if (head === "connections" && sub !== null && !isConnectionPage(sub)) {
    return ["connections", null];
  }
  if (head === "oauth") return ["connections", "apps"];
  if (head === "mcp") return ["connections", "mcp"];
  if (head === "people") return ["settings", "people"];
  // Observatory has a row on the Settings rail and no row in the sidebar, so
  // `#/settings/observatory` is an address an operator can now arrive at — and
  // it must not be where the surface lives. The Observatory reads four query
  // keys of its own straight off `window.location`, keyed on the hash's head
  // being `observatory` (`views/observatory/hash.ts`), so under `#/settings/…`
  // its analytics tab and its agent/turn selection stop being addressable. The
  // rail row is a doorway; this is the door.
  if (head === "settings" && sub === "observatory") return ["observatory", null];
  // An empty hash is the normal console entry point and uses the router's
  // Overview fallback. Keep the head as the sub-page for every non-empty
  // unknown address so the explanation can identify what failed without
  // accepting it as a real view.
  if (head && !VIEWS.includes(head as View)) return ["not-found", head];
  return null;
};
