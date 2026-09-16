use super::*;

#[test]
fn the_catalogue_ships_the_counts_the_plan_names() {
    assert_eq!(CLOUD_PROVIDERS.len(), 27, "cloud providers");
    assert_eq!(LOCAL_RUNTIMES.len(), 3, "local runtimes");
    assert_eq!(CLI_LOGINS.len(), 2, "CLI logins");
}

#[test]
fn every_entry_has_a_parseable_endpoint_and_a_known_auth_style() {
    for provider in CLOUD_PROVIDERS {
        assert!(!provider.slug.is_empty(), "empty slug");
        assert!(!provider.label.is_empty(), "{}: empty label", provider.slug);
        assert!(
            provider.endpoint.starts_with("https://"),
            "{}: cloud endpoints are https",
            provider.slug
        );
        assert!(
            endpoint_host(provider.endpoint).is_some(),
            "{}: endpoint has no parseable host",
            provider.slug
        );
        assert!(
            matches!(provider.auth, AuthStyle::Bearer | AuthStyle::Anthropic),
            "{}: a cloud provider authenticates",
            provider.slug
        );
    }
    for runtime in LOCAL_RUNTIMES {
        assert!(!runtime.slug.is_empty(), "empty slug");
        assert!(!runtime.label.is_empty(), "{}: empty label", runtime.slug);
        if let Some(endpoint) = runtime.default_endpoint {
            assert!(
                endpoint_host(endpoint).is_some(),
                "{}: default endpoint has no parseable host",
                runtime.slug
            );
        }
    }
}

#[test]
fn slugs_are_unique_across_the_whole_catalogue() {
    let mut seen: Vec<&str> = Vec::new();
    for slug in CLOUD_PROVIDERS
        .iter()
        .map(|p| p.slug)
        .chain(LOCAL_RUNTIMES.iter().map(|r| r.slug))
    {
        assert!(!seen.contains(&slug), "duplicate slug {slug}");
        seen.push(slug);
    }
}

#[test]
fn anthropic_is_the_only_non_bearer_cloud_entry() {
    let anthropic: Vec<&str> = CLOUD_PROVIDERS
        .iter()
        .filter(|p| p.auth == AuthStyle::Anthropic)
        .map(|p| p.slug)
        .collect();
    assert_eq!(anthropic, vec!["anthropic"]);
}

#[test]
fn minimax_keeps_the_openai_surface_that_is_the_fix() {
    // Reverting to `/anthropic` 404s both chat and the model listing. The
    // comment on the row says why; this makes reverting it fail loudly.
    let minimax = cloud_provider("minimax").expect("minimax is in the catalogue");
    assert_eq!(minimax.endpoint, "https://api.minimax.io/v1");
    assert_eq!(minimax.auth, AuthStyle::Bearer);
}

#[test]
fn the_managed_first_party_backend_is_not_a_row() {
    // openhuman's 27th entry is their own managed backend. Ours is modelled
    // separately, with its own auth path; a row here would give it a second
    // bearer-shaped identity.
    assert!(cloud_provider("openhuman").is_none());
}

#[test]
fn endpoint_host_drops_scheme_userinfo_port_and_path() {
    assert_eq!(
        endpoint_host("https://api.openai.com/v1").as_deref(),
        Some("api.openai.com")
    );
    assert_eq!(
        endpoint_host("api.groq.com/openai/v1").as_deref(),
        Some("api.groq.com")
    );
    assert_eq!(
        endpoint_host("http://user:pw@host.example:8080/v1").as_deref(),
        Some("host.example")
    );
    assert_eq!(
        endpoint_host("http://[::1]:11434/v1").as_deref(),
        Some("::1")
    );
    assert_eq!(
        endpoint_host("HTTPS://API.OpenAI.com/v1").as_deref(),
        Some("api.openai.com")
    );
    assert_eq!(endpoint_host("   "), None);
}

#[test]
fn only_openai_serves_the_responses_api_fallback() {
    assert!(!endpoint_is_chat_completions_only(
        "https://api.openai.com/v1"
    ));
    // The custom-slug gap: a user-defined slug aimed at a known chat-only
    // host still must not try `/responses`.
    assert!(endpoint_is_chat_completions_only(
        "https://integrate.api.nvidia.com/v1"
    ));
    assert!(endpoint_is_chat_completions_only(
        "https://api.groq.com/openai/v1"
    ));
    // A genuinely unknown proxy keeps the permissive fallback.
    assert!(!endpoint_is_chat_completions_only(
        "https://proxy.acme.dev/v1"
    ));
}

