/**
 * Where an open note says it lives (issue #1371).
 *
 * The header used to name the file and nothing else, and the tree did not
 * expand or scroll to reveal it — so a note reached from a search hit, a
 * wikilink or an `#/workspace/<id>` deep link appeared with no location at all.
 * In a tree five levels deep, with three notes called `README`, that answers
 * neither "which one is this?" nor "what sits beside it?".
 *
 * Two pure pieces carry the fix, and both are pinned here: which folders make
 * up the trail, and which ones have to be expanded for the row to exist.
 */
import { describe, expect, it } from "vitest";

import type { FsNode } from "@/api/workspace";
import { ancestorFolderIds, breadcrumbOf } from "@/lib/workspace";

function node(partial: Partial<FsNode> & { id: string; name: string }): FsNode {
  return { kind: "file", parentId: null, updatedAt: 0, ...partial } as FsNode;
}

// The shape that motivated the issue: standards/Engineering/Backend/Rust/API
// design/Pagination.md, five folders deep.
const TREE: FsNode[] = [
  node({ id: "std", name: "Standards", kind: "folder" }),
  node({ id: "eng", name: "Engineering", kind: "folder", parentId: "std" }),
  node({ id: "be", name: "Backend", kind: "folder", parentId: "eng" }),
  node({ id: "rust", name: "Rust", kind: "folder", parentId: "be" }),
  node({ id: "api", name: "API design", kind: "folder", parentId: "rust" }),
  node({ id: "page", name: "Pagination.md", parentId: "api" }),
  node({ id: "root-note", name: "README.md" }),
  node({ id: "pb", name: "Playbooks", kind: "folder" }),
  node({ id: "shallow", name: "Release checklist.md", parentId: "pb" }),
];

describe("breadcrumbOf", () => {
  it("says nothing for a note at the workspace root", () => {
    // An empty crumb rail would be chrome that says "top level" in the space a
    // real path would occupy.
    expect(breadcrumbOf(TREE, "root-note")).toEqual([]);
  });

  it("leaves the note itself out of its own trail", () => {
    // Its name is the heading right beside the trail; repeating it would spend
    // the widest crumb saying what the operator is looking straight at.
    const names = breadcrumbOf(TREE, "shallow").map((c) => c?.name);
    expect(names).toEqual(["Playbooks"]);
  });

  it("keeps the root and the last two folders when the trail is too long", () => {
    // The half that matters. Truncating a path *string* ellipsises the tail, so
    // every note under standards/Engineering/… renders an identical prefix and
    // the discriminating end is exactly what is thrown away.
    const crumbs = breadcrumbOf(TREE, "page");
    expect(crumbs.map((c) => c?.name ?? "…")).toEqual(["Standards", "…", "Rust", "API design"]);
  });

  it("elides with an explicit gap rather than by dropping folders", () => {
    // A shortened path that does not admit it is shortened reads as the whole
    // truth. The `null` is what the ellipsis crumb is rendered from.
    expect(breadcrumbOf(TREE, "page")).toContain(null);
  });

  it("does not elide a trail that already fits", () => {
    expect(breadcrumbOf(TREE, "page", 5).map((c) => c?.name)).toEqual([
      "Standards",
      "Engineering",
      "Backend",
      "Rust",
      "API design",
    ]);
  });
});

describe("ancestorFolderIds", () => {
  it("names every folder that must be expanded for the row to exist", () => {
    expect(ancestorFolderIds(TREE, "page")).toEqual(["std", "eng", "be", "rust", "api"]);
  });

  it("excludes the node itself, even when it is a folder", () => {
    // Revealing a folder means showing its row, not opening it.
    expect(ancestorFolderIds(TREE, "api")).toEqual(["std", "eng", "be", "rust"]);
  });

  it("expands nothing for a root node", () => {
    expect(ancestorFolderIds(TREE, "root-note")).toEqual([]);
  });

  it("finds nothing when the tree has not loaded yet", () => {
    // The deep-link bug in one line: `open()` ran before the tree arrived, so
    // this walked an empty array and expanded nothing. The reveal now re-runs
    // when `nodes` changes, and this is the first pass it has to survive.
    expect(ancestorFolderIds([], "page")).toEqual([]);
  });
});
