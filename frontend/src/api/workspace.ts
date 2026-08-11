// The live workspace API: the console reads and writes the company's real
// durable note tree through the host's `…/workspace` routes (REST, camelCase
// over the wire). Replaces the client-side `lib/workspace` localStorage stub —
// the same move `api/memory.ts` made for the Brain — so the operator and the
// agents finally look at one tree instead of two.
//
// Agents reach the same store through the `workspace_list` / `workspace_read` /
// `workspace_create` / `workspace_write` tools (issues #237, #551), which means
// a note an agent wrote or created shows up here and a note the operator writes
// here is readable by an agent on its next turn. Every node carries who created
// it and who last wrote it (issue #326) so the two are told apart on sight.

import type { OpenCompanyClient } from "./client";

/** Whether a node is a folder or a file. */
export type NodeKind = "folder" | "file";

/**
 * Who authored a node — the host's `WorkspaceOrigin` (issue #326).
 *
 * `seed` is neither: it shipped with the company bundle and was typed by
 * nobody. `agentId` is present exactly when `kind` is `"agent"`.
 */
export type WorkspaceOrigin =
  | { kind: "seed" }
  | { kind: "operator" }
  | { kind: "agent"; id: string };

/** The origin every node falls back to — what the host defaults a legacy node to. */
export const OPERATOR_ORIGIN: WorkspaceOrigin = { kind: "operator" };

/**
 * A short human label for an origin, or `null` for a plain operator note.
 *
 * Mirrors `ORIGIN_LABELS` in `api/memory.ts`, but returns `null` rather than
 * "Operator" for the operator case: in the Brain every row has an interesting
 * origin, whereas here the operator is the unremarkable default and badging it
 * would put a chip on nearly every note while saying nothing.
 */
export function originLabel(origin: WorkspaceOrigin | undefined): string | null {
  switch (origin?.kind) {
    case "agent":
      return `Agent · ${origin.id}`;
    case "seed":
      return "Seeded";
    default:
      return null;
  }
}

/**
 * One node in the workspace tree, as the host returns it.
 *
 * The tree read carries metadata only — `content` is always absent there, and a
 * body is fetched per file by {@link fetchFile}. `parentId` is normalized to
 * `null` at the workspace root (the host omits the field entirely).
 */
export interface FsNode {
  id: string;
  name: string;
  kind: NodeKind;
  /** null = workspace root. */
  parentId: string | null;
  /** Markdown body. Only ever set on a node built from a file read. */
  content?: string;
  /** Epoch-millis of the last update. */
  updatedAt: number;
  /** Who created this node. Never changes. */
  createdBy: WorkspaceOrigin;
  /** Who last wrote this node's body. A rename or move does not change it. */
  updatedBy: WorkspaceOrigin;
  /**
   * The media type of a **binary** node's payload, e.g. `image/png` (#553).
   *
   * Present only on a binary node, and the single test for one — the host omits
   * all three fields below on a prose note rather than sending nulls, so
   * `mime !== undefined` means "render or download this instead of editing it".
   */
  mime?: string;
  /** The payload's exact size in bytes. Host-computed. */
  size?: number;
  /** The payload's sha256, computed by the store from the stored bytes. */
  sha256?: string;
}

/** Whether a node holds bytes rather than prose. */
export function isBinary(node: Pick<FsNode, "mime">): boolean {
  return node.mime !== undefined && node.mime !== null;
}

/** One file's body plus the notes that link to it, from `GET …/workspace/file/{id}`. */
export interface WorkspaceFile {
  id: string;
  name: string;
  content: string;
  updatedAt: number;
  createdBy: WorkspaceOrigin;
  updatedBy: WorkspaceOrigin;
  /** Other files whose content links to this one via `[[name]]` — computed by the host. */
  backlinks: FsNode[];
}

/**
 * The wire shape: `parentId` is omitted at the root rather than sent as null,
 * and the two origins are optional purely for rollout skew — a console served
 * by a host that predates issue #326 gets neither field, and defaulting is
 * cheaper than a blank badge.
 */
interface FsNodeWire extends Omit<FsNode, "parentId" | "createdBy" | "updatedBy"> {
  parentId?: string | null;
  createdBy?: WorkspaceOrigin;
  updatedBy?: WorkspaceOrigin;
}

/**
 * Normalizes a node off the wire. The host omits `parentId` at the workspace
 * root (`skip_serializing_if = "Option::is_none"`), and every tree query in the
 * view keys off `parentId === null`, so an absent field becomes an explicit
 * null exactly once — here — rather than at each call site. The origins get the
 * same treatment against an older host: absent means operator, which is the
 * same default the Rust port applies to a node written before the field
 * existed.
 */
function normalize(node: FsNodeWire): FsNode {
  return {
    ...node,
    parentId: node.parentId ?? null,
    createdBy: node.createdBy ?? OPERATOR_ORIGIN,
    updatedBy: node.updatedBy ?? OPERATOR_ORIGIN,
  };
}

/** Every node in the company's workspace (metadata only; no bodies). */
export async function fetchTree(
  client: OpenCompanyClient,
  company: string | null,
): Promise<FsNode[]> {
  const nodes = await client.get<FsNodeWire[]>(`${client.scopeFor(company)}/workspace`);
  return nodes.map(normalize);
}