#[test]
fn azure_is_detected_by_host_including_subdomains_and_sovereign_clouds() {
    assert!(is_azure_endpoint(
        "https://my-resource.openai.azure.com/openai/v1"
    ));
    assert!(is_azure_endpoint(
        "https://r.services.ai.azure.com/openai/v1"
    ));
    assert!(is_azure_endpoint(
        "https://r.cognitiveservices.azure.com/openai/v1"
    ));
    assert!(is_azure_endpoint("https://r.openai.azure.us/openai/v1"));
    assert!(is_azure_endpoint("https://r.openai.azure.cn/openai/v1"));
    assert!(!is_azure_endpoint("https://api.openai.com/v1"));
}

#[test]
fn the_foundry_serverless_hosts_are_deliberately_not_azure() {
    // Classifying these would relabel a correct model id as a deployment
    // name — the exact confusion the Azure rule exists to prevent. This test
    // is the guard against adding them back for symmetry.
    assert!(!is_azure_endpoint(
        "https://r.inference.ai.azure.com/models"
    ));
    assert!(!is_azure_endpoint("https://r.models.ai.azure.com/models"));
}

#[test]
fn a_suffix_that_merely_ends_with_an_azure_host_is_not_azure() {
    // `notopenai.azure.com.evil.test` must not match, and neither must a
    // host that merely ends in the same characters without a dot boundary.
    assert!(!is_azure_endpoint("https://myopenai.azure.comx/v1"));
    assert!(!is_azure_endpoint("https://openai.azure.com.evil.test/v1"));
}

#[test]
fn codex_stores_under_openai_so_its_connected_check_can_match() {
    let codex = cli_login("codex").expect("codex is a CLI login");
    assert_eq!(codex.stored_slug, "openai");
    assert!(codex.probes);
    let claude = cli_login("claude-code").expect("claude-code is a CLI login");
    assert_eq!(claude.stored_slug, "claude-code");
    // No key changes hands, so there is nothing for a probe to present.
    assert!(!claude.probes);
}

#[test]
fn reserved_slugs_cover_cloud_local_and_stored_cli_names() {
    assert!(is_reserved_slug("openrouter"));
    assert!(is_reserved_slug("ollama"));
    assert!(is_reserved_slug("claude-code"));
    // Codex stores under `openai`, which is already reserved as a cloud row.
    assert!(is_reserved_slug("openai"));
    assert!(!is_reserved_slug("acme-gateway"));
}

#[test]
fn tinyhumans_owns_its_slug_and_is_a_catalogue_row() {
    // `provider/tinyhumans/key` is where the managed credential lives, and
    // `managed` is the word the route grammar uses. A custom provider named
    // "TinyHumans" slugified straight into the first: adding it stored a
    // vendor key where managed reads it, a managed test presented that key
    // to the platform endpoint, and removing the custom row took managed's
    // credential with it.
    assert!(is_reserved_slug(super::super::MANAGED_SLUG));
    assert!(is_reserved_slug("tinyhumans"));
    assert!(is_reserved_slug("managed"));
    // Keys rework (#2306) slice 2a: unlike before, `tinyhumans` IS now a
    // catalogue row too — reservation and catalogue membership are no
    // longer mutually exclusive for this slug.
    assert!(cloud_provider("tinyhumans").is_some());
}

#[test]
fn tinyhumans_is_a_bearer_row_on_the_proxy() {
    let tinyhumans = cloud_provider("tinyhumans").expect("tinyhumans is in the catalogue");
    assert_eq!(
        tinyhumans.endpoint,
        "https://api.tinyhumans.ai/agent-integrations/openrouter"
    );
    assert_eq!(auth_style_for("tinyhumans"), AuthStyle::Bearer);
    assert_eq!(tinyhumans.key_placeholder, Some("th-..."));
}

#[test]
fn catalog_shape_is_paged_only_for_tinyhumans_or_the_proxy_path() {
    let proxy = "https://api.tinyhumans.ai/agent-integrations/openrouter";
    assert_eq!(
        catalog_shape_for("tinyhumans", proxy),
        CatalogShape::PagedEnvelope
    );
    // Whitespace around the kind, and a blank base URL, still recognise
    // the kind on its own.
    assert_eq!(
        catalog_shape_for(" tinyhumans ", ""),
        CatalogShape::PagedEnvelope
    );
    // A trailing slash on the proxy path still matches.
    assert_eq!(
        catalog_shape_for("openrouter", &format!("{proxy}/")),
        CatalogShape::PagedEnvelope
    );
    assert_eq!(
        catalog_shape_for("openrouter", "https://api.tinyhumans.ai/openai/v1"),
        CatalogShape::OpenAi
    );
    assert_eq!(
        catalog_shape_for("custom", "http://127.0.0.1:8099/v1"),
        CatalogShape::OpenAi
    );
    for provider in CLOUD_PROVIDERS.iter().filter(|p| p.slug != "tinyhumans") {
        assert_eq!(
            catalog_shape_for(provider.slug, provider.endpoint),
            CatalogShape::OpenAi,
            "{}: only tinyhumans/the proxy path pages",
            provider.slug
        );
    }
}

