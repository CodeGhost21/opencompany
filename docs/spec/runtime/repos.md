# Bound repositories

Issue #245. A company's agents carry a full coding toolbelt and, until this
exists, nothing real to point it at: the sandbox is an empty scratch directory
that nothing ever populates, so `code-review` and `bug-triage` can only work on
text pasted into chat.

It shipped in two halves, and this document describes both:

| Half | Ships | Lives in |
| --- | --- | --- |
| **Operator** | Bind a repository + credential; the host keeps a bare mirror | `runtime::repo_manager` |
| **Agent** | `repo_checkout` / `repo_pr` behind an explicit `repo` grant | `harness::repo` |

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

## The one host-owned layer

| | Location | Written by | Network |
| --- | --- | --- | --- |
| **Mirror cache** | `<data_dir>/companies/<slug>/repos/<key>.git` | the host | yes, credentialed |

The mirror is bare, host-owned, and **outside every agent workspace**
(`harness/<company>/<agent>/workspace`). That placement is the point: it is
fetched with a credential, and an agent that could write to it could rewrite
what every later checkout resolves through.

It shares the `companies/<slug>/` prefix with the company bundle so one
company's whole footprint sits in one subtree — and therefore inside the one
quota walk `DataLayout::usage_bytes` already performs. Nothing in the fs store
reads or creates it; on a mongodb tenant no other part of that directory exists
at all, so the cache creates its own parents.

The **checkout** — a confined working tree inside one agent's workspace — is
the agent half's, and is specified under ["Confining a
checkout"](#confining-a-checkout). It is deliberately not in the operator
module: it is a confinement problem, so it is solved with the code that has to
live with it.

## Credential handling

### What is stored, and where

| Key | Holds |
| --- | --- |
| `repos/bindings` | A JSON index of every binding: URL, key, branches, sizes, timestamps, and a token **fingerprint** |
| `repos/token/<key>` | The credential itself, alone |

The split is what makes the read surface safe by construction rather than by
discipline: listing bindings reads the index document and nothing else, so no
read path has token bytes available to leak even by accident.

### The key

One string names the credential, the mirror directory, the index entry and the
revoke route, so it has to be **injective**: two repositories that share a key
share a token, and one operator's credential is then used to fetch the other's
source.

`<slug(owner)>-<slug(repo)>-<12 hex>`, where the slug lowercases and flattens
everything outside `[a-z0-9]` to `-`, and the hash is over
`<owner>/<repo>` ASCII-lowercased.

The slug alone is not injective and cannot be made so while staying readable:
owner and repository names legitimately carry `.`, `_` and `-`, so
`acme/data.pipeline`, `acme/data_pipeline` and `acme/data-pipeline` all flatten
to `acme-data-pipeline`, and `a-b/c` and `a/b-c` collide across the separator.
The hash restores the distinction because it runs *before* any flattening, over
a string only one pair of coordinates can produce — the URL parser refuses a `/`
inside either part, so the `/` in the hashed string is unambiguous.

Lowercasing the hash input is equally deliberate: GitHub resolves owner and
repository case-insensitively, so `Acme/Widgets` and `acme/widgets` are one
repository and must land on one key — otherwise binding it twice, spelled
differently, installs two credentials instead of being refused as the duplicate
it is.

Twelve hex characters is 48 bits, and is not a security parameter. A collision
would not merge two bindings: the second bind fails with a conflict naming the
key, which is a refusal an operator can see. Each part is capped at 100
characters by the parser, so a key is at most 214 bytes and `<key>.git` at most
218 — a legal filename everywhere.

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
  Every command run here (`init`, `remote`, `config`, `ls-remote`, `fetch`)
  reads nothing from stdin.
- **The helper uses shell builtins only.** Passing the token to an external
  program would put it straight back into an argv.

The mirror's `origin` URL is credential-less: the credential is answered for per
invocation, so the one file that outlives every fetch holds nothing worth
stealing.

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

That follow-up is issue #752, and the threat model it owed is now written:
[../security/agent-isolation.md](../security/agent-isolation.md). Read it before
concluding that any of the above adds up to confinement. Its residual section
is the part that matters — it holds after every control #752 tracks has landed.

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

## Confining a checkout

