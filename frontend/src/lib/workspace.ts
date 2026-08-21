// Pure tree helpers for the team's workspace — a real, durable file tree of
// folders and markdown notes that lives on the **host**, in the company's
// `WorkspaceStore`. The console reads and writes it over the `…/workspace`
// routes in `@/api/workspace`; the company's agents read and write the same
// tree through their workspace tools, so operator and agents share one surface.
//
// Nothing here persists anything. These are the derivations the view needs on
// top of a tree it has already fetched (ordering, ancestry, wiki-link title
// resolution). The one localStorage touch left is the *migration* pair at the
// bottom, which exists solely to rescue notes typed into the retired
// client-side scratchpad and is expected to become a no-op once every browser
// has been through it once.

export type { FsNode } from "@/api/workspace";

import { type LocalScope, scopedKeyAdoptingLegacy } from "@/connections/types";

import type { FsNode, RepairOutcome } from "@/api/workspace";

/* ---- queries ---- */

export function childrenOf(nodes: FsNode[], parentId: string | null): FsNode[] {
  return nodes
    .filter((x) => x.parentId === parentId)
    .sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "folder" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
}

export function nodeById(nodes: FsNode[], id: string | null): FsNode | undefined {
  return id ? nodes.find((x) => x.id === id) : undefined;
}

/**
 * A file's display title — its name without the markdown extension.
 *
 * Known divergence: this strips `.md`, `.markdown` and `.txt`, while the host's
 * `link_target` (which computes backlinks) strips only `.md`. So a `[[link]]`
 * to a `.txt` note styles as resolved here but is not counted as a backlink
 * server-side. Cosmetic, and deliberately left alone — the seeder only ever
 * creates `.md` notes, and narrowing this would silently restyle existing links.
 */
export function titleOf(node: FsNode): string {
  return node.name.replace(/\.(md|markdown|txt)$/i, "");
}

/** Resolve an Obsidian-style `[[wiki link]]` target to a file, by title. */
export function fileByTitle(nodes: FsNode[], target: string): FsNode | undefined {
  const want = target.trim().toLowerCase();
  return nodes.find((x) => x.kind === "file" && titleOf(x).toLowerCase() === want);
}

/**
 * The folder whose contents are written by code and never by hand.
 *
 * Kept in step with `DERIVED_DIR` in `src/ledger/spec.rs`. If the host ever
 * renames it, a write refused here would be one the console still offered.
 */
export const DERIVED_DIR = "derived";

/**
 * Whether the node at `id` is one the host will refuse to let a person write
 * (issue #1222).
 *
 * A **folder** rule, mirroring `is_derived_path` in `src/ledger/derived.rs`
 * character for character — including the case-insensitive comparison, because
 * "a guard that can be stepped around by capitalising a letter is not a guard".
 * That module's header argues why the invariant is the folder and not the file:
 * a per-file rule fails open, because a ledger declared next week renders a file
 * no list has heard of.
 *
 * The console applies the same rule rather than reading a flag because the wire
 * carries none — `GET …/workspace` returns no `readOnly` — and because a rule
 * evaluated from the same fact on both sides cannot drift the way two lists
 * would. It is what lets the pane refuse *before* the typing instead of after,
 * which is the whole of #1222.
 */
export function isDerivedNode(nodes: FsNode[], id: string | null): boolean {
  const ancestry = pathOf(nodes, id);
  const head = ancestry[0];
  if (!head) return false;
  return head.kind === "folder" && head.name.trim().toLowerCase() === DERIVED_DIR;
}

/**
 * The same rule as {@link isDerivedNode}, applied to a **path string** rather
 * than a tree (issue #1377).
 *
 * The search hit list has no tree to walk — it is a flat list of results, and
 * the nodes it names may sit in folders the explorer has never expanded. But
 * every hit carries its own `path`, so the folder rule can be read straight off
 * that. Kept beside `isDerivedNode` and written from the same `DERIVED_DIR`
 * constant, so the two cannot drift into disagreeing about which files a person
 * may write.
 *
 * Leading slashes are tolerated for the same reason `is_derived_path` tolerates
 * them: the guard must not be steppable around by a formatting difference.
 */
export function isDerivedPath(path: string): boolean {
  const head = path.trim().replace(/^\/+/, "").split("/")[0];
  return head !== undefined && head.trim().toLowerCase() === DERIVED_DIR;
}

