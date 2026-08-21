import { describe, expect, it } from "vitest";

import type { FsNode } from "@/api/workspace";
import { isSecretNode, isSecretPath, SECRETS_DIR, SECRETS_LABEL, SECRETS_REASON } from "@/lib/workspace";

/**
 * Issue #1465: which notes the company's agents cannot read.
 *
 * `secrets/` is the one part of the shared workspace tree the host keeps away
 * from agent tools — `is_agent_hidden_path` in
 * `src/company/workspace_scaffold.rs` drops it from the agent path index, from
 * agent writes and from agent search alike. The console knew nothing about it,
 * so the folder rendered like any other and the only statement of the rule was
 * a README seeded *inside* it.
 *
 * The rule is a **folder** rule and the host's is written on the first path
 * segment, so the cases that matter are the ones a casual check gets wrong: a
 * deep descendant, a differently-cased root, and a folder whose name merely
 * starts with the word.
 */

function node(partial: Partial<FsNode> & { id: string; name: string }): FsNode {
  return {
    kind: "file",
    parentId: null,
    updatedAt: 0,
    ...partial,
  } as FsNode;
}

const TREE: FsNode[] = [
  node({ id: "s", name: "secrets", kind: "folder" }),
  node({ id: "readme", name: "README.md", parentId: "s" }),
  node({ id: "keys", name: "Stripe keys.md", parentId: "s" }),
  node({ id: "sub", name: "vendors", kind: "folder", parentId: "s" }),
  node({ id: "deep", name: "Twilio.md", parentId: "sub" }),
  // An ordinary note, and a folder whose name merely contains the word.
  node({ id: "plays", name: "Playbooks", kind: "folder" }),
  node({ id: "runbook", name: "Runbook.md", parentId: "plays" }),
  node({ id: "lookalike", name: "secrets-old", kind: "folder" }),
  node({ id: "inside", name: "Archived key.md", parentId: "lookalike" }),
  // The host compares case-insensitively so a `Secrets` node cannot become an
  // agent-visible twin of the real one.
  node({ id: "shouty", name: "Secrets", kind: "folder" }),
  node({ id: "shoutychild", name: "Root password.md", parentId: "shouty" }),
];

describe("which notes the workspace marks as hidden from agents", () => {
  it("marks the folder and everything under it", () => {
    expect(isSecretNode(TREE, "s")).toBe(true);
    expect(isSecretNode(TREE, "readme")).toBe(true);
    expect(isSecretNode(TREE, "keys")).toBe(true);
    expect(isSecretNode(TREE, "sub")).toBe(true);
    // Deeper than the host scaffolds, and marked anyway: the rule is the
    // folder, so it holds for a path nothing seeded.
    expect(isSecretNode(TREE, "deep")).toBe(true);
  });

  it("cannot be stepped around by capitalising a letter", () => {
    // A false *negative* here is the whole failure mode of this issue: an
    // unmarked note that agents genuinely cannot read is one an operator files
    // a credential beside, believing the folder is ordinary.
    expect(isSecretNode(TREE, "shouty")).toBe(true);
    expect(isSecretNode(TREE, "shoutychild")).toBe(true);
  });

  it("leaves ordinary notes alone", () => {
    expect(isSecretNode(TREE, "runbook")).toBe(false);
    expect(isSecretNode(TREE, "plays")).toBe(false);
    // Only the root segment counts. `secrets-old/` is ordinary shared content
    // host-side, and a false positive here would promise privacy it does not
    // have — the more dangerous of the two mistakes.
    expect(isSecretNode(TREE, "lookalike")).toBe(false);
    expect(isSecretNode(TREE, "inside")).toBe(false);
    expect(isSecretNode(TREE, null)).toBe(false);
    expect(isSecretNode(TREE, "no-such-node")).toBe(false);
  });

  it("reads the same rule off a path, for the search hit list", () => {
    expect(isSecretPath("secrets/Stripe keys.md")).toBe(true);
    expect(isSecretPath("/secrets/Stripe keys.md")).toBe(true);
    expect(isSecretPath("secrets")).toBe(true);
    expect(isSecretPath("secrets/vendors/Twilio.md")).toBe(true);
    expect(isSecretPath("Secrets/Root password.md")).toBe(true);
    // The host trims before it strips slashes; so does this. A guard a stray
    // space defeats is not a guard.
    expect(isSecretPath("  /secrets/Stripe keys.md")).toBe(true);

    expect(isSecretPath("Playbooks/secrets/Notes.md")).toBe(false);
    expect(isSecretPath("secrets-old/Archived key.md")).toBe(false);
    expect(isSecretPath("Runbook.md")).toBe(false);
    expect(isSecretPath("")).toBe(false);
    expect(isSecretPath("/")).toBe(false);
  });

  it("agrees with the tree rule on the same node", () => {
    // The two predicates are the point of failure: one is asked by the tree and
    // the header, the other by the search list, and a disagreement would mean
    // the same note is marked private in one pane and shared in another.
    expect(isSecretNode(TREE, "keys")).toBe(isSecretPath("secrets/Stripe keys.md"));
    expect(isSecretNode(TREE, "deep")).toBe(isSecretPath("secrets/vendors/Twilio.md"));
    expect(isSecretNode(TREE, "inside")).toBe(isSecretPath("secrets-old/Archived key.md"));
    expect(isSecretNode(TREE, "runbook")).toBe(isSecretPath("Playbooks/Runbook.md"));
    expect(isSecretNode(TREE, "shoutychild")).toBe(isSecretPath("Secrets/Root password.md"));
  });

  it("names the folder the host names", () => {
    // If `SECRETS_ROOT` is ever renamed host-side, a folder the console calls
    // private is one the agents can read. This is the tripwire.
    expect(SECRETS_DIR).toBe("secrets");
  });

  it("states the audience, and states the other half of the rule", () => {
    // "Private" would describe who may open it in the console, which is
    // everyone. The fact is who cannot read it elsewhere.
    expect(SECRETS_LABEL).toBe("Hidden from agents");
    // And the reason has to say what the rest of the tree means, because the
    // operator being warned is the one choosing between two folders.
    expect(SECRETS_REASON).toContain("Everything outside it, they can read.");
  });
});