The issue proposes hardlinking objects from the cache into each checkout: same
filesystem, near-instant. It is also shared mutable state — a hardlinked object
file *is the same inode* as the mirror's, so an agent that can write in its
workspace can `chmod` and rewrite an object every other agent's checkout
resolves through.

`git clone --shared` is **not** the answer to that, and an earlier draft of this
document said it was. A `--shared` clone records the mirror's path in
`.git/objects/info/alternates` and leaves `origin` pointing at it, so a commit
in the checkout followed by `git push origin HEAD:refs/heads/main` advances the
host's mirror directly — an agent can poison what every later checkout of that
repository reads. Git resolves that path out of the checkout's own
`.git/config`, so no guard on the *command* an agent may run closes it, and
removing or replacing `origin` does not either: the mirror's path is still
sitting in the alternates file, and the pushing process is the same uid that
owns the mirror, so a `chmod` is not a boundary.

### What ships: a full object copy, then sever

1. **Refresh the mirror** host-side, through the same hardened, credentialed
   fetch the bind path uses (plus `refs/pull/N/head` when a pull request is
   named). This is the only thing in the process that touches the token.
2. **Clone over the `file://` transport** into
   `<workspace>/repos/<key>`, `--single-branch`. The URL form is load-bearing
   and must not be "simplified" to a bare path: a path-shaped source turns on
   git's local optimization — hardlinks, or an alternates entry under
   `--shared` — while a `file://` URL forces the ordinary fetch/pack path and
   produces a genuine object **copy**.
3. **A pull request** is fetched by refspec into `refs/oc/pr/N` and checked out
   detached, *before* the sever — the only window in which the checkout is
   allowed to name the mirror at all.
4. **Sever.** Remove `origin`; delete `.git/FETCH_HEAD` and `.git/ORIG_HEAD`,
   both of which record the source URL verbatim; and hard-error if
   `.git/objects/info/alternates` exists, because if it does, git shared objects
   and the checkout's isolation is a fiction. Reflogs never exist: the clone
   sets `core.logAllRefUpdates=false` in the new repository *before* fetching,
   so `clone: from file:///…` is never written into `.git/logs/HEAD` — a file no
   `remote remove` touches, and the leak that would make "severed" a claim about
   one file rather than a property of the directory.

After step 4, **no byte under the checkout's `.git/` names the mirror**, which
is what the test asserts — by grep, over every file.

### The second line, and its honest limit

Every mirror also carries an always-refusing `pre-receive` hook, so a push aimed
at the mirror's *explicit path* is refused by the receiving end rather than
relying on the pusher having no address.