#[test]
fn a_kind_lands_in_the_category_its_scrub_rule_needs() {
    assert_eq!(category_of("openrouter"), Category::Cloud);
    assert_eq!(category_of("ollama"), Category::Local);
    assert_eq!(category_of("lmstudio"), Category::Local);
    assert_eq!(category_of("claude-code"), Category::Cli);
    // Codex stores under `openai`, and `openai` is a cloud row in its own
    // right — so the shared slug stays Cloud. Reading it as a CLI login
    // would give the OpenAI row the CLI's slug-less scrub rule and orphan
    // every route naming it.
    assert_eq!(category_of("openai"), Category::Cloud);
    assert_eq!(category_of("acme-gateway"), Category::Cloud);
}

/// `needs_key` is enforced by the host, so a wrong `true` is not a cosmetic
/// defect — it makes the runtime unaddable. None of the three projects called
/// "omlx" requires a key, and two have no auth mechanism at all, so no local
/// runtime may demand one.
#[test]
fn no_local_runtime_demands_a_key() {
    let with_keys: Vec<&str> = LOCAL_RUNTIMES
        .iter()
        .filter(|r| r.needs_key)
        .map(|r| r.slug)
        .collect();
    assert!(
        with_keys.is_empty(),
        "a local runtime that demands a key cannot be added at all: {with_keys:?}"
    );
}

#[test]
fn no_other_module_writes_its_own_openrouter_attribution_headers() {
    // The failure this guards is not a wrong value, it is a SECOND value.
    // `harness::built_in::provider` and `harness::roster_build` each spelled
    // the referer out, with different hosts, so one company's turn traffic
    // and its roster-build traffic reached OpenRouter's dashboard as two
    // apps. Both copies looked right in isolation, which is why reading
    // either one never found it.
    //
    // So the assertion is about shape rather than content: any file that
    // mentions the header must reach this constant for its value.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            if path.file_name().and_then(|n| n.to_str()) == Some("catalogue.rs") {
                continue;
            }
            // Both headers, not just the first. `X-Title` has the same
            // divergence mode — a module spells it out with a value of its
            // own and nothing notices — and checking one of a pair is how
            // the guard ends up proving less than it appears to.
            let spells_out_referer =
                source.contains("\"HTTP-Referer\"") && !source.contains("OPENROUTER_REFERER");
            let spells_out_title =
                source.contains("\"X-Title\"") && !source.contains("OPENROUTER_TITLE");
            if spells_out_referer || spells_out_title {
                offenders.push(path.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these files write an OpenRouter attribution header without reading \
         `catalogue::OPENROUTER_REFERER`, which is how the two copies \
         diverged the first time: {offenders:?}"
    );
}

// ── The cross-language check ────────────────────────────────────────────
//
// The console needs this table too, and TypeScript cannot read a Rust
// `const`. So there are two copies, and the only thing that keeps two
// hand-maintained copies honest is something that fails when they disagree.
// openhuman has the same two copies and no such test, which is listed as a
// defect for exactly that reason; this repo had six copies and two had
// already silently drifted.
//
// The reader below is deliberately strict — one field per line, quoted
// values, no comments inside an entry — and the mirror's header says so. A
// forgiving parser would let the file drift into a shape the test quietly
// stops checking, which is worse than no test because it reads as coverage.

/// The repository root, which is **not** `CARGO_MANIFEST_DIR`.
///
/// The package manifest lives in `crates/opencompany-core/` while `src/`,
/// `tests/` and `frontend/` stayed at the repository root and are reached
/// from it with `../../` paths. So a repo-relative path joined onto
/// `CARGO_MANIFEST_DIR` lands in a directory that does not exist, and these
/// tests failed with "No such file or directory" rather than on anything
/// they meant to check.
///
/// Walking up to the first ancestor that actually holds the console mirror
/// keeps this correct under both that layout and a single root crate,
/// rather than hard-coding a `../..` that one of the two would get wrong.
fn repo_root() -> std::path::PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|dir| dir.join("frontend/src/inference/catalogue.ts").is_file())
        .unwrap_or(manifest)
        .to_path_buf()
}