/**
 * What the tree, the search list and the note header all call a `derived/`
 * file (issue #1377).
 *
 * One phrase in three places on purpose. "Read only" — what the header chip
 * used to say — reports that an edit is unwelcome without saying *why*, and a
 * rule with no reason behind it is the kind of rule people work around or file
 * as a bug. "Written by a ledger" is the reason, and it is short enough to fit
 * a tree row, a search hit and a header chip alike.
 */
export const DERIVED_LABEL = "Written by a ledger";

/**
 * The long form of {@link DERIVED_LABEL} — what happens if you edit it anyway,
 * and where the edit actually belongs.
 *
 * Lives here rather than in `WorkspaceView` because the search list is a
 * sibling module that would otherwise have to import from its own parent.
 */
export const DERIVED_REASON =
  "This file is written by a ledger and re-derived on every write to it — " +
  "an edit here would be erased. Change it on the Ledgers page instead.";

/**
 * The one folder in this shared tree the company's agents cannot reach.
 *
 * Kept in step with `SECRETS_ROOT` in `src/company/workspace_scaffold.rs`. The
 * host scaffolds it on every boot and excludes it from the agent path index,
 * from agent writes and from agent search — listing, reading, searching and
 * writing, all four. If the host ever renames it, a folder the console called
 * private would be one the agents could read.
 *
 * The sibling of {@link DERIVED_DIR}, and the exact inverse of it (issue
 * #1465). `derived/` is "agents write this, you do not"; `secrets/` is "you
 * write this, agents do not". To an operator scanning one tree they are the
 * same kind of fact, so they are marked the same way and their strings live
 * side by side here.
 */
export const SECRETS_DIR = "secrets";

/**
 * Whether the node at `id` is the `secrets/` root or something beneath it
 * (issue #1465).
 *
 * A **folder** rule, mirroring `is_agent_hidden_path` in
 * `src/company/workspace_scaffold.rs` — first path segment, compared
 * case-insensitively so a `Secrets` node cannot become an accidental
 * agent-visible twin, and by segment rather than string prefix so a
 * `secrets-old/` remains ordinary shared content.
 *
 * The console evaluates the same rule rather than reading a flag because the
 * wire carries none — `GET …/workspace` returns no `hiddenFromAgents` — and
 * because a rule read off the same fact on both sides cannot drift the way two
 * lists would. Written exactly like {@link isDerivedNode}, on purpose.
 */
export function isSecretNode(nodes: FsNode[], id: string | null): boolean {
  const ancestry = pathOf(nodes, id);
  const head = ancestry[0];
  if (!head) return false;
  return head.kind === "folder" && head.name.trim().toLowerCase() === SECRETS_DIR;
}

/**
 * The same rule as {@link isSecretNode}, applied to a **path string** rather
 * than a tree (issue #1465).
 *
 * The search hit list replaces the tree in the explorer pane, so it has no
 * ancestry to walk — but every hit carries its own `path`. Written from the
 * same {@link SECRETS_DIR} constant so the two cannot disagree about which
 * notes an agent can read.
 *
 * Leading slashes are tolerated and the string is trimmed first, because the
 * host's `is_agent_hidden_path` does both: a guard a stray space defeats is not
 * a guard.
 */
export function isSecretPath(path: string): boolean {
  const head = path.trim().replace(/^\/+/, "").split("/")[0];
  return head !== undefined && head.trim().toLowerCase() === SECRETS_DIR;
}

/**
 * What the tree, the search list and the note header all call a note under
 * `secrets/` (issue #1465).
 *
 * One phrase in three places, for the same reason {@link DERIVED_LABEL}
 * is: three surfaces describing one rule in three wordings is how an operator
 * comes to believe there are three rules. It states the audience rather than a
 * permission — "Private" would say who may not open it in the console, which is
 * nobody; the fact is who cannot read it *elsewhere*.
 */
export const SECRETS_LABEL = "Hidden from agents";

/**
 * The long form of {@link SECRETS_LABEL} — the whole of the rule, including the
 * half that is about the rest of the tree.
 *
 * The second sentence is the one that matters. Until this shipped, the only
 * statement of the rule was a `README.md` seeded *inside* `secrets/`, which you
 * read only if you already went looking — and the operator who most needs it is
 * the one deciding where to put a credential, looking at a tree in which
 * nothing said one folder was different.
 */