It is the second line, not the first, and the difference matters: the primary
defence is that the sanctioned attack has no address left to aim at. The agent
shell and the host run **as the same user**, so an agent that escapes shell
confinement can edit or delete that hook and then push. This raises the bar; it
is not a kernel boundary. The real ones — a distinct uid for the agent shell, or
a read-only bind mount — are the same follow-up named under ["The honest
limit"](#the-honest-limit), and are not claimed here.

### The lifecycle

A checkout's life is one turn's — with one deliberate exception the write tier
needs (issue #796). Every path the tools create is recorded on a per-turn
ledger, and an RAII janitor claimed at each turn's entry point deletes them on
the way out — success, error, steer cancel, redirect exhaustion and panic-unwind
alike. A mid-loop redirect deletes the abandoned turn's checkout too, so a re-run
starts from a fresh tree rather than one a discarded turn half-patched.

**The exception: surviving an explicit approval request.** A write can span an
agent's deliberate `request_approval`, which ends the turn while it waits. So a
checkout a *task* turn parked with is held on a task-keyed retained
set the janitor does not touch, keyed by the task the parked approval carries
(`GrantedCall::origin_task`); the approval continuation reclaims it, so the resumed
turn commits and publishes on the same tree — and the same commit — the parked
turn left. It is deleted when the task's resumed step finishes without parking
again, or — if the approval is denied or expired — swept the next time any turn
claims the janitor and no live grant still names the task. This is the "deleted
at task end" this tier always promised; a per-turn delete made it a deadlock
under supervision.

A host killed mid-turn ends no turn, so boot sweeps
`<harness>/<company>/*/workspace/repos` before the company starts. It is
tenant-scoped: one company booting can never delete another's bytes.

`workspace/repos` is on the publish scan's skip list. Cloned source and spilled
diffs are third-party content that appears as thousands of new files the moment
a checkout runs, and issue #244's nudge must never ask an agent whether somebody
else's repository is a deliverable.

## The agent surface

Two tools, behind an **explicit** `repo` grant. The catch-all `*` does not
confer it — the `media` / `composio` / `search` precedent, and sharper here:
a checkout puts a third party's source inside a sandbox the same agent may hold
`shell` over, so a wildcard set for file and shell tools must not carry it in.

| Tool | Does | Answers with |
| --- | --- | --- |
| `repo_checkout(repo, ref? \| pr?)` | Refresh, clone, sever | A workspace-**relative** path and the head commit — never the mirror's path |
| `repo_pr(repo, number)` | Metadata + unified diff, host-side | The diff inline, or a workspace file when it is too large to read inline |

Three gates, and the third is not redundant: an explicit grant, a wired manager,
and **at least one binding**. A granted, wired, unbound company has nothing to
resolve against, so every call would be a refusal listing an empty set — wiring
nothing and warning is the honest state, and it is what the console's
"granted but nothing bound" notice tells the operator to fix.

A `repo` argument is a **lookup** against what the operator bound — by key, by
canonical URL (through the same strict parser the bind route uses), or by
`owner/repo` — and an unknown one is refused with the list of what *is* bound.
Nothing an agent passes is ever interpolated into a path or a URL. An
agent-supplied `ref` goes through the same validator an operator-supplied branch
does, and must be one the binding actually mirrors.

Both tools are `Reach::Consequence`, `Standing::PerCall`: **park** under
`supervised` and `auto`, **denied** under `readonly`, allowed under `full`. Both
names read like reads and neither is one in the sense the declaration table
means — each pulls third-party-authored content into the agent's context and
reaches the forge host-side under the operator's credential, and one of them
writes a tree.

Neither is feature-gated. The mirror and the git runner are always compiled, and
without a forge client `repo_pr` degrades through the manager's honest
"not wired" answer rather than through a build that omits the tool — hiding an
agent-reachable surface behind a feature no CI job compiles is how three
previous gaps happened.

### Quota, before the bytes move

A checkout is refused — not evicted — when it would push the company past
`[workspace].tree_quota_gb`, estimated at `2 ×` the mirror's measured size and
checked before anything is transferred. Same rule as the fetch path, same
reason: quietly deleting somebody else's checkout to make room turns a disk
problem into a mystery.

## Two deliberate departures from the issue

**Never prune.** Mirrors are configured `gc.auto=0` and `gc.pruneExpire=never`,
and **space is reclaimed only by revoking a binding**, which deletes the whole
mirror at once. A prune is pure risk on a cache that is refetched incrementally
and never read twice, and it also reserves the property the checkout tier needs
whichever confinement it picks: an alternate object store holds no reference a
`gc` in the mirror can see, so a prune here could delete objects a live checkout
is resolving through.

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

- **The host must keep secrets off its own disk** (issue #752). On
  `OPENCOMPANY_STORAGE=fs` or `sqlite` the credential would land as plaintext
  under the company's data directory, readable by the uid the agent shell runs
  as, so the bind is refused with `409 Conflict` before anything is parsed or
  written. The same condition refuses **boot** for a company whose roster grants
  `repo`, and withholds the repo tools at agent build. Remedies:
  `OPENCOMPANY_STORAGE=mongodb` with a Mongo URI, or drop the `repo` grant.
  This is a breaking change for `fs` deployments that already bound one — see
  [storage.md](storage.md) and, for why a warning would not have been honest,
  [../security/agent-isolation.md](../security/agent-isolation.md).
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

Diffs are truncated at 1 MiB with a visible marker, host-side. The agent tier
adds a second, smaller boundary for a different reason: every tool result is cut
on its way into the model's context, so an in-band megabyte would be silently
clipped to a fraction of itself with no way to reach the rest. A diff over the
inline cap is therefore written whole into `<workspace>/repos/<key>.pr-N.diff`
and the reply names that path — a file the agent can read, grep and page with
the tools it already holds. The reply also says which of the two cuts it is
looking at, because "the host stopped at 1 MiB" and "too big to read inline"
call for different next moves.

## Testing

Every mirror test runs against a bare `file://` fixture built in a temp
directory — a `main` branch, a `topic` branch and a `refs/pull/7/head`. No test
in this module touches the network.

Mocking git would test a mock. The bugs this code can actually have — a refspec
that fetches everything, a mirror that prunes objects out from under an
alternate, a token that lands in `.git/config` — are all bugs in how git is
*driven*, and only a real git catches them.

The credential tests bind with a sentinel token, then walk every byte the mirror
wrote asserting it appears nowhere, and separately assert the environment git
receives carries none either.

Two binds racing on one key are driven concurrently, because the failure they
guard against only exists in the interleaving — see
["Concurrent binds"](#concurrent-binds).

The checkout tier's headline test is an **attack**, not an inspection. It
materializes a checkout, commits a poison file, then makes both pushes an agent
could actually make — `git push origin HEAD:refs/heads/main`, and a push naming
the mirror's path explicitly — and asserts the mirror's refs *and object list*
are byte-identical afterwards. Asserting "`origin` is absent" would not catch
the rejected design at all: `--shared` with `origin` removed still shares
objects.

Isolation is then proved by **destruction**: the mirror directory is deleted and
`git fsck --strict` and `git log` still succeed in the checkout. A hardlinked
clone is caught separately by asserting `st_nlink == 1` on every object file,
and the sever is checked by grepping every byte under `.git/` for the mirror's
path.

## Freshness

The roster is rebuilt when the binding set moves, fingerprinted over
`(key, token_fingerprint, branches)` — so a bind, a credential rotation and a
revoke each reach the agent on the company's next turn with no restart. All
three have to move it: a rotation changes nothing about *which* repositories
exist, and a revoke blanks a credential while the key survives, so a roster
keyed on the set alone would hand an agent a tool over a binding that can no
longer fetch. `size_bytes` and `last_fetched_millis` are deliberately excluded —
both move on every fetch, and a fetch is what the agent's own tool does.

## Console

Grants are **not editable** from the console, for any namespace: a tool grant is
version-controlled manifest state, and `AgentDetailDto.editable` excludes tools
for both agent sources. So the repositories card *reports* rather than offers a
control it could not honour.

It states who can open a checkout — resolved through the same roster-grant walk
the harness builds agents with, so it says what is wired rather than what the
manifest looks like it should wire — and names whichever half of the setup is
missing:

- **granted, nothing bound** → bind one;
- **bound, nobody holds `repo`** → add `repo` to `[tools].allow`, and a broad
  `*` deliberately will not do.

Both are otherwise silent: the tools simply are not wired, and the only symptom
is an agent that says it cannot see the code.

## Beyond this tier — the write tier

The read tier above ships no push path of its own. The **write tier** is a
separate, explicitly-added capability layered on top, arranged so that adding it
is a new opt-in rather than a hole that was already open here. It is built in
three parts:

- **A `repo.write` grant + credential push-capability** (issue #734). The grant
  is distinct from and tighter than `repo`: a bare `repo` (the read tier every
  company sets) never confers it, and the catch-all `*` never confers it. Each
  binding also records whether its bound credential can push, read from the
  forge's `permissions.push` at bind time and healed on the next fetch while it
  is still unknown. Granting `repo.write` over a read-only credential is
  fail-closed — it warns and wires nothing.
- **`repo_publish`** (issue #735). The agent commits locally in its checkout,
  then publishes host-side. The host *fetches* the checkout's committed HEAD into
  the mirror on a host-owned `oc/<company>/<task>` branch — a fetch never invokes
  `receive-pack`, so the read tier's `pre-receive` refusal and its no-push
  contract test stay untouched — and, only after the operator approves, pushes
  that exact commit to the remote. The agent still holds no credentialed remote
  and never pushes; every structural refusal (host-generated namespaced branch,
  never a force push, never the default branch, never a ref outside `oc/`) lives
  in `RepoManager` where no prompt can reach it.
- **Pull-request creation** (issue #736). After the push, the host opens a pull
  request into the repository's default branch, best-effort: a PR that fails to
  open leaves the branch on the remote and reports that honestly on the task,
  rather than failing the publish.

Still absent: **signed** commits carrying a per-agent identity key (issue #738,
deferred — plain author/committer attribution ships with the write tier, but
signing waits for a consumer that verifies a signature); a distinct uid or
read-only bind mount for the agent shell (the real filesystem boundary, named
under ["The honest limit"](#the-honest-limit)); and forges other than GitHub.
