# The instance data root

Where one host keeps its state, who is allowed to write it, and what happens
when two processes want the same directory.

Split out of [`storage.md`](storage.md), which is at the repository's 500-line
ceiling. That file describes the *layout* inside the root; this one describes
the root itself.

## Resolution order

`src/store/paths.rs::resolve_home` and `src/app/config.rs::data_dir_from` answer
the same question and must agree — a host whose bundles and workspace resolved
differently would split its own state in half.

1. `--home` (the flag outranks everything)
2. `OPENCOMPANY_DATA_DIR` — what the platform manager injects into every tenant
   container
3. `$HOME/.opencompany`
4. `$USERPROFILE\.opencompany` — Windows sets this and not `HOME`
5. `.opencompany`, relative

Step 4 exists because step 5 is dangerous rather than merely inelegant. A
relative root resolves against the process working directory, which for a
double-clicked application is wherever the launcher happened to put it —
plausibly `C:\Program Files`, plausibly unwritable, and plausibly *different*
between launches. Two runs would quietly use two stores. `HOME` still wins where
both are set: git-bash and MSYS set both, and a user who exported `HOME` meant
it.

`OPENCOMPANY_HOME` is refused loudly rather than ignored — see `paths.rs`.

## Single writer, enforced

The runtime journal is single-writer. Two processes over one root overwrite each
other's companies, and until `src/store/lock.rs` existed nothing stopped them —
`resolve_home` handed the same directory to every caller that asked.

On a server that was survivable, because a second `opencompany serve` against
one root is a deliberate act. A desktop application is different in kind: it is
launched by double-clicking, and being launched twice is ordinary. So is the
common development shape — `opencompany serve` in a terminal against
`~/.opencompany`, then the desktop app opening the same root.

`serve` and `app::boot::prepare_instance` both take an exclusive advisory lock
on `<root>/.lock` (`flock`/`LockFileEx` via `fs2`) and hold it for the life of
the process. A second instance is refused immediately with a message naming the
directory and `OPENCOMPANY_DATA_DIR`.

An OS advisory lock rather than a pid file: the lock belongs to the open file
description, so the kernel drops it when the process exits for any reason —
clean exit, panic, `SIGKILL`, power loss. There is no stale state to detect and
nothing for an operator to delete by hand. The lock file itself is created if
absent and never removed; deleting it on release would race a second process
that has already opened the same path.

Scope: this is a *process* boundary on one machine. Two hosts over a network
filesystem are outside what `flock` promises, and that layout was never safe to
share regardless.

### Hosted deployments: the overlap window

**Open question for the platform.** In hosted mode the manager runs each tenant
as a container over `OPENCOMPANY_DATA_DIR=/data`. If a rollout ever has the new
pod running while the old one is still alive over the same volume, the new pod
now **fails to boot** with a configuration error where it previously started and
silently raced.

Refusing is the correct behaviour — the alternative is two writers over one
journal — but it changes what a deploy does, so it needs a decision rather than
a discovery:

- If the manager already uses a `Recreate`-style strategy (old pod terminated
  before the new one starts), nothing needs to change.
- If it uses a rolling strategy with overlap, either the strategy or the
  readiness gate has to account for a boot that legitimately refuses.

Whoever owns `opencompany-manager` should confirm which it is before this
reaches a tenant.

## Running two hosts side by side

Give each its own root. The lock makes this mandatory rather than advisory: the
second host over one root is refused at boot, where it previously started and
wrote over the first's companies.

```sh
OPENCOMPANY_DATA_DIR=/tmp/oc-a opencompany serve \
  --company companies/e2e_harness --bind 127.0.0.1:8095 &
OPENCOMPANY_DATA_DIR=/tmp/oc-b opencompany serve \
  --company companies/e2e_harness --bind 127.0.0.1:8096 &
```

`--home /tmp/oc-a` places the bundles the same way and takes precedence, but it
does **not** move the shared workspace — prefer the variable for side-by-side
hosts.

## Instance identity

`<root>/instance-id` holds 16 random bytes, hex-encoded, minted on first boot
and served unauthenticated at `/spec`. It exists so a client holding several
connections can ask "is this the same host I already know?" and get a durable
answer — a URL cannot do that job, because a host moves between `localhost`, a
LAN address and a tunnel over one afternoon.

Random rather than derived from hostname, bind address or company set: `/spec`
is unauthenticated, so anything derived is a fact about the deployment handed to
anyone who asks. Random bytes name nothing. It authenticates nothing either —
do not grow a check that treats knowing it as proof of anything.

Both `instance-id` and `.lock` are runtime state and are ignored by git. A
committed `instance-id` would give every clone the same public identity, which
is the one thing the file exists to prevent.
