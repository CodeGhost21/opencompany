# Phase 2a — TinyHumans on the proxy, part 2

This is the continuation of [part 1](phase-2a-tinyhumans-on-proxy.md). Section
numbers carry on from part 1. `file:line` references are on
`upstream/main @ fcfb3e1bc` (2026-09-14).

---

## 5.10 Target code (console)

### `frontend/src/inference/catalogue.ts`

Append this after the `modelscope` entry (:229-235), as the **last** array
entry:

```ts
  {
    slug: "tinyhumans",
    label: "TinyHumans",
    endpoint: "https://api.tinyhumans.ai/agent-integrations/openrouter",
    auth: "bearer",
    keyPlaceholder: "th-...",
  },
```

Follow the editing rules at :11-23:

- fields in the order slug, label, endpoint, auth, keyPlaceholder;
- double quotes;
- no comments inside the entry.

Then two comment edits:

- `:53`: "The 26 hosted providers" becomes "The 27 hosted providers".
- `:448-456`: add "`tinyhumans` is also a cloud row now; it stays here so
  reservation does not depend on the table."

`INTERNAL_SLUGS` itself (:457) is unchanged.

### `frontend/src/inference/connect.ts`

- **Delete** `offersManaged` (:127-154) with its doc comment.
- **Delete** the `managedEntry` block in `addOptions` (:172-182). Change the
  function to take one parameter:

  ```ts
  export function addOptions(providers: readonly Provider[]): AddOptions {
    return {
      cloud: CLOUD_PROVIDERS.filter((p) => !isConnected(providers, p.slug)).map((p) => ({
        value: p.slug,
        label: p.label,
        detail: endpointHost(p.endpoint),
      })),
      local: /* unchanged */,
      cli: /* unchanged */,
    };
  }
  ```

  Rewrite the doc comment (:156-167) to add: "TinyHumans is offered once, as
  its catalogue row, and hidden once any row has slug `tinyhumans`, whether an
  added row or entry zero on a managed config."
- **Delete** `if (optionSlug === MANAGED_OPTION_SLUG) return null;` (:254).
- **Delete** the `if (optionSlug === MANAGED_OPTION_SLUG) { … }` branch in
  `credentialAsk` (:295-305).

  Both deleted lines are now unreachable, because `cloudProvider("tinyhumans")`
  matches first. The cloud branch then yields title "Connect TinyHumans",
  `needsKey: true`, `needsEndpoint: false`, `keyPlaceholder: "th-..."`.
- **Keep** `export const MANAGED_OPTION_SLUG = "tinyhumans";` (:125). Rewrite
  its doc to "The TinyHumans catalogue slug; also the legacy Managed row's
  slug."
- Remove the `ManagedState` import (:24) if nothing else in the file uses it.

### `frontend/src/inference/AddProviderDialog.tsx`

Remove the `managed` prop:

- the destructuring at :62;
- the prop type at :68-69;
- the `ManagedState` import at :24.

Call `addOptions(providers)` at :73.

### `frontend/src/api/inference.ts`

Add this to `ManagedState` (:210-236), after `configured`:

```ts
  /**
   * Whether the separate legacy Managed row renders. False when `providers`
   * already lists slug `tinyhumans`. Optional: an older host does not send it,
   * and absent reads as `configured`, which is what the row always keyed on.
   */
  legacyRow?: boolean;
```

### `frontend/src/inference/ProviderList.tsx`

1. Add an exported pure function below `MANAGED_SLUG` (:68):

   ```ts
   /** Whether the legacy Managed row renders: the chain resolves AND no row is listed for it. */
   export function showsLegacyManagedRow(managed: ManagedState | undefined): boolean {
     return managed?.configured === true && managed.legacyRow !== false;
   }
   ```
2. Replace `{managed?.configured && (` at :253 with
   `{managed && showsLegacyManagedRow(managed) && (`.

Leave the empty-state gate at :217 alone. With `providers` empty, nothing is
listed, so `legacyRow` equals `configured`.