export const SECRETS_REASON =
  "The company's agents cannot list, read, search or write anything under " +
  "`secrets/`. Everything outside it, they can read.";

/**
 * A move that changes who can read a note, and what to say about it
 * (issue #1465).
 *
 * `Move to…` is the only control in the console that changes a note's audience,
 * and it changed it in **both** directions with no more than a "moved" toast.
 * Moving into `secrets/` revokes agent access; moving out grants it. Returns
 * `null` for every other move, which is nearly all of them — a warning shown on
 * moves that change nothing is a warning nobody reads on the move that does.
 *
 * The copy lives here rather than in the dialog so it can be pinned by a test
 * without mounting `WorkspaceView`, and so it sits beside the predicate whose
 * answer it describes.
 */
export type MoveAudienceChange = "hidden" | "exposed";

export interface MoveAudienceWarning {
  /** Which direction the audience moves — the two are not equally dangerous. */
  change: MoveAudienceChange;
  /** The consequence, in one sentence, in the console's own voice. */
  title: string;
  /** What is moving, where, and what to check before confirming. */
  body: string;
  /** The confirm button's own words, so it never reads a bare "Move". */
  confirmLabel: string;
}

export function moveAudienceWarning(
  nodes: FsNode[],
  node: FsNode,
  destId: string | null,
): MoveAudienceWarning | null {
  const wasSecret = isSecretNode(nodes, node.id);
  // The workspace root is never `secrets/`, so a `null` destination is always
  // agent-visible.
  const willBeSecret = destId !== null && isSecretNode(nodes, destId);
  if (wasSecret === willBeSecret) return null;

  // A folder move takes every note under it in the same call, so the sentence
  // has to say so — "this note" would understate a move of thirty. The title
  // is the line that gets read, so it says it too: a heading that promises one
  // note above a paragraph describing a subtree is the wrong half to be vague
  // in.
  const folder = node.kind === "folder";
  const what = folder ? "this folder and everything in it" : "this note";
  // The title needs the short form — it is a heading, not a sentence.
  const subject = folder ? "this folder" : "this note";
  const name = titleOf(node);

  return willBeSecret
    ? {
        change: "hidden",
        title: `Agents will no longer be able to read ${subject}.`,
        body:
          `“${name}” moves into ${SECRETS_DIR}/, where ${what} stops being ` +
          "visible to the company's agents — they cannot list, read, search or write it.",
        confirmLabel: "Move into secrets",
      }
    : {
        change: "exposed",
        title: `Agents will be able to read ${subject}.`,
        body:
          `“${name}” moves out of ${SECRETS_DIR}/, and ${what} becomes part of the ` +
          "shared tree every agent can read and search. Check it holds no credentials first.",
        confirmLabel: "Move out of secrets",
      };
}

/**
 * A **rename** that changes who can read a note, and what to say about it
 * (issue #1465).
 *
 * The hole `moveAudienceWarning` left. The host's rule is `is_agent_hidden_path`
 * in `src/company/workspace_scaffold.rs` — the *first path segment*, compared to
 * `secrets` — and a rename of a root folder rewrites that segment for its whole
 * subtree. Renaming `secrets/` to `vault/` therefore hands every note under it
 * to every agent in the company, and the host allows it: a `PATCH
 * …/workspace/<id>` with `{"name":"vault"}` answers `200`, because nothing on
 * that path is guarded the way `derived/` is by `DerivedGuardWorkspace`.
 *
 * Allowing it is defensible — the operator owns `secrets/`, and an operator who
 * means to retire the folder is entitled to. Doing it in silence is not, and
 * silence is exactly what the move warning exists to end. So a rename that
 * crosses the boundary is put through the same panel, in the same words.
 *
 * Only a **root** node can cross it. A rename deeper in the tree cannot touch
 * the first segment — that belongs to an ancestor — so this returns `null` for
 * every one of them, which is nearly all renames.
 *
 * Not reachable by an agent, for the record: agent workspace tools address the
 * tree through `PathIndex::build_for_agent`, which drops hidden nodes from both
 * the path and the id map, and the move tool confines destinations to the
 * agent's own `Agents/<id>/` home. This is an operator-surface rule, and this is
 * the operator surface.
 */
