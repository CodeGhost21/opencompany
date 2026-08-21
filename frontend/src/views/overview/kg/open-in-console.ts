// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Where a node on the graph lives in the rest of the console (issue #1308).
 *
 * The Overview graph draws every teammate, every open board card, every
 * workflow and every desk the company declares — and, until this, linked to
 * none of them. It was the console's landing page and its only terminal
 * surface: an operator could learn that a teammate had never run, and then had
 * to leave through the sidebar and find them again by hand.
 *
 * Nothing new is invented here. Every address below already exists and is
 * already the page that names the same thing; this is the lookup that says
 * which one, and it is pure so a test can assert the whole table without
 * rendering a graph.
 *
 * ## Why the node id is the input
 *
 * A node id already carries the host's own id for the thing it draws —
 * `emp:<agentId>`, `task:<taskId>`, `team:desk:<deskId>` (`model.ts`,
 * `adapter.ts`). Reading the destination off the id means the card cannot link
 * to a different record than the node it was opened from, which a second
 * id threaded through the component tree could quietly start doing.
 *
 * ## What deliberately has no destination
 *
 * - **A tool.** A grant is a string in `company.toml` (`mcp:*`, `workspace.*`),
 *   not a record with a page. The console has no address for one, and
 *   inventing a link to the MCP tab would point at a server that may have
 *   nothing to do with the grant — exactly the `grant_matches` guesswork
 *   `views/overview/README.md` forbids.
 * - **A workflow stage.** A stage is a node inside a saved graph. The flow has
 *   an address; the node within it does not.
 * - **The company core.** It is the page you are already on.
 */

/** A console address, and the words for the control that goes there. */
export interface ConsoleDestination {
  /** The `href` — a real hash link, so cmd-click and copy-link both work. */
  hash: string;
  /** What the control says. Names the destination, not the act of leaving. */
  label: string;
}

/** `desk:<id>` — the department id the adapter derives from a desk. */
const DESK_PREFIX = "desk:";

/**
 * The department id a desk was turned into, back to the desk's own id.
 *
 * The inverse of `departmentIdOfDesk` in `adapter.ts`, and it lives here rather
 * than there so the two are read together: `#/company/<deskId>` wants the id
 * the host serves, and the graph is holding the prefixed one.
 */
function deskIdOf(departmentId: string): string | null {
  return departmentId.startsWith(DESK_PREFIX)
    ? departmentId.slice(DESK_PREFIX.length) || null
    : null;
}

/**
 * The console address for a graph node, or `null` where none names it.
 *
 * `nodeId` is the graph's own id (`KGNode.id`). Ids are host-supplied, so every
 * segment is encoded — a desk called `Sales & Ops` or a task id with a slash
 * would otherwise write a different address than the one it names.
 */
export function destinationFor(nodeId: string): ConsoleDestination | null {
  if (nodeId.startsWith("emp:")) {
    const id = nodeId.slice("emp:".length);
    return id ? { hash: `#/team/${encodeURIComponent(id)}`, label: "Open teammate" } : null;
  }
  if (nodeId.startsWith("task:")) {
    const id = nodeId.slice("task:".length);
    return id ? { hash: `#/tasks/${encodeURIComponent(id)}`, label: "Open card" } : null;
  }
  if (nodeId.startsWith("flow:")) {
    const id = nodeId.slice("flow:".length);
    return id ? { hash: `#/workflows/${encodeURIComponent(id)}`, label: "Open workflow" } : null;
  }
  if (nodeId.startsWith("team:")) {
    const deskId = deskIdOf(nodeId.slice("team:".length));
    // A department the adapter did not build from a desk — `UNPLACED`, say —
    // has no desk behind it and therefore no page. It is not an error; it is
    // the honest answer, and it is why this returns `null` rather than
    // guessing at `#/company`.
    return deskId ? { hash: `#/company/${encodeURIComponent(deskId)}`, label: "Open desk" } : null;
  }
  // A human is a person who can sign in, and the console's page for that is the
  // people list. There is no per-person address to deep-link to, so this names
  // the list rather than pretending to name the row.
  if (nodeId.startsWith("person:")) {
    return { hash: "#/settings/people", label: "Open people" };
  }
  return null;
}

/**
 * The Brain, where the company's notes live.
 *
 * A note on the constellation is a real memory entry, but `#/memory` addresses
 * the surface rather than one entry, so this is a constant instead of a lookup
 * over the note's id.
 */
export const MEMORY_DESTINATION: ConsoleDestination = {
  hash: "#/memory",
  label: "Open Brain",
};
