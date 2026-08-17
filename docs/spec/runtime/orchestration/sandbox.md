# Containerised programming tools

*Phase P5. Giving agents a real place to write and run code.*

Terms: [glossary](../../glossary.md). Read
[security/agent-isolation.md](../../security/agent-isolation.md) first — it is
authoritative on what actually confines an agent, and this document adds a
control rather than replacing that analysis.

---

## What exists

The toolbelt has `shell` and `code` grant namespaces, with patch and git
operations behind the latter. The vendored runtime ships a sandbox module with a
Docker backend and a working-directory jail — **and OpenCompany does not enable
it**. Landlock and bubblewrap backends exist upstream and are likewise unbuilt.

So an agent that runs code today runs it with whatever confinement the host
process has. That is the gap.

## What ships

The sandbox behind a new `sandbox` feature, plus a persistent per-company code
tree with a derived index, plus write-path placement that a shell cannot bypass.

Per this repo's testing rules, a new feature needs a row in the feature-lane
manifest naming the lane that runs its tests, and that lane must go through the
scoped-suite runner, which asserts a non-zero test count. A feature-gated test
that no lane enables compiles and reports nothing.

---

## Container posture

Normative, and each line is a control rather than a preference:

- Unprivileged user; **all** Linux capabilities dropped; `no-new-privileges`.
- **Read-only root filesystem.**
- **Exactly one mount**: the company's own workspace directory. Not the
  repository, not a home directory, not the container socket, not a broad host
  path.
- Process, memory, command-duration and output limits, all enforced.
- Network access retained — provider, search and telemetry calls need it.
  **This container adds no egress boundary of its own.** Whatever the pod can
  reach, code running in this sandbox can reach, including any private or
  link-local address the pod's network namespace can route to. Egress
  containment is [C1 in agent-isolation.md](../../security/agent-isolation.md#the-children-of-752),
  an open, product-level decision (default-deny egress versus the open-web
  tools it would break — see [the unresolved
  tension](../../security/agent-isolation.md#an-unresolved-tension-egress-policy-versus-the-web-tools))
  that this document does not resolve and MUST NOT be described as resolved
  here. Do not claim this sandbox contains SSRF; say what C1's state actually
  is.

### On the memory limit

**Set one, and do not set it low.** The sibling runtime's cap was 2 GiB and it
was wrong. A live container was OOM-killed mid-attempt, and an OOM kill is the
worst-shaped failure available: the kernel stops the process, so nothing reaches
the console, the run simply *ceases to appear*, and everything in flight is
lost.

Worse, the failure was recorded as a result. That workspace's own notes carried
"exact search stops at N=14" as a discovered ceiling. It was a sandbox limit
wearing the costume of a finding.

Two requirements follow: the cap must cover the runtime, every concurrent child,
and every subprocess they spawn between them; and the smoke lane MUST check the
container-events stream for OOM kills, because nothing else will tell you.

### Package installs

Installs land **inside the workspace**, so a read-only root survives and
dependencies persist beside the work that needs them. A run that must install
its toolchain before it can do anything spends its budget on setup; bake the
common ones into the image instead.

---

## Placement, enforced in code

An agent writing files freely produces a workspace whose listing is mostly
noise. The sibling runtime recorded one that reached thirty-one programs, four
data files and a scatter of captured output at its root, with the two documents
carrying the actual reasoning buried among them.

So placement is a rule in the write path, not an instruction in a prompt:

- Root is an **allowlist**, not a default. Programs, program output, downloads
  and configuration each have a home.
- A path that already names a directory is **left alone** — naming one is a
  decision, and the rule has no better information than the caller who made it.
- **A move is reported in the tool result, never performed silently.** A model
  not told where its file went writes the next one to the same place and then
  cannot read either back.

### The shell bypasses the tool layer

A heredoc or a redirect writes files without going through any file tool. So a
sweep MUST run after **every** shell command, applying the same placement rule
to whatever appeared.

The sweep never overwrites an existing destination, but it **reports the
collision**. A no-op sweep is silent.

### Path safety

Absolute paths, traversal, and symlinks resolving outside the workspace root are
all rejected, with canonical parents verified before any write. This is code, in
the tool layer, and not a sentence in a prompt.

---

## The code library

A `code/lib/` tree, persistent across runs, with a **derived** `INDEX.md`
carrying each function's signature, what it returns, and what established it
correct. That last column is the part not readable from the source.

- **One subject per module**, not one function per file. The tighter rule was
  tried and cost more than it saved: a routine needing a companion function did
  not fit, so it was inlined instead, and the directory filled with helpers
  nothing imported.
- Reading the helper you need should cost a few hundred bytes, not the whole
  library. That is what keeps the index worth routing into a prompt.
- **A row that has drifted from its function is worse than no row**: the next
  agent calls it as described rather than reading the source. Hence derived.

The index is a natural routed document for code-writing roles — see
[alignment.md](context-routing.md).

---

## Checkpointing

Work is checkpointed as it happens, so a run's intermediate states survive.

- After-tool middleware, fired for write tools **and the shell path**. The
  sibling runtime omits the shell from its write set, which means shell-written
  files are committed only incidentally by the next tool write. Do not repeat
  that.
- The history lives in an **out-of-band git directory** (`workspace.git/`). The
  working tree has only Git's `.git` pointer file, so ordinary Git commands work
  there without putting the object database among agent-authored files.
- **The pointer file is never trusted.** An agent can plant a `.git` of its own
  (it owns its workspace), so the checkpointer always runs its Git commands
  against an explicit `--git-dir` for the out-of-band path, rewrites any planted
  pointer back to it, and isolates those commands from inherited config and
  hooks (`GIT_CONFIG_NOSYSTEM`, `GIT_CONFIG_GLOBAL`, `core.hooksPath=`): a
  checkpoint commit must not execute code an agent wrote (CWE-94).
- An unchanged tree is a **no-op, not an error**.
- **A failed checkpoint never fails the tool that succeeded.** And precisely
  because it swallows failures silently, the commit lock from
  [alignment.md](alignment.md#locking) is mandatory — without it, lost commits
  are invisible.
- Generated artifacts stay in the workspace. Do not write them into a source
  directory.

This behavior is opt-in through `[workspace].git_enabled = true`; disabled is
the compatibility default.

---

## Declaring cost before spending it

A tool-building role SHOULD state time and space cost before substantial
execution, and a declaration naming a search strategy instead of a cost SHOULD
be refused. Brute force that validates a real method on small inputs is
legitimate; an unbounded search is not.

This is a weaker control than the others here — it is a prompt-level discipline,
and this document is otherwise about things enforced in code. It is included
because it is cheap and because the failure it prevents (a run that spends its
whole budget enumerating) is common. Treat it as guidance, not as a boundary.

---

## Verification

- Traversal, absolute paths, and escaping symlinks are all rejected.
- A sweep after a shell heredoc files the written artifact and names the move in
  the result.
- A sweep that would overwrite reports the collision instead.
- A container smoke test asserts read-only root, dropped capabilities, and the
  single mount.
- The smoke lane checks the container-events stream for OOM kills — a container
  that vanishes without an error must not read as a pass.
- A failed checkpoint leaves the successful tool result intact.
- Two concurrent write cascades do not strand a git index lock.
- Shell-written files are checkpointed, not merely swept.
- The `sandbox` feature has a row in the feature-lane manifest and a lane that
  runs its tests through the scoped-suite runner.