fn mirror_source() -> String {
    let path = repo_root().join("frontend/src/inference/catalogue.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("console mirror at {} is unreadable: {e}", path.display()))
}

/// The text between `export const <name> … = [` and the closing `];`.
fn array_body<'a>(src: &'a str, name: &str) -> &'a str {
    let decl = format!("export const {name}");
    let start = src
        .find(&decl)
        .unwrap_or_else(|| panic!("the mirror declares no {name}"));
    let open = src[start..]
        .find('[')
        .unwrap_or_else(|| panic!("{name} is not an array literal"))
        + start
        + 1;
    let close = src[open..]
        .find("\n];")
        .unwrap_or_else(|| panic!("{name} is not terminated by a bare `];`"))
        + open;
    &src[open..close]
}

/// Object literals in an array body, as ordered `(field, value)` lists.
fn object_entries(body: &str) -> Vec<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let mut current: Option<Vec<(String, String)>> = None;
    for line in body.lines() {
        let line = line.trim();
        if line == "{" {
            current = Some(Vec::new());
            continue;
        }
        if line == "}," || line == "}" {
            if let Some(fields) = current.take() {
                entries.push(fields);
            }
            continue;
        }
        let Some(fields) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once(':') else {
            panic!("mirror entry line is not `field: value` — {line:?}");
        };
        let value = value.trim().trim_end_matches(',').trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        fields.push((key.trim().to_string(), value.to_string()));
    }
    assert!(
        current.is_none(),
        "an unterminated object literal in the mirror"
    );
    entries
}

/// Bare quoted strings in an array body.
fn string_entries(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',');
            line.strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_string)
        })
        .collect()
}

/// A field of a parsed entry, or `None` when the mirror omits it.
fn field<'a>(entry: &'a [(String, String)], name: &str) -> Option<&'a str> {
    entry
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// A field the mirror must carry.
fn required<'a>(entry: &'a [(String, String)], name: &str, row: usize) -> &'a str {
    field(entry, name).unwrap_or_else(|| panic!("mirror row {row} has no `{name}`: {entry:?}"))
}

#[test]
fn the_console_mirror_lists_the_same_cloud_providers() {
    let src = mirror_source();
    let entries = object_entries(array_body(&src, "CLOUD_PROVIDERS"));
    assert_eq!(
        entries.len(),
        CLOUD_PROVIDERS.len(),
        "the mirror lists {} cloud providers, Rust lists {}",
        entries.len(),
        CLOUD_PROVIDERS.len()
    );
    for (row, (entry, provider)) in entries.iter().zip(CLOUD_PROVIDERS).enumerate() {
        assert_eq!(
            required(entry, "slug", row),
            provider.slug,
            "row {row} slug"
        );
        assert_eq!(
            required(entry, "label", row),
            provider.label,
            "{}: label",
            provider.slug
        );
        assert_eq!(
            required(entry, "endpoint", row),
            provider.endpoint,
            "{}: endpoint",
            provider.slug
        );
        assert_eq!(
            required(entry, "auth", row),
            provider.auth.as_str(),
            "{}: auth style",
            provider.slug
        );
        assert_eq!(
            field(entry, "keyPlaceholder"),
            provider.key_placeholder,
            "{}: key placeholder",
            provider.slug
        );
    }
}

#[test]
fn the_console_mirror_lists_the_same_local_runtimes() {
    let src = mirror_source();
    let entries = object_entries(array_body(&src, "LOCAL_RUNTIMES"));
    assert_eq!(entries.len(), LOCAL_RUNTIMES.len(), "local runtime count");
    for (row, (entry, runtime)) in entries.iter().zip(LOCAL_RUNTIMES).enumerate() {
        assert_eq!(required(entry, "slug", row), runtime.slug, "row {row} slug");
        assert_eq!(
            required(entry, "label", row),
            runtime.label,
            "{}: label",
            runtime.slug
        );
        assert_eq!(
            field(entry, "defaultEndpoint"),
            runtime.default_endpoint,
            "{}: default endpoint",
            runtime.slug
        );
        assert_eq!(
            required(entry, "needsKey", row),
            if runtime.needs_key { "true" } else { "false" },
            "{}: needs a key",
            runtime.slug
        );
    }
}