### `frontend/src/inference/ProvidersTab.tsx`

1. Delete the branch at :219-222 (`else if (draft.kind === MANAGED_OPTION_SLUG)
   { await actions.saveManagedKey(…) }`). A `tinyhumans` draft now takes the
   ordinary path: draft probe (:229-242), model step, `actions.add(draft)`.
2. Remove `managed={state.status?.managed}` from `<AddProviderDialog>` (:389).
3. Pass the replace note to `<ProviderConnectDialog>` (:400-411):

   ```tsx
   replacesKey={
     connecting === MANAGED_OPTION_SLUG &&
     editing === null &&
     state.status?.managed?.source === "provider_key"
   }
   ```

Leave these as they are:

- **`onManagedReplaceKey` (:342-345)** still opens `MANAGED_OPTION_SLUG`. That
  now opens the TinyHumans catalogue add, so "Add a key" / "Replace key" on the
  legacy row turns it into a normal row.
- **`onManagedRemoveKey`, the managed `RemoveProviderDialog` (:455-474),
  `onManagedToggle` and `onManagedTest`** still call the legacy routes.

### `frontend/src/inference/ProviderConnectDialog.tsx`

1. Add the prop to the props type (~:100-133): `replacesKey?: boolean;`.
   Destructure it with default `false`.
2. Directly under the API key `<Input>` inside the `ask.needsKey` block
   (:281-296), render:

   ```tsx
   {replacesKey && (
     <p className="text-xs text-muted-foreground" data-testid="inference-connect-replaces-key">
       A key is already saved for Managed. Connecting TinyHumans replaces it with this one.
     </p>
   )}
   ```

Keep the "Or connect your TinyHumans account" block (:345-361). It is still
true. The model field (:305-336) needs no change. TinyHumans reaches it because
the probe classifies its catalog as needing a model (§12.5).

---

## 6. Ordered edit list

Make one commit per row, in this order. Run `cargo fmt --all -- --check` before
each Rust commit and the three frontend gates before each console commit. Push
after each commit.

| # | Commit subject | Sections | Includes tests (§8) |
|---|---|---|---|
| 1 | `Add a paged catalog parser for the TinyHumans proxy` | 5.2 | 8.1 `paged_catalog.rs` |
| 2 | `Read model catalogs in the shape each provider publishes` | 5.1 (enum + fn + const only), 5.3, 5.4, 5.5 | 8.1 `catalogue.rs` shape test, `probe.rs`, `inference_models.rs` |
| 3 | `Add TinyHumans to the provider catalogue` | 5.1 row + docs, 5.6, catalogue.ts row, `docs/modules/inference/catalogue.md` | 8.1 catalogue + providers + resolution + parser; 8.2 catalogue |
| 4 | `Show exactly one TinyHumans row on the LLM page` | 5.7, 5.10 (connect, dialogs, list, tab, api type) | 8.1 status tests; 8.2 managed-row, connect, one-row render; 8.3 e2e |
| 5 | **GATED:** `Point the managed endpoint constants at the OpenRouter proxy` | 5.9 | 8.1 3b checks |

For commit 3, update `docs/modules/inference/catalogue.md`:

- `:19`: "26 user-addable entries" becomes "27".
- Add a row 27 to the table at `:50`: `tinyhumans` | TinyHumans | bearer |
  `th-...` | `https://api.tinyhumans.ai/agent-integrations/openrouter`.
- `:52-56`: add a sentence saying TinyHumans is a row with a paged catalog
  (`CatalogShape::PagedEnvelope`), while openhuman's `openhuman` entry is still
  not ported.

After commit 4, stop and raise the PR as a draft, unless the gate in §12.1 has
been answered.

---

## 7. Data carry-over

- **Stored keys: none.** Nothing is copied, renamed or cleared at boot. No new
  key is created.
- **A company whose legacy Managed row reads `provider/tinyhumans/key`** keeps
  that row and that key unchanged until someone adds TinyHumans.
