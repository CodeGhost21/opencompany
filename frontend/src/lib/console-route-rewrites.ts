import { BOARD_LEDGER } from "@/lib/board-columns";
import { type View, VIEWS } from "@/lib/console-routes";
import { taskIdFromSegment } from "@/lib/task-route";
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
  if (head === "memory") return ["settings", "brain"];
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
  // `#/connections` predates the split into OAuth / MCP / Inference; the
  // accounts it named are the OAuth page.
  if (head === "connections") return ["settings", "oauth"];
  if (head === "oauth") return ["settings", "oauth"];
  if (head === "mcp") return ["settings", "mcp"];
  if (head === "people") return ["settings", "people"];
  // An empty hash is the normal console entry point and uses the router's
  // Overview fallback. Keep the head as the sub-page for every non-empty
  // unknown address so the explanation can identify what failed without
  // accepting it as a real view.
  if (head && !VIEWS.includes(head as View)) return ["not-found", head];
  return null;
};