#[test]
fn the_console_mirror_lists_the_same_cli_logins() {
    let src = mirror_source();
    let entries = object_entries(array_body(&src, "CLI_LOGINS"));
    assert_eq!(entries.len(), CLI_LOGINS.len(), "CLI login count");
    for (row, (entry, login)) in entries.iter().zip(CLI_LOGINS).enumerate() {
        assert_eq!(
            required(entry, "optionSlug", row),
            login.option_slug,
            "row {row} option slug"
        );
        // The Codex trap: this is the assertion that keeps the console's
        // already-connected check able to match at all.
        assert_eq!(
            required(entry, "storedSlug", row),
            login.stored_slug,
            "{}: stored slug",
            login.option_slug
        );
        assert_eq!(
            required(entry, "label", row),
            login.label,
            "{}: label",
            login.option_slug
        );
        assert_eq!(
            required(entry, "probes", row),
            if login.probes { "true" } else { "false" },
            "{}: probes",
            login.option_slug
        );
    }
}

#[test]
fn the_console_mirror_lists_the_same_azure_hosts() {
    let src = mirror_source();
    let hosts = string_entries(array_body(&src, "AZURE_ENDPOINT_HOSTS"));
    assert_eq!(
        hosts,
        AZURE_ENDPOINT_HOSTS
            .iter()
            .map(|h| h.to_string())
            .collect::<Vec<_>>(),
        "the Azure host lists have diverged — including, possibly, by adding \
         the Foundry serverless hosts to one side"
    );
}

#[test]
fn the_console_mirror_uses_the_same_copy() {
    let src = mirror_source();
    let start = src
        .find("export const COPY = {")
        .expect("the mirror declares no COPY");
    let open = src[start..].find('{').expect("COPY is not an object") + start + 1;
    let close = src[open..]
        .find("\n} as const;")
        .expect("COPY is not terminated by `} as const;`")
        + open;
    let mut pairs = Vec::new();
    for line in src[open..close].lines() {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        pairs.push((key.trim().to_string(), value.to_string()));
    }
    let lookup = |name: &str| -> String {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .unwrap_or_else(|| panic!("the mirror's COPY has no `{name}`"))
            .1
            .clone()
    };
    assert_eq!(lookup("groupCloud"), copy::GROUP_CLOUD);
    assert_eq!(lookup("groupLocal"), copy::GROUP_LOCAL);
    assert_eq!(lookup("groupCli"), copy::GROUP_CLI);
    assert_eq!(lookup("placeholderCloud"), copy::PLACEHOLDER_CLOUD);
    assert_eq!(lookup("placeholderLocal"), copy::PLACEHOLDER_LOCAL);
    assert_eq!(lookup("placeholderCli"), copy::PLACEHOLDER_CLI);
    assert_eq!(lookup("helperCloud"), copy::HELPER_CLOUD);
    assert_eq!(lookup("helperLocal"), copy::HELPER_LOCAL);
    assert_eq!(lookup("helperCli"), copy::HELPER_CLI);
    assert_eq!(lookup("detailLocal"), copy::DETAIL_LOCAL);
    assert_eq!(lookup("detailCli"), copy::DETAIL_CLI);
    assert_eq!(pairs.len(), 11, "a copy string was added to one side only");
}

// ---- auth style and endpoint normalisation ------------------------------

#[test]
fn anthropic_is_the_only_non_bearer_entry_in_the_catalogue() {
    // A port that assumes one auth style breaks exactly one provider, and it
    // is the one people try first. Worse, the rejection classifies as `auth`
    // — the one destructive class — so the connect flow would delete a key
    // that was never wrong.
    assert_eq!(auth_style_for("anthropic"), AuthStyle::Anthropic);
    for provider in CLOUD_PROVIDERS.iter().filter(|p| p.slug != "anthropic") {
        assert_eq!(
            auth_style_for(provider.slug),
            AuthStyle::Bearer,
            "{} should be bearer",
            provider.slug
        );
    }
}

#[test]
fn a_keyless_local_runtime_sends_no_auth_header_and_omlx_does() {
    assert_eq!(auth_style_for("ollama"), AuthStyle::None);
    assert_eq!(auth_style_for("lmstudio"), AuthStyle::None);
    // Still bearer, though omlx no longer *requires* a key: an operator
    // running `jundot/omlx --api-key` must still be able to authenticate.
    // This is the assertion that would have caught the auth style silently
    // becoming `None` as a side effect of correcting `needs_key`.
    assert_eq!(auth_style_for("omlx"), AuthStyle::Bearer);
    assert!(
        !local_runtime("omlx").expect("omlx row").needs_key,
        "accepting a key is not the same as demanding one"
    );
}

#[test]
fn an_unknown_kind_is_a_custom_openai_compatible_endpoint() {
    // Which by definition speaks bearer — that is what "OpenAI-compatible"
    // means in the field the operator typed it into.
    assert_eq!(auth_style_for("my-gateway"), AuthStyle::Bearer);
    assert_eq!(auth_style_for("custom"), AuthStyle::Bearer);
}