- **Adding TinyHumans needs the key typed into the dialog, like any provider.**
  The add overwrites `provider/tinyhumans/key` (`providers.rs:352-362`). The
  dialog says so beforehand (`inference-connect-replaces-key`). On success:
  - the record is written;
  - `legacyRow` becomes false;
  - the legacy row disappears, leaving one row.

  If the add is rolled back, §5.6 (c) restores the previous key.
- **Why the add does not silently reuse the stored key:**
  - The host never returns a credential to the console. Reusing it would need a
    host-side copy route, which is a new surface that item 12 owns.
  - "Type the key, pick the model" is the flow item 14 asks for, the same as
    OpenRouter.
  - The slot is one address, so after the overwrite the legacy chain (for
    explicit `managed` routes) and the new row present the **same** key. Nothing
    diverges.
- **Entry zero `inference/config = {provider: managed}`** is untouched. The add
  is refused, and the entry-zero row is the TinyHumans row.
- **Health:** `inference/health["tinyhumans"]` is one slot. The legacy Managed
  Test and the new row both write it, and only one of the two rows renders at a
  time.

---

## 8. Tests

The fake key everywhere is `th-not-a-real-key` and the model is
`openai/gpt-4o-mini`.

### 8.1 Rust

**`src/company/inference/paged_catalog.rs`** (`mod tests`):

- `the_page_path_asks_for_the_maximum_page`: `page_path(0)` equals
  `"/models?limit=500&offset=0"`.
- `a_page_is_unwrapped_and_unknown_fields_are_ignored`: a body with `object`,
  `pricing`, `supports_tools`, `input_modalities` and `display_name`. Assert the
  ids, `name`, `context_length`, `raw_len` and `total`.
- `an_openai_shaped_body_is_an_error_not_an_empty_page`: parsing
  `{"data":[{"id":"openai/gpt-4o-mini"}]}` gives `Err` containing "envelope".
- `success_false_is_an_error_carrying_the_reason`.
- `a_malformed_entry_is_dropped_but_still_advances_paging`: ids `42`, `"   "`
  and a string `context_length`.
- `paging_follows_total_and_stops_there`: two pages, total 3.
- `a_clamped_limit_costs_requests_not_models`.
- `an_empty_page_ends_a_read_whose_total_is_never_reached`.
- `a_page_with_no_total_is_the_whole_answer`.
- `duplicates_across_pages_are_kept_once`.
- `the_page_bound_reports_truncation_instead_of_looping`: 20 pushes with
  total 1,000,000 give `Truncated { read: 20, total: 1_000_000 }`.

**`src/company/inference/catalogue.rs`:**

- `the_catalogue_ships_the_counts_the_plan_names` (:1077): 26 becomes 27.
- Rename `managed_owns_its_slug_even_though_it_is_not_a_catalogue_row`
  (:1257-1271) to `tinyhumans_owns_its_slug_and_is_a_catalogue_row`. Keep the
  three `is_reserved_slug` asserts. Change the last line to
  `assert!(cloud_provider("tinyhumans").is_some());` and update its comment.
- New `tinyhumans_is_a_bearer_row_on_the_proxy`: endpoint equals the proxy URL,
  `auth_style_for("tinyhumans") == AuthStyle::Bearer`, placeholder `th-...`.
- New `catalog_shape_is_paged_only_for_tinyhumans_or_the_proxy_path`:
  - `("tinyhumans", <proxy>)` → Paged;
  - `(" tinyhumans ", "")` → Paged;
  - `("openrouter", "https://api.tinyhumans.ai/agent-integrations/openrouter/")`
    → Paged;
  - `("openrouter", "https://api.tinyhumans.ai/openai/v1")` → OpenAi (a
    literal, not the constant);
  - `("custom", "http://127.0.0.1:8099/v1")` → OpenAi;
  - every other `CLOUD_PROVIDERS` row with its own endpoint → OpenAi.
