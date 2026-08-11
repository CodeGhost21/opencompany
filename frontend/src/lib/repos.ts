// The repositories card's one derivation (issue #245, agent half): given what
// is bound and who is granted, which half of the setup is missing?
//
// A pure function rather than three conditions inline in the component, because
// the thing worth getting right here is not how it renders — it is *which* of
// four states an operator is in, and three of them look identical on screen
// (nothing happens). A branch that silently picks the wrong one tells an
// operator their setup is fine when no agent can read a line of code.

import type { Repo } from "@/api/repos";

/** What the card should tell the operator about their setup. */
export type RepoNotice =
  /** The company grants `repo` and has bound nothing to read. */
  | "granted-unbound"
  /** Repositories are bound and no roster agent holds the `repo` grant. */
  | "bound-ungranted"
  /** Bound and readable, or nothing configured either way — say nothing. */
  | null;

/** What the card knows. */
export interface RepoCoverageInput {
  /** The bindings the host returned. */
  repos: Repo[];
  /** The roster agents whose effective grants include `repo`. */
  grantedAgents: string[];
  /**
   * Whether the company manifest explicitly grants `repo`.
   *
   * `undefined` is **unknown** — an older host, or a `/capabilities` read that
   * failed — and is deliberately not the same as `false`. A card that told an
   * operator "you have not granted this" on the strength of a failed request
   * would send them to edit a manifest that is already correct.
   */
  repoGranted: boolean | undefined;
}

/** The card's rendering decision. */
export interface RepoCoverage {
  /** Which mismatch to name, if any. */
  notice: RepoNotice;
  /** Who can open a checkout. Empty when nobody can. */
  readableBy: string[];
}

/**
 * Which half of the repository setup is missing.
 *
 * The two halves are decided on two different surfaces — bindings on this page,
 * the `repo` tool grant in the version-controlled manifest — so each one alone
 * looks like a finished setup from where it was made. Ordering matters:
 * `bound-ungranted` is checked first because it is the state an operator is
 * most likely to be in *and* least likely to suspect, having just successfully
 * bound something.
 */
export function repoCoverage({
  repos,
  grantedAgents,
  repoGranted,
}: RepoCoverageInput): RepoCoverage {
  const readableBy = repos.length > 0 ? grantedAgents : [];
  if (repos.length > 0 && grantedAgents.length === 0) {
    return { notice: "bound-ungranted", readableBy };
  }
  // Only on a definite `true`: unknown must not be read as granted either, or a
  // host that cannot answer would tell every operator to bind something.
  if (repos.length === 0 && repoGranted === true) {
    return { notice: "granted-unbound", readableBy };
  }
  return { notice: null, readableBy };
}