export function renameAudienceWarning(
  nodes: FsNode[],
  node: FsNode,
  nextName: string,
): MoveAudienceWarning | null {
  const wasSecret = isSecretNode(nodes, node.id);
  // A nested node's first segment is its root ancestor's name, which a rename
  // here does not touch.
  const willBeSecret =
    node.parentId === null ? nextName.trim().toLowerCase() === SECRETS_DIR : wasSecret;
  if (wasSecret === willBeSecret) return null;

  const folder = node.kind === "folder";
  const what = folder ? "this folder and everything in it" : "this note";
  const subject = folder ? "this folder" : "this note";
  const name = titleOf(node);
  const next = nextName.trim();

  return willBeSecret
    ? {
        change: "hidden",
        title: `Agents will no longer be able to read ${subject}.`,
        body:
          `Renaming “${name}” to “${next}” puts it at the top of ${SECRETS_DIR}/, where ` +
          `${what} stops being visible to the company's agents — they cannot list, read, ` +
          "search or write it.",
        confirmLabel: "Rename into secrets",
      }
    : {
        change: "exposed",
        title: `Agents will be able to read ${subject}.`,
        body:
          `${SECRETS_DIR}/ is hidden from agents by its name. Renaming it to “${next}” ends ` +
          `that: ${what} becomes part of the shared tree every agent can read and search. ` +
          "Check it holds no credentials first.",
        confirmLabel: "Rename out of secrets",
      };
}

/** Ancestor folders (root → current), for breadcrumbs. */
export function pathOf(nodes: FsNode[], id: string | null): FsNode[] {
  const path: FsNode[] = [];
  let cur = nodeById(nodes, id);
  while (cur) {
    path.unshift(cur);
    cur = nodeById(nodes, cur.parentId);
  }
  return path;
}

/**
 * One step of a note's location, for the header breadcrumb (issue #1371).
 *
 * `null` is the elided middle — a crumb the trail could not fit, rendered as an
 * ellipsis rather than dropped, so the operator can see that the path is longer
 * than what is shown instead of reading a shortened path as the whole truth.
 */
export type Crumb = FsNode | null;

/**
 * The folders a note sits inside, shortened to at most `max` of them.
 *
 * The note itself is **not** in the trail: its name is already the heading beside
 * it, and repeating it would spend the widest crumb saying what the operator is
 * looking straight at.
 *
 * When the trail is too long, the **root and the last two folders** survive and
 * the middle collapses. That split is the useful one: the root says which part
 * of the company this belongs to, and the last two say what it sits next to.
 * Truncating the string instead — which is what `truncate` on a single span did
 * — ellipsises the *tail*, so every note under `Standards/Engineering/…` renders
 * the identical prefix and the discriminating end is exactly what is thrown away.
 */
export function breadcrumbOf(nodes: FsNode[], id: string | null, max = 3): Crumb[] {
  const folders = pathOf(nodes, id).slice(0, -1);
  if (folders.length <= max) return folders;
  // `max - 2` leading crumbs, the ellipsis, then the final two.
  return [...folders.slice(0, Math.max(max - 2, 1)), null, ...folders.slice(-2)];
}

/**
 * The ids of every folder a node is nested in — what has to be expanded for the
 * node's own row to exist in the tree (issue #1371).
 *
 * Excludes the node itself even when it is a folder: revealing a folder means
 * showing its row, not opening it.
 */
export function ancestorFolderIds(nodes: FsNode[], id: string | null): string[] {
  return pathOf(nodes, id)
    .slice(0, -1)
    .filter((node) => node.kind === "folder")
    .map((node) => node.id);
}

/** Ids of a node and all its descendants (for delete / move guards). */
export function subtreeIds(nodes: FsNode[], id: string): Set<string> {
  const ids = new Set<string>([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const node of nodes) {
      if (node.parentId && ids.has(node.parentId) && !ids.has(node.id)) {
        ids.add(node.id);
        grew = true;
      }
    }
  }
  return ids;
}

/**
 * Descendant counts for `id`, split by kind and excluding the node itself —
 * for a delete confirmation to say "3 notes and 1 folder" rather than a bare
 * "everything inside it" (issue #1255). Built on {@link subtreeIds}, which
 * includes `id` itself; this one deliberately does not.
 */