- **These pass unchanged; re-run them:**
  - `anthropic_is_the_only_non_bearer_entry_in_the_catalogue` (:1651);
  - `every_entry_has_a_parseable_endpoint_and_a_known_auth_style` (:1084);
  - `the_console_mirror_lists_the_same_cloud_providers` (:1478), once the TS row
    is appended last;
  - the `endpoint_is_chat_completions_only` asserts (:1184-1196). That function
    has no caller outside these tests (`git grep`), and after this change
    `api.tinyhumans.ai` counts as a chat-only host.

**`src/company/inference/probe.rs`:** add
`a_paged_probe_reads_every_page_with_the_bearer`. It uses a loopback `axum`
router on `127.0.0.1:0`, on the same pattern as the other loopback tests:

- **Mock:** serve `GET /agent-integrations/openrouter/models`. Offset 0 returns
  2 ids with total 3; any later offset returns 1 id.
- **Call:** `probe_models(base, Some(FAKE), AuthStyle::Bearer, LOCAL_OFFERED,
  CatalogShape::PagedEnvelope)`.
- **Assert:**
  - 3 ids come back;
  - 2 requests were made, with queries `limit=500&offset=0` and then
    `limit=500&offset=2`;
  - both carried `Authorization: Bearer th-not-a-real-key`.

Also add `a_paged_probe_that_gets_an_openai_body_is_unknown_not_auth`: it fails
with `class == ProbeClass::Unknown`.

**`src/server/inference_models.rs`:** add a helper
`spawn_proxy_catalog(respond: fn(usize) -> (u16, String)) -> (String, Seen)`
serving `/agent-integrations/openrouter/models`, then these tests:

- `the_paged_catalog_is_read_to_total_with_the_bearer`: two pages.
- `a_259_model_catalog_arrives_in_one_page_at_limit_500`: 259 generated ids
  plus `openai/gpt-4o-mini`, `total: 260`. Assert one request and 260 models.
- `a_503_before_the_snapshot_loads_is_memoized_not_empty`:
  `catalog_cache_scoped(&shaped_endpoint(&base, Paged), Some(scope))
  .lookup_failure(now).is_some()`.
- `a_401_or_403_on_the_paged_catalog_is_not_memoized`: loop over both statuses;
  `lookup_failure` is `None`.
- `an_openai_shaped_answer_is_not_read_as_the_paged_catalog`: error contains
  "envelope".
- `a_paged_failure_never_names_the_endpoint_credential`: base with
  `alice:hunter2@`.
- `an_oversized_page_is_refused_not_buffered`: body length
  `PAGE_BODY_CAP + 1`.
- `one_url_read_in_two_shapes_is_two_cache_slots`.

**`src/company/inference.rs`:** add
`a_tinyhumans_row_resolves_direct_on_its_own_key_and_model`.

- **Setup:**
  - `store::put_provider` a `tinyhumans` draft from `catalogue::cloud_provider("tinyhumans")`;
  - `models` = the four tiers → `openai/gpt-4o-mini`;
  - key set at `provider_key_key("tinyhumans")`.
- **Assert:**
  - `base_url` is the proxy;
  - `!decl.is_proxied()`;
  - the bearer is the fake key;
  - `model_for_tier("agentic-v1", &decl.models, decl.vocabulary())` is
    `openai/gpt-4o-mini`.

Add a second case that stores `tinyhumans/key` (the account) and **no** row
key, and asserts the row's bearer is `None`. This is D-set: there is no fallback
for a row.

**`src/harness/built_in/provider.rs`:** add
`a_proxy_reply_with_extra_top_level_and_usage_keys_parses`.

- **Payload:**
  - `choices[0].message.content = "pong"`;
  - top-level `service_tier`, top-level `openhuman: {billing, usage}`;
  - `usage.cost` and `usage.is_byok`.
- **Assert** through `model_response_from_payload`: text `pong`, input tokens
  12, output tokens 3.

There is no streaming test (§5.9).

**`src/server/ops/inference.rs`** (tests; helpers `home()` :1407,
`state_with_company` :1476, `send` :1812, `runtime_with`):

