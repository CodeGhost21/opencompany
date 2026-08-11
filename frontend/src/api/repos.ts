// The repository binding API (issue #245, operator half): the console reads and
// writes which real repositories a company's agents will be able to read.
//
// The credential is write-only, and that asymmetry is the whole shape of this
// module. A token goes out on `bindRepo` and is never returned by anything: the
// read shape carries a `tokenFingerprint` — twelve hex characters of the
// token's SHA-256 — which is enough to see that a credential is installed, to
// notice a rotation, and to tell two bindings apart, and is not enough to be
// worth stealing.
//
// Standalone functions over the shared client, mirroring `api/mcp.ts` and
// `api/skills.ts`, so `OpenCompanyClient` needs no change.

import type { OpenCompanyClient } from "./client";
import { expectList } from "./mcp";

/** One bound repository, as the host reports it. Never carries a credential. */
export interface Repo {
  /** The cache key, and the id the revoke route takes. */
  key: string;
  /** The canonical `https://github.com/<owner>/<repo>` URL. */
  url: string;
  /** The owning user or organization. */
  owner: string;
  /** The repository name. */
  repo: string;
  /** The branches the host mirrors. Never empty. */
  branches: string[];
  /** A non-secret identifier for the stored credential. */
  tokenFingerprint: string;
  /** When the mirror last completed a fetch, in epoch milliseconds. */
  lastFetchedMillis?: number;
  /** The mirror's size on disk, in bytes. */
  sizeBytes: number;
  /** When the binding was made, in epoch milliseconds. */
  boundAtMillis: number;
}

/** The list route's body. */
export interface RepoList {
  repos: Repo[];
  /**
   * Whether this host can also read pull-request metadata and diffs, which
   * needs a forge HTTP client not every build links. The card reports the gap
   * rather than implying a capability that is not there.
   */
  pullRequestsAvailable: boolean;
}

/** A mutating route's body. `repo` is absent on revoke. */
export interface RepoMutation {
  repo?: Repo;
  note: string;
}

/** The bind body. `token` is write-only — no read ever returns it. */
export interface BindRepo {
  /** `https://github.com/<owner>/<repo>`. The host accepts nothing else. */
  url: string;
  /** A fine-grained, read-only, single-repository personal access token. */
  token: string;
  /** Branches to mirror. Omit or leave empty for the repository's default. */
  branches?: string[];
}

/** What a company has bound. */
export async function listRepos(
  client: OpenCompanyClient,
  company: string | null,
): Promise<RepoList> {
  const body = await client.get<unknown>(`${client.scopeFor(company)}/repos`);
  // Same defence as `api/mcp.ts`: `client.get<T>` casts an unparsed body
  // straight to `T`, so the declared type is a claim about the host and never a
  // check on it. Checking the one property this route promises turns a wrong
  // claim into a load error here instead of a crash several renders away.
  const record = (body ?? {}) as Partial<RepoList>;
  return {
    repos: expectList<Repo>(record.repos ?? [], "repository list"),
    pullRequestsAvailable: record.pullRequestsAvailable === true,
  };
}

/** Bind a repository. Fails without storing anything if the host refuses it. */
export function bindRepo(
  client: OpenCompanyClient,
  company: string | null,
  body: BindRepo,
): Promise<RepoMutation> {
  return client.post<RepoMutation>(`${client.scopeFor(company)}/repos`, body);
}

/** Unbind one: drops the credential and deletes the mirror from the host. */
export function revokeRepo(
  client: OpenCompanyClient,
  company: string | null,
  key: string,
): Promise<RepoMutation> {
  return client.post<RepoMutation>(
    `${client.scopeFor(company)}/repos/${encodeURIComponent(key)}/revoke`,
    {},
  );
}