export function subtreeCounts(nodes: FsNode[], id: string): { files: number; folders: number } {
  const ids = subtreeIds(nodes, id);
  ids.delete(id);
  let files = 0;
  let folders = 0;
  for (const node of nodes) {
    if (!ids.has(node.id)) continue;
    if (node.kind === "file") files++;
    else folders++;
  }
  return { files, folders };
}

/**
 * The tree as it stands after a duplicate-folder repair (issue #759).
 *
 * The host's answer is a list of *changes*, not a new tree, so this replays them
 * onto the copy the tab already has — the same thing the delete and move
 * handlers do, and for the same reason: a full refetch would discard the open
 * note's unsaved draft.
 *
 * Order matters. Relocations are applied first, then a removed folder takes
 * whatever is still filed under it. That is not defensive padding: the host only
 * ever deletes a folder it has just proved empty, but *this* copy of the tree can
 * be stale — an agent may have created a note in it since the last fetch — and
 * dropping the folder while keeping the phantom child would leave a row hanging
 * off a parent that no longer exists.
 */
export function applyRepair(nodes: FsNode[], outcome: RepairOutcome): FsNode[] {
  const relocated = new Map<string, string>();
  for (const folder of outcome.folders) {
    for (const child of folder.moved) relocated.set(child.id, folder.intoId);
  }
  const moved = nodes.map((node) => {
    const parentId = relocated.get(node.id);
    return parentId === undefined ? node : { ...node, parentId };
  });

  const gone = new Set<string>();
  for (const folder of outcome.folders) {
    if (folder.removed) for (const id of subtreeIds(moved, folder.id)) gone.add(id);
  }
  return moved.filter((node) => !gone.has(node.id));
}

/** Notes get a markdown extension unless they already carry a known one. */
export function ensureMdExt(name: string): string {
  return /\.(md|markdown|txt)$/i.test(name) ? name : `${name}.md`;
}

/* ---- migration off the retired localStorage scratchpad ---- */

/** Where the pre-connection console kept this company's scratchpad. */
const legacyWorkspaceKey = (scope: LocalScope) => `oc-workspace:${scope.company ?? "single"}`;

const KEY = (scope: LocalScope) =>
  scopedKeyAdoptingLegacy("oc-workspace", scope, legacyWorkspaceKey(scope));

/**
 * Ids the *bundled seed* used. These four nodes were app source shipped to
 * every browser — marketing copy about a "Spring launch" campaign that belonged
 * to no real company. They carry zero user information, so they are dropped
 * rather than imported: pushing them into the host would inject invented
 * content into every company's genuine workspace.
 */
const SEED_ID_PREFIX = "seed-";

/**
 * Notes a user actually typed into the old client-side workspace, if any.
 *
 * Returns only nodes the *user* authored (the retired `addFile`/`addFolder`
 * minted `fs-…` ids); bundled seed nodes are filtered out. An empty array means
 * there is nothing worth rescuing — either the key is absent, unparseable, or
 * holds nothing but the seed.
 */
export function readLegacyLocalNodes(scope: LocalScope): FsNode[] {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(KEY(scope));
  } catch {
    return [];
  }
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  return parsed.filter(
    (node): node is FsNode =>
      Boolean(node) &&
      typeof (node as FsNode).id === "string" &&
      typeof (node as FsNode).name === "string" &&
      ((node as FsNode).kind === "file" || (node as FsNode).kind === "folder") &&
      !(node as FsNode).id.startsWith(SEED_ID_PREFIX),
  );
}

/** Whether the legacy key exists at all (so a seed-only key can be swept). */
export function hasLegacyLocal(scope: LocalScope): boolean {
  try {
    return localStorage.getItem(KEY(scope)) !== null;
  } catch {
    return false;
  }
}

/**
 * Drop the retired scratchpad for this company.
 *
 * Removes the **origin** as well as this connection's adopted copy. Unlike the
 * other adopted keys — where two connections each inheriting the old tour
 * progress is the intended reading — this one is *consumed* by the import. Left
 * behind, the next connection to look would adopt it and offer the same notes
 * again, and the operator would import them twice with nothing saying so.
 */
export function clearLegacyLocal(scope: LocalScope): void {
  try {
    localStorage.removeItem(KEY(scope));
    localStorage.removeItem(legacyWorkspaceKey(scope));
  } catch {
    /* storage unavailable — nothing to clear */
  }
}