- `adding_tinyhumans_over_a_legacy_managed_config_is_refused_and_writes_nothing`.
  1. `send PUT /api/v1/company/inference {"provider":"managed"}`.
  2. `send POST …/inference/providers {"kind":"tinyhumans","key":FAKE,"model":"openai/gpt-4o-mini"}`.
  3. Assert `400` and a body containing "TinyHumans is already connected. Edit
     the existing row rather than adding a second one."
  4. Assert the raw body does not contain `FAKE`.
  5. `GET` status. Exactly one `providers[]` entry has slug `tinyhumans`, with
     `origin == "entryZero"` and no `health`, and `managed.legacyRow == false`.
- `adding_tinyhumans_without_a_key_is_refused`: `{"kind":"tinyhumans"}` gives
  400 "TinyHumans needs an API key."; no `tinyhumans` entry in `providers`.
- `a_listed_tinyhumans_row_hides_the_legacy_managed_row`: `runtime_with`, then
  `store::put_provider` the row, set its key, and call
  `effective_status_with(&runtime, None, false)`. Assert:
  - one `tinyhumans` provider, `origin == "indexed"`;
  - `dto.managed.configured`;
  - `dto.managed.source == "provider_key"`;
  - `!dto.managed.legacy_row`.
- `an_account_key_only_company_shows_the_legacy_managed_row`: set only
  `tinyhumans/key` (`company_key::KEY_KEY`). Assert no `tinyhumans` provider,
  `source == "company_account"`, `legacy_row == true`.
- `a_company_with_nothing_shows_no_legacy_row`: `configured == false` and
  `legacy_row == false`.

**`src/server/ops/inference/providers.rs`** (tests, :2110):

`restore_previous_key_puts_back_the_replaced_key_and_none_does_nothing`:

- `runtime_with`, then set the slot to `"th-new-not-a-real-key"`;
- call `restore_previous_key(…, Some("th-not-a-real-key"))`; the slot reads the
  old value;
- call it with `None`; the slot is unchanged.

Rollback cannot be driven end to end on a host test, because the TinyHumans
endpoint is a fixed internet URL. Record that in the PR description as an
intentionally untested edge.

**Existing tests to change:** only the two catalogue tests above. Any compile
error from the new `shape` parameter is fixed by adding `CatalogShape::OpenAi`.
After gated 3b, run `git grep -n 'openai/v1' -- src` and check that every
remaining hit is on the §5.9 "stay" list.

### 8.2 Frontend unit (`frontend/test/unit/`)

**`inference-catalogue.test.ts`:**

- `toHaveLength(26)` becomes 27.
- New `it("lists TinyHumans once, as a bearer row on the proxy")`:
  - `CLOUD_PROVIDERS.filter((p) => p.slug === "tinyhumans")` has length 1;
  - its endpoint is the proxy URL and its auth `"bearer"`;
  - `isReservedSlug("tinyhumans")` is true.
- The non-bearer list stays `["anthropic"]`.

**`inference-managed-row.test.ts`:**

- Delete the `describe("where managed is offered")` block, together with the
  `offersManaged` and `MANAGED_OPTION_SLUG` imports.
- Add `describe("TinyHumans is offered once, as its catalogue row")`:
  - `addOptions([]).cloud.filter((o) => o.value === "tinyhumans")` has length 1,
    with `{label: "TinyHumans", detail: "api.tinyhumans.ai"}`;
  - it is hidden when `providers` holds a `tinyhumans` row with
    `origin: "indexed"`;
  - it is hidden when that row has `origin: "entryZero"`.
- Add `describe("showsLegacyManagedRow")`:
  - configured with `legacyRow` false → false;
  - configured with `legacyRow` true → true;
  - configured with `legacyRow` absent (older host) → true;
  - not configured → false.
- The remaining describes stay.

**`inference-connect.test.ts`:**

- `addOptions([provider("groq")])` still has `CLOUD_PROVIDERS.length - 1` cloud
  options (:48); no change.
- Add `credentialAsk("tinyhumans")` → `{title: "Connect TinyHumans", needsKey:
  true, needsEndpoint: false, keyPlaceholder: "th-..."}`.
