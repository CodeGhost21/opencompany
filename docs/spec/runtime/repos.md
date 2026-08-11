# Bound repositories

Issue #245, **operator half**. A company's agents carry a full coding toolbelt
and, until this exists, nothing real to point it at: the sandbox is an empty
scratch directory that nothing ever populates, so `code-review` and
`bug-triage` can only work on text pasted into chat.

This document specifies the half that ships first: an operator binds a
repository and a credential, and the host keeps a mirror of it. **There is no
agent surface here** — no `repo` grant, no `repo_checkout`, no
`repo_pr` tool, nothing that puts a checkout into a workspace during a turn.

## Why the split is here and not somewhere else

The line is drawn at the credential boundary on purpose. Everything that
*reaches an account* — token intake, storage, host-side use, redaction,
revocation — lands in one change, complete. The tier that *hands an agent a
checkout* builds on something already whole.

The alternative was worse in a specific way, not just untidy: a half-wired
credential path is the one intermediate state this feature must never ship in.
A stored token with no revoke, or a mirror with no quota, or a bind whose
failure leaves a live credential behind, are each a real exposure that lives on
`main` between two merges.

## The two layers

| | Location | Written by | Network |
| --- | --- | --- | --- |
| **Mirror cache** | `<data_dir>/companies/<slug>/repos/<key>.git` | the host | yes, credentialed |
| **Checkout** | wherever the caller asks | `git clone --shared` off the mirror | no |

The mirror is bare, host-owned, and **outside every agent workspace**
(`harness/<company>/<agent>/workspace`). That placement is the point: it is
fetched with a credential, and an agent that could write to it could rewrite
what every later checkout resolves through.

It shares the `companies/<slug>/` prefix with the company bundle so one
company's whole footprint sits in one subtree — and therefore inside the one
quota walk `DataLayout::usage_bytes` already performs. Nothing in the fs store
reads or creates it; on a mongodb tenant no other part of that directory exists
at all, so the cache creates its own parents.

`RepoManager::materialize` is implemented and tested in this tier and has no
caller. The tool that materializes into an agent workspace, and the lifecycle
that removes the checkout at task end, are the follow-up's.

## Credential handling

### What is stored, and where

| Key | Holds |
| --- | --- |
| `repos/bindings` | A JSON index of every binding: URL, key, branches, sizes, timestamps, and a token **fingerprint** |
| `repos/token/<key>` | The credential itself, alone |

The split is what makes the read surface safe by construction rather than by
discipline: listing bindings reads the index document and nothing else, so no
read path has token bytes available to leak even by accident.

The fingerprint is the first 12 hex characters of the token's SHA-256. It is
enough to see that a credential is installed, to notice a rotation, and to tell
two bindings apart; it is far too short to attack a random 40-plus-character
token, and far too short to be mistaken for one.

`SecretStore` has no delete, so a revoked credential is stored as the empty
string and every read site treats empty as unset — the same convention the
Telegram channel credentials use.

### How the token reaches git

Never as an argument, an environment variable, a URL, or a line of git config.
All four are readable by anything running as the same user — `/proc/<pid>/cmdline`,
`/proc/<pid>/environ`, and the file itself forever for the last two.

Instead: git is spawned with a `GIT_ASKPASS` helper — a fixed six-line script
containing no secret — and the token is written into the child's **stdin**,
which is an anonymous pipe. The helper answers the username prompt with a fixed
literal and reads the password off that pipe. The bytes exist in a pipe buffer
and in two process memories, and nowhere else.

Two deliberate details:

- **stdin, not a dedicated inherited descriptor.** Handing git an extra open
  descriptor means `pre_exec` and a `dup2` — `unsafe` plus a new `libc`
  dependency — to obtain a pipe with exactly the properties stdin already has.
  Every command run here (`fetch`, `ls-remote`, `clone`, `init`, `config`,
  `rev-parse`) reads nothing from stdin.
- **The helper uses shell builtins only.** Passing the token to an external
  program would put it straight back into an argv.

The mirror's `origin` URL is credential-less, and a materialized checkout's
`origin` is the mirror's **local path** — so git run against a checkout has no
credentialed remote to reach for.

### The honest limit

The agent shell and the host-side fetch run **as the same user in the same
container**. This is not a kernel boundary and must not be described as one.

What it does close is the whole class of *incidental* exposure: a token in a
log, in a config file, in a process listing, in a workspace `.git/config`, in a
crash dump, in a support screenshot. That class is real, it is where credentials
actually leak in practice, and closing it is worth doing.

What it does not stop is a prompt-injected agent holding the `shell` grant that
attacks `/proc` during a fetch window. The protections that matter there are
different ones, and they are named rather than implied:

- a fine-grained, **single-repository, read-only** token, so the blast radius of
  a stolen credential is one repository's source and nothing else;
- the audit log on every shell call — detection, not prevention;
- container-level egress policy and per-agent user namespaces, which are the
  real fix and are a follow-up, not a claim made here.

### Repository configuration decides what runs

Issue #459 reclassified `read_workspace_state` for exactly this reason: it
shells out to git in a directory an agent can write, and a repository's own
configuration can name programs for git to execute. Every invocation in this
module therefore pins `core.hooksPath=/dev/null`, clears `core.fsmonitor`,
clears any inherited `credential.helper`, starts from an **empty** environment
with `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL=/dev/null`, and points
`$HOME` at a scratch directory with no `.gitconfig` in it.