#[test]
fn a_bare_origin_gains_the_v1_an_openai_surface_lives_at() {
    // `http://localhost:11434` is what Ollama's own documentation prints,
    // and it is not where the OpenAI-compatible surface is.
    assert_eq!(
        normalize_local_endpoint("http://localhost:11434").as_deref(),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(
        normalize_local_endpoint("  http://localhost:11434/  ").as_deref(),
        Some("http://localhost:11434/v1")
    );
}

#[test]
fn a_path_the_operator_supplied_is_left_exactly_as_typed() {
    // Appending is not guessing. Someone who typed a path meant it.
    assert_eq!(
        normalize_local_endpoint("https://acme.example/api/gateway").as_deref(),
        Some("https://acme.example/api/gateway")
    );
    assert_eq!(
        normalize_local_endpoint("http://127.0.0.1:1234/v1/").as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );
}

#[test]
fn an_endpoint_carrying_a_credential_is_not_an_endpoint() {
    // The security half of the same refusal. `normalize_local_endpoint` is
    // the funnel every stored endpoint passes through, so refusing here is
    // what makes "no credential is ever stored in a `base_url`" a property
    // of the store rather than of whichever handler remembered to check.
    for bad in [
        "http://alice:hunter2@127.0.0.1:8597/v1",
        "https://alice@api.acme.example/v1",
        "http://alice:hunter2@127.0.0.1:8597",
        // A password may itself contain an `@`; the authority still has one.
        "http://alice:hun@ter2@127.0.0.1:8597/v1",
    ] {
        assert!(endpoint_has_credentials(bad), "`{bad}` carries userinfo");
        assert!(
            normalize_local_endpoint(bad).is_none(),
            "`{bad}` must not normalise into something storable"
        );
    }
}

#[test]
fn a_second_scheme_does_not_hide_the_credential_behind_it() {
    // Codex review on #2281: an uppercase scheme was once prefixed with a
    // second one by setup normalisation, and the first authority (`HTTP:`)
    // has no `@` — so reading only that one missed the credential entirely.
    for bad in [
        "http://HTTP://alice:hunter2@127.0.0.1:8597/v1",
        "https://http://alice@api.acme.example/v1",
    ] {
        assert!(endpoint_has_credentials(bad), "`{bad}` carries userinfo");
        assert!(
            normalize_local_endpoint(bad).is_none(),
            "`{bad}` must not be storable"
        );
        let said = redact_endpoint(bad);
        assert!(
            !said.contains("alice") && !said.contains("hunter2"),
            "`{bad}` redacted to `{said}`"
        );
    }
    // An uppercase scheme on its own is an ordinary endpoint.
    assert!(endpoint_has_credentials(
        "HTTP://alice:hunter2@127.0.0.1:8597/v1"
    ));
    assert!(!endpoint_has_credentials("HTTPS://api.acme.example/v1"));
}

#[test]
fn a_credential_is_found_in_every_authority_an_http_client_could_read() {
    // Codex and CodeRabbit review on #2281. Each shape once hid its
    // credential from the refusal, the redaction, or both.
    for (bad, said) in [
        // One slash: WHATWG URL parsing still reads the authority after it.
        (
            "http:/alice:hunter2@127.0.0.1:8597/v1",
            "http:/***@127.0.0.1:8597/v1",
        ),
        // Three slashes: the extra one is skipped, not an empty authority.
        (
            "http:///alice:hunter2@127.0.0.1:8597/v1",
            "http:///***@127.0.0.1:8597/v1",
        ),
        // Backslashes, which a special scheme reads as slashes.
        (
            "http:\\\\alice:hunter2@127.0.0.1:8597/v1",
            "http:\\\\***@127.0.0.1:8597/v1",
        ),
        // No slash at all.
        (
            "HTTP:alice:hunter2@127.0.0.1:8597/v1",
            "***@127.0.0.1:8597/v1",
        ),
        // Two authorities, two credentials: both go.
        (
            "http://alice:one@outer/http://bob:two@inner/v1",
            "http://***@outer/http://***@inner/v1",
        ),
    ] {
        assert!(endpoint_has_credentials(bad), "`{bad}` carries userinfo");
        assert!(
            normalize_local_endpoint(bad).is_none(),
            "`{bad}` must not be storable"
        );
        assert_eq!(redact_endpoint(bad), said, "`{bad}`");
    }
    // A path is still a path. A gateway that proxies to another URL, with
    // an `@` later in that path, carries no credential and stays storable.
    let gateway = "https://gateway.example/proxy/http://upstream/@me";
    assert!(!endpoint_has_credentials(gateway));
    assert_eq!(normalize_local_endpoint(gateway).as_deref(), Some(gateway));
    for good in [
        gateway,
        "http://127.0.0.1:8597/v1",
        "http://[::1]:11434/v1",
        "https://api.acme.example:8443/v1/@me",
        "localhost:1234/v1",
    ] {
        assert!(!endpoint_has_credentials(good), "`{good}` has no userinfo");
        assert_eq!(redact_endpoint(good), good);
    }
}

#[test]
fn a_scheme_in_a_well_formed_path_is_not_an_authority() {
    // Codex review on #2281: WHATWG parsing gives this endpoint no userinfo.
    // `http:user@example.com` is path text, so the endpoint is accepted.
    let gateway = "https://gateway.example/proxy/http:user@example.com/v1";
    assert!(!endpoint_has_credentials(gateway));
    assert_eq!(normalize_local_endpoint(gateway).as_deref(), Some(gateway));
    // What is *said* about it still masks the segment that looks like one:
    // the redaction reads wider than the refusal, by design.
    assert_eq!(
        redact_endpoint(gateway),
        "https://gateway.example/proxy/http:***@example.com/v1"
    );
    // A port-less host that happens to end in `:` does not start a hop.
    assert!(!endpoint_has_credentials("http://localhost:/v1/@me"));
    // Nor does a host *named* like a scheme with one slash after it: every
    // parser reads `http://http:/v1@beta` as host `http`, empty port, path
    // `/v1@beta`. It is storable; the wider redaction still masks the
    // lookalike when it is said.
    let empty_port = "http://http:/v1@beta";
    assert!(!endpoint_has_credentials(empty_port));
    assert_eq!(
        normalize_local_endpoint(empty_port).as_deref(),
        Some(empty_port)
    );
    assert_eq!(redact_endpoint(empty_port), "http://http:/***@beta");
    // And a doubled scheme still does — the one case a hop exists for.
    assert!(endpoint_has_credentials(
        "https://http://alice@api.acme.example/v1"
    ));
}

#[test]
fn tabs_and_line_breaks_do_not_hide_a_credential() {
    // Codex review on #2281: a URL parser removes ASCII tab, LF and CR
    // wherever they appear, so each of these reaches a client carrying
    // `alice:hunter2`.
    for bad in [
        "http:\t//alice:hunter2@127.0.0.1:8597/v1",
        "http://ali\nce:hunter2@127.0.0.1:8597/v1",
        "http://http:\t//alice:hunter2@127.0.0.1:8597/v1",
        "http://alice:hunter2\r@127.0.0.1:8597/v1",
    ] {
        assert!(endpoint_has_credentials(bad), "{bad:?} carries userinfo");
        assert!(
            normalize_local_endpoint(bad).is_none(),
            "{bad:?} must not be storable"
        );
        let said = redact_endpoint(bad);
        assert!(
            !said.contains("hunter2") && !said.contains("alice"),
            "{bad:?} redacted to {said:?}"
        );
    }
    assert_eq!(
        redact_endpoint("http:\t//alice:hunter2@127.0.0.1:8597/v1"),
        "http://***@127.0.0.1:8597/v1"
    );
}

#[test]
fn an_at_sign_in_the_path_is_not_a_credential() {
    // The `@` has to be inside the authority. A path may legitimately carry
    // one, and refusing those would reject perfectly good endpoints.
    for good in [
        "https://api.acme.example/v1/@me",
        "https://api.acme.example/v1?to=a@b",
        "https://api.acme.example/v1#a@b",
    ] {
        assert!(!endpoint_has_credentials(good), "`{good}` has no userinfo");
        assert_eq!(redact_endpoint(good), good);
    }
}

#[test]
fn redacting_an_endpoint_removes_the_credential_and_nothing_else() {
    // Observed in the incident: reqwest masks userinfo in its own error
    // Display (`for url (http://127.0.0.1:8597/v1/models)`), and then the
    // handler's own `format!` put it back from the endpoint we hold.
    assert_eq!(
        redact_endpoint("http://alice:hunter2@127.0.0.1:8597/v1"),
        "http://***@127.0.0.1:8597/v1"
    );
    assert_eq!(
        redact_endpoint("https://alice@api.acme.example/v1"),
        "https://***@api.acme.example/v1"
    );
    // The last `@` in the authority is the delimiter, so a password
    // containing one is removed whole rather than half-left behind.
    assert_eq!(
        redact_endpoint("http://alice:hun@ter2@127.0.0.1:8597/v1"),
        "http://***@127.0.0.1:8597/v1"
    );
    // Scheme-less, as `normalize_setup_base_url` accepts.
    assert_eq!(
        redact_endpoint("alice:hunter2@localhost:1234/v1"),
        "***@localhost:1234/v1"
    );
    // Nothing to redact: byte-for-byte the same endpoint, trimmed.
    assert_eq!(
        redact_endpoint("  https://api.openai.com/v1  "),
        "https://api.openai.com/v1"
    );
}

#[test]
fn only_http_and_https_are_endpoints() {
    // Rejected here rather than at the probe, because this is the one
    // category whose endpoint the operator types — and the connect flow's
    // ordering says reject before any write.
    for bad in [
        "file:///etc/passwd",
        "ftp://acme.example/v1",
        "localhost:11434",
        "",
        "   ",
        "http://",
    ] {
        assert!(
            normalize_local_endpoint(bad).is_none(),
            "`{bad}` is not an endpoint"
        );
    }
}
/// The defect: `output_modalities` and `limit` were applied on the
/// authenticated path only, so the connect probe and the post-404 fallback
/// took OpenRouter's defaults — text-only, capped at 500 — and a company
/// whose `vision-v1` tier needs a vision model saw a picker with none.
#[test]
fn every_openrouter_catalogue_read_asks_for_the_whole_catalogue() {
    for endpoint in [
        "https://openrouter.ai/api/v1",
        "https://openrouter.ai/api/v1/",
        "https://eu.openrouter.ai/api/v1",
    ] {
        let query = catalog_query(endpoint);
        assert!(
            query.contains("output_modalities=all"),
            "{endpoint} would silently drop every non-text model"
        );
        assert!(
            query.contains("limit=1000"),
            "{endpoint} would truncate at OpenRouter's default of 500"
        );
    }
    // The authenticated path already asked for both; the point is that the
    // two now agree rather than each carrying its own copy.
    let scoped = scoped_catalog_path("https://openrouter.ai/api/v1", true).expect("scoped");
    for parameter in ["output_modalities=all", "limit=1000"] {
        assert!(scoped.contains(parameter), "{scoped}");
        assert!(catalog_query("https://openrouter.ai/api/v1").contains(parameter));
    }
}

#[test]
fn a_non_openrouter_endpoint_gets_no_query_string() {
    // These parameters are OpenRouter's, not the OpenAI dialect's. Fireworks
    // rejects unknown fields outright and several hosts 400 on an
    // unrecognised query, so this must not become a blanket addition.
    for endpoint in [
        "https://api.anthropic.com/v1",
        "http://localhost:11434/v1",
        "https://api.groq.com/openai/v1",
    ] {
        assert_eq!(catalog_query(endpoint), "", "{endpoint}");
    }
}

#[test]
fn only_openrouters_own_host_gets_the_account_scoped_catalogue() {
    assert!(is_openrouter_endpoint("https://openrouter.ai/api/v1"));
    assert!(is_openrouter_endpoint("https://openrouter.ai/api/v1/"));
    assert!(is_openrouter_endpoint("https://eu.openrouter.ai/api/v1"));
    // The platform proxy fronts OpenRouter and serves the same catalogue,
    // but the account behind it is the server's, not the tenant's — and it
    // is a different host, which is the whole point of matching on one.
    assert!(!is_openrouter_endpoint(
        "https://api.tinyhumans.ai/openai/v1"
    ));
    assert!(!is_openrouter_endpoint(
        "https://openrouter.ai.example.com/v1"
    ));
    assert!(!is_openrouter_endpoint("http://127.0.0.1:11434/v1"));
}

#[test]
fn the_scoped_catalogue_needs_both_the_host_and_a_credential() {
    let path = scoped_catalog_path(super::super::OPENROUTER_BASE_URL, true)
        .expect("OpenRouter with a key reads the account-scoped list");
    assert!(path.starts_with("/models/user"));
    // `output_modalities` defaults to `text`, so leaving it off silently
    // drops every image, audio and embedding model.
    assert!(path.contains("output_modalities=all"), "{path}");
    assert!(path.contains("limit=1000"), "{path}");

    // Account-scoping is a question about a key. With none there is nothing
    // to scope to, and the public registry is the honest answer.
    assert!(scoped_catalog_path(super::super::OPENROUTER_BASE_URL, false).is_none());
    // Host-specific on purpose, the same way the Azure deployment-name rule
    // is. Every other provider has its own account restrictions or none.
    assert!(scoped_catalog_path("https://api.openai.com/v1", true).is_none());
    assert!(scoped_catalog_path("https://api.tinyhumans.ai/openai/v1", true).is_none());
}