- Add `probeEndpoint("tinyhumans")` → the proxy URL.

**New file `inference-tinyhumans-one-row.test.ts`:** use
`renderToStaticMarkup(createElement(ProviderList, props))`, as in
`inference-hub-account-links.test.ts`. The fixture `Provider` fills every
required field of `frontend/src/inference/types.ts`, which `typecheck:unit`
enforces. Handlers are `() => {}`. `testState` returns `{ kind: "idle" }` and
`routingState` returns `null`.

- `it("a fresh company that added TinyHumans renders exactly one TinyHumans row")`:
  - **Props:** providers `[tinyhumans indexed]`; managed `{source:
    "provider_key", configured: true, legacyRow: false, baseUrl: <proxy>}`.
  - **Assert:** the markup has exactly one `data-testid="inference-provider-tinyhumans"`
    and no `data-testid="inference-provider-managed"`.
- `it("a company with only an account key renders the legacy Managed row")`:
  - **Props:** providers `[]`; managed `{source: "company_account",
    configured: true, legacyRow: true, …}`.
  - **Assert:** it contains `inference-provider-managed` and not
    `inference-provider-tinyhumans`.

### 8.3 E2E (`frontend/test/e2e/inference.spec.ts`)

Add `test("TinyHumans is added through its catalogue row and shows as exactly one row")`.

The add probe goes to `api.tinyhumans.ai`, which CI cannot reach with a real
key, so this test stubs **the console's API calls only**, with `page.route`, on
the pattern of `brain-virtualization.spec.ts:48`:

1. **Stub the probe.** `**/api/v1/company/inference/probe` →
   `{ok: true, modelCount: 260, needsModel: true, models: [259 ids "vendor/model-001"… plus "openai/gpt-4o-mini"]}`.
2. **Stub the add.** On `POST **/api/v1/company/inference/providers`:
   - record the request body;
   - `const real = await (await route.fetch({ url: <same origin>/api/v1/company/inference, method: "GET" })).json();`
   - push a `tinyhumans` provider (`origin: "indexed"`, `models` with the four
     tiers, `keyConfigured: true`, `health: {state: "ok", at: …}`);
   - if `real.managed` is present, set `real.managed.legacyRow = false`;
   - fulfill `{status: real, note: "TinyHumans is connected and answering."}`;
   - from then on, fulfill `GET **/api/v1/company/inference` with that same
     status.
3. **Drive the page:**
   - `openInference(page)`;
   - `choose(page, "cloud", "TinyHumans")`;
   - fill `#inference-connect-key` with `th-not-a-real-key`;
   - click `inference-connect-submit`.
4. **Assert the model step:**
   - `inference-connect-model` is visible;
   - `#inference-connect-model-options option` has count 260.
5. **Pick the model:** fill `inference-connect-model` with
   `openai/gpt-4o-mini`, then submit.
6. **Assert the request:** the recorded body has `kind: "tinyhumans"`,
   `model: "openai/gpt-4o-mini"`, `key: "th-not-a-real-key"`.
7. **Assert the page:** `inference-provider-tinyhumans` has count 1 and
   `inference-provider-managed` has count 0.

State in the test comment that it is console-only. The host logic is covered by
§8.1.

The existing test "Managed is a connected row only when its chain actually
resolves" (:96-127) stays unchanged. On the live-brain lane the chain resolves
through the instance and no row is listed, so `legacyRow` is true.

---

## 9. Console / UI

- **Add dialog:** a Cloud entry "TinyHumans", detail `api.tinyhumans.ai`. The
  "Managed (TinyHumans)" entry is gone.
- **Connect dialog title:** "Connect TinyHumans". The key placeholder is
  `th-...`.
- **Replace note:** "A key is already saved for Managed. Connecting TinyHumans
  replaces it with this one." (`inference-connect-replaces-key`,
  `text-muted-foreground`).
- **Model step:** existing field `inference-connect-model`, datalist
  `inference-connect-model-options`, existing copy (`ProviderConnectDialog.tsx:330-334`).
