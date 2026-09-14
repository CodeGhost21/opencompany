# Phase 6a — remove four environment variables (part 1 of 2)

Slice 6a of the keys rework (issue #2306), added by the operator on
2026-09-15. Read [README.md](README.md) and [phase-6.md](phase-6.md) first.

- **Code read at:** `upstream/main @ fcfb3e1bc` (2026-09-14), the same
  revision every other slice in this folder reads. `upstream/main` was
  re-fetched on 2026-09-15 and is still at `fcfb3e1bc`.
- **Runs after:** 2a, 2d, 5b. Do not start this slice on a branch that has not
  landed all three — see §0.
- **Removes exactly:** `OPENCOMPANY_INFERENCE_KEY`, `OPENCOMPANY_INFERENCE_URL`,
  `OPENCOMPANY_COMPOSIO_BACKEND_URL`, `TINYHUMANS_TOKEN_FILE`. Nothing else.
  `OPENHUMAN_*` names are unaffected (Q16); so are `TINYHUMANS_API_KEY`,
  `TINYHUMANS_API_URL` and `OPENCOMPANY_INFERENCE_MODEL`.
- **Continues in:** [part 2](phase-6a-remove-env-vars-part2.md) — Composio,
  the E2E fixture replacement, docs, tests, must-not-touch, done-when.

## 0. Read first: why order is load-bearing here

This is the one slice in this folder that **removes an escape hatch other
slices are still using while they land**. Get the order wrong and a company
that resolves fine on the commit before 6a gets a 400 or an outage on the
commit after it, with no variable left to unblock it.

**`OPENCOMPANY_INFERENCE_URL` is not only a URL override.** current-state.md
§3.1 (managed chain step 4) and §8 (env var table) both show it feeding
`hosted_endpoint_from_env` (`src/harness/built_in/provider.rs:182-195`), which
backs `EnvDefault` (`src/company/inference.rs:459-466`) — the **lowest-
precedence source `resolve_legacy_scoped` tries in its own right** (its step
3, `inference.rs:1544-1589`), independent of whether any company names
`managed`. A keyless `openrouter` declaration, an unset company on a host with
no other configuration, and a hosted tenant with no default all fall through
to this arm today.

Phase 2a's own §0 gate exists for exactly this reason: flipping
`PLATFORM_BASE_URL` / `DEFAULT_TINYHUMANS_INFERENCE_URL` to the proxy while an
injected `OPENCOMPANY_INFERENCE_URL` could still point that arm at the old
`/openai/v1` surface would leave two live behaviours, and the proxy 400s a
tier name that `/openai/v1` silently accepted. 2a's own text: "**Never rewrite
the injected URL**... Hosted tenants run on whatever the manager injects,
so that is a manager-side prerequisite."

Once 6a removes the variable, that escape hatch is gone **by construction**:
the constant becomes the *only* base URL left for the env-default arm, with
no override anywhere. So:

1. **2a must have already landed**, including its gated commit 5 (moving the
   constants to the proxy) — otherwise 6a instantly and permanently strands
   every company on that arm at `/openai/v1` with no way to point them
   anywhere else, which is worse than the outage 2a's §0 gate was written to
   avoid, because now nothing can fix it short of a code change.
2. **2d must have already landed** — the proxy 400s a tier name, and once
   `OPENCOMPANY_INFERENCE_URL` is gone there is no way to route that arm's
   traffic anywhere that still accepts one.
3. **5b should have already landed** — not for correctness (5b does not
   change what the env-default arm resolves to), but because 6a's own edits
   touch `resolve_legacy_scoped` and `hosted_endpoint_from_env`, both of which
   5b also touches indirectly through `resolve_effective_scoped`'s tail.
   Landing 6a first would mean re-deriving line numbers twice.

**Do not run 6a's Rust commits (§5) before confirming all three land.** Check
with `git grep -n "fn resolve_for_turn" src/company/inference.rs` (2b/5b
present) and `git grep -n "agent-integrations/openrouter\"" src/company/inference.rs src/harness/built_in/provider.rs` (2a's constants already point at
the proxy — the "3b" commit from 2a's own gate). If either check fails, stop
and report; do not proceed by relaxing this slice's scope.

**The E2E consequence is severe and is the reason §9/part 2 §3 exist.** The
Console E2E "live brain" and "mock brain" lanes stand a local fixture server
up and point the host at it with `OPENCOMPANY_INFERENCE_URL` +
`OPENCOMPANY_INFERENCE_KEY` (`frontend/playwright.config.ts:214-228`). Once
those variables are gone, `hosted_endpoint_from_env` can no longer be pointed
at a local fixture — it resolves only to the real `api.tinyhumans.ai` proxy,
with a real-shaped static key or nothing. Part 2 §3 designs the replacement:
an explicit `tinyhumans` provider row (2a), whose `base_url` is set directly
in the stored row rather than read from the environment, and which is never
proxied (`decl_for_indexed`, `inference.rs:1413-1443`) — so it never touches
any of the four removed variables at all.

## 1. Files

| File | What changes |
|---|---|
| `src/harness/built_in/provider.rs` | `hosted_endpoint_from_env` (:182-195), `harness_inference_from_env` doc (:131-146), `PlatformCredentialStatus::boot_warning` (:343-361) |
| `src/company/inference.rs` | `EnvDefault` doc (:459-466) |
| `src/company/credentials.rs` | `TOKEN_FILE_ENV` (:66), `TinyhumansTokenSource` (`Tier::ProjectedFile`, `from_env`, `from_parts`, `source_of_parts`, `tier`, `token_file`, `describe`, `hash_identity`, `invalidate`, `current_at`), module doc (:1-52) |
| `src/app/config.rs` | `RuntimeConfig.tinyhumans_token_file` field (:715-719), `credential_source` (:745-758), the `resolve()` build (:930-938, :965-981) |
| `src/app/doctor.rs` | `value_of` (:87-91), `FIELDS` (:106), `report`'s `CREDENTIAL_SOURCE_FIELD` layer match (:130-140ish), the `cycles` capability's `needs` text (:150-155) |
| `src/app/types.rs` | `AppConfig::credential_available_in` / `credential_source_in` (:255-267) |
| `src/bin/opencompany.rs` | `tinyhumans_credential` build (:2153-2157) |
| `src/company/composio.rs` | `COMPOSIO_BACKEND_URL_ENV` (:32), `backend_url_or_default` doc (:28-32) — part 2 §2 |
| `src/harness/built_in/composio.rs`, `src/harness/built_in/mod.rs`, `src/runtime/builder.rs`, `src/server/ops/composio.rs` | every `COMPOSIO_BACKEND_URL_ENV` read — part 2 §2 |
| `frontend/playwright.config.ts`, `frontend/test/e2e/host.sh`, a new shared fixture helper | E2E provider-row replacement — part 2 §3 |
| `docs/spec/runtime/config.md`, `docs/modules/inference/credentials.md`, `docs/modules/inference/current-state.md`, `docs/modules/openhuman/README.md`, `docs/modules/composio/data-model.md`, `gitbooks/developers/configuration.md` | part 2 §4 |

## 2. Current code

`hosted_endpoint_from_env` (`provider.rs:182-195`), the credential+URL pair
both `harness_inference_from_env` and `PlatformCredentialStatus::resolve`
read:

```rust
pub(crate) fn hosted_endpoint_from_env(env: &dyn EnvSource) -> Option<(Credential, String)> {
    let credential = match env
        .get("OPENCOMPANY_INFERENCE_KEY")
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        Some(key) => Credential::from_value(key),
        None => Credential::from_source(Arc::new(TinyhumansTokenSource::from_env(env)?)),
    };
    let base_url = env
        .get("OPENCOMPANY_INFERENCE_URL")
        .unwrap_or_else(|| DEFAULT_TINYHUMANS_INFERENCE_URL.to_string());
    Some((credential, base_url))
}
```

`TinyhumansTokenSource::from_env` (`credentials.rs:207-212`), the two-tier
resolver both this function and `search_backend_from_env` share:

```rust
pub fn from_env(env: &dyn EnvSource) -> Option<Self> {
    Self::from_parts(
        env.get(TOKEN_FILE_ENV).as_deref().map(Path::new),
        env.get(API_KEY_ENV).as_deref(),
    )
}
```

`RuntimeConfig`'s manual `AppConfig` build for `/spec` (`src/bin/opencompany.rs:2153-2157`):

```rust
let tinyhumans_credential = std::env::var("OPENCOMPANY_INFERENCE_KEY")
    .or_else(|_| std::env::var("TINYHUMANS_API_KEY"))
    .ok()
    .filter(|value| !value.trim().is_empty())
    .map(opencompany::ports::types::SecretValue);
```

## 3. Target code: `hosted_endpoint_from_env` and `EnvDefault`

```rust
/// Resolve the shared hosted-endpoint `(credential, base_url)` pair the
/// managed LLM chain's env-default arm addresses.
///
/// The credential is always the platform token source
/// ([`TinyhumansTokenSource::from_env`]: a static [`TINYHUMANS_API_KEY`] —
/// [`OPENCOMPANY_INFERENCE_KEY`] and the projected-file tier were removed in
/// the keys rework, issue #2306 phase 6a). **Nothing configured ⇒ `None`.**
/// The base URL is always [`DEFAULT_TINYHUMANS_INFERENCE_URL`] — an injected
/// [`OPENCOMPANY_INFERENCE_URL`] override was removed in the same phase, so
/// there is no way to point this arm anywhere else. A deployment that needs a
/// different endpoint (staging, a local fixture) does so with an explicit
/// provider row instead (2a) — see phase-6a-remove-env-vars-part2.md §3.
pub(crate) fn hosted_endpoint_from_env(env: &dyn EnvSource) -> Option<(Credential, String)> {
    let credential = Credential::from_source(Arc::new(TinyhumansTokenSource::from_env(env)?));
    Some((credential, DEFAULT_TINYHUMANS_INFERENCE_URL.to_string()))
}
```

`env` stays a parameter (not dropped): `TinyhumansTokenSource::from_env` still
needs it for `TINYHUMANS_API_KEY`, and dropping it would ripple through every
caller's signature for no gain.

`harness_inference_from_env`'s doc (:131-146) loses its "Precedence" bullets
for credential and url; keep only the model bullet and reword the intro to:
"Resolve a [`HostedProvider`] configuration (and its default model) from the
environment. The credential is the platform token source or nothing; the URL
is always [`DEFAULT_TINYHUMANS_INFERENCE_URL`]."

`EnvDefault`'s doc (`inference.rs:459-466`) — replace both field docs:

```rust
#[derive(Clone, Debug)]
pub struct EnvDefault {
    /// Always [`DEFAULT_TINYHUMANS_INFERENCE_URL`] (`harness_inference_from_env`).
    /// No environment override exists after phase 6a.
    pub base_url: String,
    /// The platform token source, or the static `TINYHUMANS_API_KEY` value it
    /// resolves to. `OPENCOMPANY_INFERENCE_KEY` was removed in phase 6a.
    pub credential: Credential,
}
```

`PlatformCredentialStatus::boot_warning` (`provider.rs:343-361`) reads
`TOKEN_FILE_ENV` and `API_KEY_ENV` from `credentials.rs` already — no `use`
change — but its "no platform credential resolved" message
(`:346-353`) currently names both tiers ("A hosted tenant expects the
platform-projected token volume named by `{TOKEN_FILE_ENV}`; a local or
self-hosted instance expects a static `{API_KEY_ENV}`"). After 6a there is
only one tier; reword to: `"no platform credential resolved: managed
inference, embeddings, web_search and media generation are ALL unwired for
every company on this deployment (fail-closed). Set a static {API_KEY_ENV}."`
Delete the `self.projected_tier && !self.media` arm (:356-361) entirely —
`projected_tier` can never be `true` once `TinyhumansTokenSource` has no
projected-file tier (§5, part 2) — after confirming with
`git grep -n "projected_tier" src` that nothing else reads the field before
deleting it from `PlatformCredentialStatus` too.

## 4. Target code: `src/bin/opencompany.rs`

```rust
// Hosted-brain credential, resolved with the same precedence the harness uses
// (`harness_inference_from_env`) so `/spec`'s `cycles_available` reflects
// whether cognition can actually run. `OPENCOMPANY_INFERENCE_KEY` was removed
// in phase 6a (issue #2306); only the static platform key remains.
let tinyhumans_credential = std::env::var("TINYHUMANS_API_KEY")
    .ok()
    .filter(|value| !value.trim().is_empty())
    .map(opencompany::ports::types::SecretValue);
```

Then check whether `resolve_serve_base_url("TINYHUMANS_API_URL", …)` just
below (:2161-2167) reads anything from `OPENCOMPANY_INFERENCE_URL` —
`git grep -n "OPENCOMPANY_INFERENCE_URL" src/bin/opencompany.rs` on
`fcfb3e1bc` returns nothing, so no further edit is needed here; confirm this
on the branch before moving on.

## 5. Target code: `TinyhumansTokenSource` becomes static-only

Deleting the read of `TOKEN_FILE_ENV` is not a one-line change: the type's
whole reason for having two tiers, a cache, a JWT-`exp` parser and an
`invalidate` method was the projected-file tier. With it gone the type
collapses to a single static value, so this section deletes rather than
patches.

**Delete:**

- `TOKEN_FILE_ENV` (:66) and its doc.
- `Tier::ProjectedFile { path, cache }` (:162-165) and `Cached` (:154-157).
- `TinyhumansTokenSource::projected_file` (:180-187).
- The `if let Some(path) = token_file { … }` branch of `from_parts`
  (:224-235), and its `token_file: Option<&Path>` parameter.
- `token_file()` (:273-278).
- The `Tier::ProjectedFile` arm of `describe()` (:284-286), `tier()`
  (:262), `credential_source()`'s indirection through `tier()` if `TokenTier`
  collapses to one variant (see below), `hash_identity`'s `0u8` arm
  (:302-305), and `invalidate()`'s body (:317-321; keep the method as a
  no-op so callers that still call it — `provider.rs`'s 401 handling — need
  no edit, or delete every caller too; **prefer keeping it a no-op**, since
  deleting the caller touches turn-retry code this slice has no reason to
  open).
- The cache-window machinery `MAX_CACHE_WINDOW`, `TTL_FRACTION`,
  `unverified_jwt_exp`, `cache_window` (find with
  `git grep -n "fn unverified_jwt_exp\|fn cache_window\|MAX_CACHE_WINDOW\|TTL_FRACTION" src/company/credentials.rs`)
  once nothing but the deleted branch calls them.
- Every test in `credentials.rs`'s `#[cfg(test)] mod test` that builds a
  `TinyhumansTokenSource::projected_file` or sets `TOKEN_FILE_ENV`. List them
  first with `git grep -n "projected_file\|TOKEN_FILE_ENV" src/company/credentials.rs`
  and delete only the ones in the test module; the production sites are
  covered above.

**Keep, simplified:**

```rust
/// How this instance obtains its TinyHumans credential — a static value held
/// for the life of the process. The platform-projected-token-file tier was
/// removed in the keys rework (issue #2306 phase 6a): every hosted tenant now
/// needs a company-level `provider/tinyhumans/key` (2a) instead. See
/// not-handled.md's TINYHUMANS_TOKEN_FILE entry for the migration this implies.
pub struct TinyhumansTokenSource {
    token: String,
}

impl TinyhumansTokenSource {
    pub fn static_key(token: impl Into<String>) -> Self {
        Self { token: token.into() }
    }

    /// `None` when `API_KEY_ENV` is unset or blank.
    pub fn from_env(env: &dyn EnvSource) -> Option<Self> {
        Self::from_parts(env.get(API_KEY_ENV).as_deref())
    }

    pub fn from_parts(api_key: Option<&str>) -> Option<Self> {
        let key = api_key?.trim();
        (!key.is_empty()).then(|| Self::static_key(key))
    }

    pub fn source_of_parts(has_static_credential: bool) -> CredentialSource {
        if has_static_credential { CredentialSource::Static } else { CredentialSource::None }
    }

    pub fn credential_source(&self) -> CredentialSource {
        CredentialSource::Static
    }

    pub fn describe(&self) -> String {
        "static (set)".to_string()
    }

    pub fn hash_identity<H: std::hash::Hasher>(&self, hasher: &mut H) {
        use std::hash::Hash;
        self.token.hash(hasher);
    }

    pub fn invalidate(&self) {}

    pub async fn current(&self) -> Result<String> {
        Ok(self.token.clone())
    }
}
```

**A real design choice, not a detail:** whether `TokenTier` and
`CredentialSource::Attested` are deleted too, or kept with `Attested`
permanently unreachable. Keeping `Attested` unreachable is smaller (nothing
that serialises it — DTOs, the `credential_source` wire enum — needs to
change) but leaves a dead wire value. Deleting it is more thorough but
touches every `match` on `CredentialSource` (`git grep -n "CredentialSource::" src`
finds them) and the frontend type that mirrors it
(`git grep -n "attested" frontend/src`). **Recommendation: keep `Attested` and
`TokenTier::ProjectedFile` as unreachable-but-typed for this slice** — the
console still needs to render *some* value for a company that has not
restarted since a rollback (a config file from before 6a would resolve
differently on an old binary; the wire enum should not need to change with
the binary's own env), and re-derive whether to remove them entirely as a
follow-up once no `RuntimeConfig` on any deployed binary can still produce
`attested`. State this choice in the PR body either way.

Continued in [part 2](phase-6a-remove-env-vars-part2.md) (§6 `RuntimeConfig` /
doctor / `AppConfig`, §7 `OPENCOMPANY_COMPOSIO_BACKEND_URL`, §8 the E2E fixture
replacement, §9 docs, §10 tests, §11 must-not-touch, §12 done-when, §13
gotchas).

<!-- TODO(handover): every `file:line` across both parts of this slice was
     produced by a research subagent reading `upstream/main @ fcfb3e1bc` and
     then assembled into prose by a second pass — neither step was
     independently re-verified by the orchestrating agent line-by-line the way
     phase-1c's evidence blocks were (each with its own `git grep` output
     pasted in). Before an implementer relies on this slice, re-run the
     `git grep` commands this file already names (§0's two checks, §1's file
     list, part 2 §7's `backend_url_or_default(` grep, part 2 §8's
     `managesLiveLlm\|managesFixtures` grep, part 2 §12's final grep) and
     confirm each still matches. No section is known-wrong; this is an
     unverified-provenance flag, not a specific defect. Also: phase-6.md's
     "Commit order" section pointed at a non-existent "§7 commit order" in
     this file (§7 is actually `OPENCOMPANY_COMPOSIO_BACKEND_URL`) — fixed to
     point at part 2 §12 "Done when" instead, but neither part actually spells
     out a numbered commit sequence the way phase-1.md/phase-3.md/phase-5.md
     do for their own slices. Add one (mirroring those files' "## Commit
     order" sections) before treating 6a as ready to implement. -->

## 14. Commit order (added at handover; verify against §7's grep-based file
list before relying on the exact split)

1. `src/harness/built_in/provider.rs` + `src/company/inference.rs`: §3's
   `hosted_endpoint_from_env` / `EnvDefault` edit, plus tests.
2. `src/company/credentials.rs`: §5's `TinyhumansTokenSource` collapse to
   static-only, plus its test-module deletions.
3. `src/app/config.rs` + `src/app/doctor.rs` + `src/app/types.rs`: part 2 §6.
4. `src/bin/opencompany.rs`: §4.
5. Composio (`src/company/composio.rs` and its five call sites): part 2 §7.
6. `frontend/playwright.config.ts` + every affected E2E spec: part 2 §8 — its
   own text warns this may be large enough to be its own sub-task; split
   further if so, and report to the operator rather than shipping it partial.
7. Docs: part 2 §9.

Run `cargo fmt --all -- --check` before each Rust commit and the three
frontend typecheck gates before the E2E commit, per the folder's standing
rule. Push after each commit; verify CI by head SHA before starting the next.