/** One file's content and its server-computed backlinks. 404s on a folder id. */
export async function fetchFile(
  client: OpenCompanyClient,
  company: string | null,
  id: string,
): Promise<WorkspaceFile> {
  const file = await client.get<
    Omit<WorkspaceFile, "backlinks" | "createdBy" | "updatedBy"> & {
      backlinks: FsNodeWire[];
      createdBy?: WorkspaceOrigin;
      updatedBy?: WorkspaceOrigin;
    }
  >(`${client.scopeFor(company)}/workspace/file/${encodeURIComponent(id)}`);
  return {
    ...file,
    createdBy: file.createdBy ?? OPERATOR_ORIGIN,
    updatedBy: file.updatedBy ?? OPERATOR_ORIGIN,
    backlinks: file.backlinks.map(normalize),
  };
}

/** Create a folder or file. The host mints the id and the timestamp. */
export async function createNode(
  client: OpenCompanyClient,
  company: string | null,
  input: { name: string; kind: NodeKind; parentId?: string | null; content?: string },
): Promise<FsNode> {
  const node = await client.post<FsNodeWire>(`${client.scopeFor(company)}/workspace`, input);
  return normalize(node);
}

/** Overwrite a file's content; returns the new last-updated stamp. */
export function writeFile(
  client: OpenCompanyClient,
  company: string | null,
  id: string,
  content: string,
): Promise<{ updatedAt: number }> {
  return client.put<{ updatedAt: number }>(
    `${client.scopeFor(company)}/workspace/file/${encodeURIComponent(id)}`,
    { content },
  );
}

/**
 * Rename and/or move a node, returning the authoritative node.
 *
 * The `parentId` contract is a double option and the distinction is load-bearing:
 * **omit** the key to leave the parent alone (a pure rename), and pass an
 * explicit `null` to move the node back to the workspace root. `JSON.stringify`
 * drops `undefined` properties but keeps `null`, so building the body
 * conditionally — as below — maps straight onto the server's
 * `Option<Option<String>>`. Spreading `{ parentId }` unconditionally would turn
 * every rename into a move-to-root.
 *
 * The move-cycle guard (a folder into its own descendant) is the store's, not
 * the console's: it answers 400 `invalid_request`.
 */
export async function renameMoveNode(
  client: OpenCompanyClient,
  company: string | null,
  id: string,
  changes: { name?: string; parentId?: string | null },
): Promise<FsNode> {
  const body: { name?: string; parentId?: string | null } = {};
  if (changes.name !== undefined) body.name = changes.name;
  if ("parentId" in changes) body.parentId = changes.parentId ?? null;
  const node = await client.patch<FsNodeWire>(
    `${client.scopeFor(company)}/workspace/${encodeURIComponent(id)}`,
    body,
  );
  return normalize(node);
}

/** Delete a node; folders go recursively. */
export function deleteNode(
  client: OpenCompanyClient,
  company: string | null,
  id: string,
): Promise<void> {
  return client.del<void>(`${client.scopeFor(company)}/workspace/${encodeURIComponent(id)}`);
}

/**
 * Upload a file of any kind (issue #553).
 *
 * `createNode` sends a JSON body, which cannot carry bytes, so an image or a
 * PDF has to arrive as `multipart/form-data` on its own route. The host — not
 * this function — decides whether the result is a note or a binary node: a file
 * that is typed as text *and* decodes as UTF-8 becomes a prose note, so an
 * uploaded `.md` keeps its editor, its backlinks and its diffable history.
 *
 * `fetch` directly rather than `client.post`: the shared request helper sets a
 * JSON content-type and `JSON.stringify`s its body, and a multipart upload must
 * let the browser set the boundary itself.
 */
export async function uploadFile(
  client: OpenCompanyClient,
  company: string | null,
  file: File,
  parentId?: string | null,
): Promise<FsNode> {
  const form = new FormData();
  form.append("file", file, file.name);
  if (parentId) form.append("parentId", parentId);
  const node = await client.postForm<FsNodeWire>(
    `${client.scopeFor(company)}/workspace/upload`,
    form,
  );
  return normalize(node);
}

/**
 * Fetch a binary node's payload as an object URL.
 *
 * A plain `<img src="…/workspace/blob/{id}">` would not work: the route needs
 * the bearer token the client holds, and an image element cannot carry one. So
 * the bytes are fetched through the authenticated client and wrapped in an
 * object URL the element can point at.
 *
 * **The caller must revoke the returned URL** when it is done with it —
 * `URL.revokeObjectURL` — or the blob stays resident for the life of the
 * document.
 */
export async function fetchBlobUrl(
  client: OpenCompanyClient,
  company: string | null,
  id: string,
): Promise<string> {
  const blob = await client.getBlob(
    `${client.scopeFor(company)}/workspace/blob/${encodeURIComponent(id)}`,
  );
  return URL.createObjectURL(blob);
}

/** A human-readable size, for the metadata a binary node shows instead of a body. */
export function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined) return "unknown size";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