- **Row:** existing `ProviderRow`, `data-testid="inference-provider-tinyhumans"`,
  label "TinyHumans", sub-line host `api.tinyhumans.ai`, health chip
  `inference-provider-tinyhumans-health`.
- **Legacy row:** `inference-provider-managed`. It renders only when
  `showsLegacyManagedRow`.
- **Tokens only:** the classes used above already exist. No raw hex;
  `scripts/ci/assert-design-tokens.sh` must pass.

---

## 10. Must not touch

- **The legacy managed chain:**
  - `load_managed_key` (`inference.rs:936`);
  - `managed_source` (:1079);
  - `managed_identity` (:1116);
  - `resolve_endpoint` (:644-694);
  - `decl_for_probe`;
  - `is_managed_choice` (:399) and `normalize_provider` (:382).
- **Legacy routes:** `set_managed_key` (`providers.rs:1697-1777`),
  `set_managed_enabled` (:1461), and `test_managed` apart from the one shape
  argument.
- **`inference/managed/enabled`**, and every routing piece:
  - `resolve.rs`;
  - the routes handlers;
  - `auto_route_sole_provider`;
  - `managed_parked_tiers`.
- **Tier handling and the store:**
  - `DEFAULT_TIER_MODELS`, `TierVocabulary`, `model_for_tier`,
    `tier_overrides`;
  - `store.rs` (`put_provider`, `list_providers`, `legacy_slot_is_managed`).
- **Other surfaces:**
  - Composio, the Account page, `company_key.rs`;
  - `TINYHUMANS_TOKEN_FILE`;
  - `vendor/`.
- **E2E hosts:** `frontend/playwright.config.ts` env, `mock-brain.mjs`,
  `live-brain-proxy.mjs`.
- **Other agents' work:** every other file in `docs/key-reworks/`, and any
  worktree other than the one you hold.

---

## 11. Done when

1. Commits 1–4 (§6) are pushed.
2. **Rust:** `cargo fmt --all -- --check` is clean locally. Clippy and tests
   are verified **on CI by head SHA**:
   - `gh api "repos/tinyhumansai/opencompany/actions/runs?head_sha=$SHA"` gives
     the run id;
   - `gh api …/actions/runs/$RUN/jobs` must show zero failures **and** zero
     pending;
   - **both Console E2E lanes must be completed**, not just absent.

   Never trust `gh pr checks`.
3. **Frontend:** each of `npm run typecheck`, `npm run typecheck:unit` and
   `npm run typecheck:e2e` is run and named; `scripts/ci/assert-design-tokens.sh`
   passes.
4. **Browser evidence** on a claimed port, verified with
   `local/wt ports --verify`, in light and dark, with screenshots:
   - the add flow's model step with the full 259-id list (a real key from the
     environment, never on disk; or the §8.3 stub);
   - the Connected card with exactly one TinyHumans row;
   - a company with only an account key showing the legacy Managed row.
5. The PR description lists the untested edge (§8.1, rollback end to end) and
   says there is no streaming path (§5.9).
6. E2E fixture hosts keep `OPENCOMPANY_INFERENCE_URL=http://…/v1`. They are
   unaffected by commit 5, so neither lane needs a config change.
7. Commit 5 is either absent, with a line in the PR description saying so, or
   present **with** the operator's answer to §12.1 quoted.

---

## 12. Gotchas and stop points

### 12.1 STOP — before commit 5 (3b)

Ask the operator, and do not guess. After the constant flip, legacy managed
traffic hits the proxy with tier names. That traffic is:

- a company with no row that resolves through the managed chain;
- the setup wizard's Managed Test (`provider::probe`, model `chat-v1`);
- the setup brain (`DEFAULT_HOSTED_MODEL = "chat-v1"`);
- every hosted tenant that does not get `OPENCOMPANY_INFERENCE_URL` injected.

The proxy answers 400 "not a valid OpenRouter slug". Options:

- **(A) Hold commit 5 until slice 2d** (tier names never on the wire) gives the
  legacy path a model. Recommended; it matches D-legacy.