Every invocation is also bounded by a deadline. A git that reaches a network it
cannot finish talking to does not fail — it waits, and the HTTP request waits
with it.

## Two deliberate departures from the issue

**Alternates, not hardlinks.** The issue proposes hardlinking objects from the
cache into each checkout: same filesystem, near-instant. It is also shared
mutable state — a hardlinked object file *is the same inode* as the mirror's, so
an agent that can write in its workspace can `chmod` and rewrite an object every
other agent's checkout resolves through. `git clone --shared` registers the
mirror as an alternate object store instead: read-only from the checkout's side,
and just as instant.

The consequence is that the mirror must never prune. An alternate holds no
reference a `gc` in the mirror can see, so a prune there can delete objects a
live checkout still needs. Mirrors are configured `gc.auto=0` and
`gc.pruneExpire=never`, and **space is reclaimed only by revoking a binding**,
which deletes the whole mirror at once.

**Refusal, not eviction.** The issue proposes evicting a mirror that pushes the
cache over quota. A bound repository is operator-configured state, and silently
deleting one converts a disk problem into an inexplicable "the agent can't see
our code any more" problem hours later, with nothing in the console explaining
it. An over-quota fetch fails loudly, names the cap, and says what to change.

## Quota

The cache is capped by `[workspace].tree_quota_gb` — the same limit that bounds
a company's workspace tree. Both answer "how much may one company hold on this
host", and a mirror is company-held binary payload like any other. No new knob,
and a hosted tenant that already sets one gets the cache covered without
touching its config. Absent means unlimited, the self-hosted default.

Two checks, because they catch different things:

1. **At bind, against the advertised size** (`GET /repos/{owner}/{repo}` →
   `size`), before a single object is transferred. Advisory only.
2. **After every fetch, against the measured size.** A packfile or LFS bomb does
   not have to match what the API advertises, which is why the first check
   cannot be the only one.

## HTTP surface

All three under both scope forms (`/api/v1/companies/{id}/…` and the
single-company alias `/api/v1/company/…`).

| Route | Authority | Does |
| --- | --- | --- |
| `POST …/repos` | admin | Validate, store the credential, mirror, record |
| `GET …/repos` | member | The redacted list + whether diffs are readable |
| `POST …/repos/{key}/revoke` | admin | Drop the entry, blank the credential, delete the mirror |

The two mutations require authority over the company (issue #403): what a
company's agents will read, and under whose credential, is decided *for* the
company. The list is a member read — which repositories a company reads is part
of knowing what the company is, and the body carries no credential material.

### Intake rules

- **https://github.com/&lt;owner&gt;/&lt;repo&gt; only.** Another forge, `http`,
  `ssh`, a userinfo section, a port, extra path segments, a query string, a `..`
  — each is refused rather than normalized. A URL is the thing a credential is
  about to be sent to.
- **A classic PAT (`ghp_…`) is refused**, with the steps to make a fine-grained
  one. Intake is the only moment the operator is still holding the token and can
  go make a better one. A GitHub App installation token (`ghs_…`) is accepted:
  it is already repository-scoped.
- **A failed bind stores nothing.** The token is written before the network is
  touched — it is what touches it — so a bad credential, an unreachable
  repository or an over-quota result rolls the whole thing back: the credential
  is blanked and the half-built mirror removed. The state this avoids is a
  binding that exists, cannot fetch, and holds a credential nobody remembers
  installing.

`POST …/revoke` rather than `DELETE …/repos/{key}` because it is not only a
deletion: it blanks the stored credential and removes the mirror from disk, and
it is the only way cache space comes back.

## Pull requests

`RepoManager::pull_request` returns a pull request's metadata plus its unified
diff (`Accept: application/vnd.github.diff`), host-side, with no Composio
involvement. The forge client is dependency-inverted behind a trait and its real
implementation is gated on the `github` feature, so the default build links no
HTTP client on this path and every test runs offline. Without one the manager
answers "not wired" rather than an empty diff a caller would read as "no
changes", and `GET …/repos` reports `pullRequestsAvailable: false` so the
console can say so instead of offering a control that fails.

Diffs are truncated at 1 MiB with a visible marker. Spilling an oversized diff
to a file belongs to the tier that has a workspace to spill into.

## Testing

Every mirror test runs against a bare `file://` fixture built in a temp
directory — a `main` branch, a `topic` branch and a `refs/pull/7/head`. No test
in this module touches the network.

Mocking git would test a mock. The bugs this code can actually have — a refspec
that fetches everything, a checkout whose `origin` still points at the
credentialed remote, a prune that eats an alternate's objects, a token that
lands in `.git/config` — are all bugs in how git is *driven*, and only a real
git catches them.

The credential tests bind with a sentinel token, then walk every byte the
mirror and the checkout wrote asserting it appears nowhere, and separately
assert the environment git receives carries none either.

## Not in this tier

The `repo` grant (excluded from `*`, like `composio`), `HarnessDeps.repos` with
a rebuild fingerprint, `repo_checkout` / `repo_pr` tools, clone-into-sandbox
lifecycle with deletion at task end, the boot sweep for orphaned checkouts, and
the grant editor's surfacing of all of it.

No push path exists anywhere: write tier — PR creation, agent-attributed
commits, namespaced branches, operator approval — is a separate follow-up.