- **(B) Flip now, and have the manager inject
  `OPENCOMPANY_INFERENCE_URL=https://api.tinyhumans.ai/openai/v1`** for hosted
  tenants, so they keep today's endpoint. Self-hosted and docker-dev instances
  on `TINYHUMANS_API_KEY` still break.
- **(C) Flip now and accept the 400s.** Hosted tenants lose their brain.

The manager-side prerequisite in every case: hosted tenants run on whatever
`OPENCOMPANY_INFERENCE_URL` the manager injects, verbatim. Moving them to the
proxy means injecting the proxy URL **and** a model source (2d). That is a
change in `opencompany-microservices`, not in this repo.

### 12.2 The row resolves direct, never through the managed chain

`is_managed_choice("tinyhumans")` is **true** (`inference.rs:399-401`). Any code
that sends a row's kind through `resolve_endpoint` or `decl_for_probe` would
send it to the platform URL with the chain's identities. Indexed rows go through
`decl_for_indexed`, which states `proxied = false` (:1431-1437). Keep it that
way, and do not add `is_managed_choice` checks on the row path.

### 12.3 One key slot, two readers

Adding the row writes `provider/tinyhumans/key`, which is step 1 of the legacy
chain. Afterwards `managed.configured` is true (source `provider_key`). So:

- `auto_route_sole_provider` does not route (`managed_resolves`,
  `providers.rs:577`);
- unset routes resolve to the primary provider, which is the row when it is the
  only enabled one or the marked default;
- explicit `managed` routes keep using the chain, on the same key.

This is expected and needs no code. Removing the row clears the slot
(`delete_provider`), and with it the legacy step 1. Item 12's banner owns what
happens next.

### 12.4 Health is shared

The legacy Managed Test and the row both write `inference/health["tinyhumans"]`.
Only one of them renders, so there is no conflict. Do not add a second slot.

### 12.5 The model step appears because of evidence, not the slug

The proxy catalog has no tier names and none of the four `DEFAULT_TIER_MODELS`
ids (live, 2026-09-14). `TierVocabulary::from_catalog_ids` therefore returns
`Unknown` and `needs_an_explicit_model` returns true (`providers.rs:668-670`).

- **Risk:** if the proxy ever lists `qwen/qwen3.8-max` (the vision id), it
  returns `Concrete`, `needsModel` becomes false, and the dialog skips the
  field.
- **Before commit 4:** check the live list contains none of the four ids.
- **If one appears:** STOP and report. Slice 2c makes the step unconditional.
  Do not special-case the slug here.

### 12.6 Metering is not handled

The row is not proxied (`proxied = false`), and the cost comments say the
`/openai/v1` passthrough reports no USD (`cost.rs:87`, `:229`). The proxy does
return `usage.cost` and `openhuman.billing`. Whether the metering layer reads
them for the row is **not** in this slice. Record it in `not-handled.md` through
the main session; do not edit that file yourself.

### 12.7 The probe cap is enough

`PROBE_CATALOGUE_LIMIT` is 500 (`providers.rs:268`), so all 259 ids reach the
datalist. The paged success body is capped at 4 MiB (`PAGE_BODY_CAP`), not the
probe's 64 KiB (`probe.rs:653`). A 500-entry page would overflow 64 KiB.

### 12.8 Row order is load-bearing

`the_console_mirror_lists_the_same_cloud_providers` (`catalogue.rs:1478`)
compares the Rust and TS tables entry by entry. Put `tinyhumans` last in
**both**.

### 12.9 Claims in this doc: verified vs inferred

- **Verified live on 2026-09-14** (by the parent session): the proxy facts,
  namely 259 ids, the envelope, the 400s and the SSE shape.
- **Verified by reading `fcfb3e1bc`:** every `file:line` above.
- **Inferred, not browser-checked:** that hosted tenants get no
  `OPENCOMPANY_INFERENCE_URL` (from `CLAUDE.md`), and the "two rows today"
  entry-zero case in part 1 §1.
