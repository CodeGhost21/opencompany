//! The store, against an in-memory port. No host, no network.
//!
//! The cases that must never regress are the convergence ones: an existing
//! company keeps working untouched, its first save moves it, and a second
//! provider's credential is genuinely a second credential rather than the same
//! slot under a new name — which is the bug the whole rework exists to fix.
//!
//! Index/config semantics: single-provider and multi-provider convergence,
//! endpoint round-tripping, and legacy flat-key migration.

use super::*;

#[derive(Default)]
struct MemSecrets {
    map: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[async_trait::async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, _company: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|value| SecretValue(value.clone())))
    }
    async fn set(&self, _company: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

fn company() -> CompanyId {
    CompanyId::new("acme")
}

async fn seed(secrets: &MemSecrets, pairs: &[(&str, &str)]) {
    for (key, value) in pairs {
        secrets
            .set(&company(), key, SecretValue((*value).to_string()))
            .await
            .expect("seed");
    }
}

fn stored_index(secrets: &MemSecrets) -> serde_json::Value {
    let raw = secrets
        .map
        .lock()
        .unwrap()
        .get(PROVIDER_INDEX_KEY)
        .cloned()
        .expect("index written");
    serde_json::from_str(&raw).expect("index is JSON")
}


#[tokio::test]
async fn a_company_with_nothing_configured_has_no_providers() {
    let secrets = MemSecrets::default();
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn the_legacy_flat_keys_are_read_as_entry_zero() {
    // An existing company must keep working with nothing written and nothing
    // moved. This is the whole argument for convergence over migration.
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[(PROVIDER_SECRET, "exa"), (API_KEY_SECRET, "exa-key")],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].slug, "exa");
    assert!(providers[0].enabled);
    assert!(
        provider_key_configured(&company(), &secrets, "exa")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn managed_and_unknown_slugs_are_never_synthesised_into_a_row() {
    // Neither is a connection, so neither is a row to render or to resolve.
    for slug in ["managed", "google", ""] {
        let secrets = MemSecrets::default();
        seed(&secrets, &[(PROVIDER_SECRET, slug)]).await;
        assert!(
            list_providers(&company(), &secrets)
                .await
                .unwrap()
                .is_empty(),
            "{slug}"
        );
    }
}

#[tokio::test]
async fn saving_entry_zeros_key_moves_it_and_clears_the_flat_address() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[(PROVIDER_SECRET, "exa"), (API_KEY_SECRET, "exa-key")],
    )
    .await;

    store_provider_key(&company(), &secrets, "exa", "exa-key-2")
        .await
        .unwrap();

    assert_eq!(
        secrets.map.lock().unwrap().get(API_KEY_SECRET).cloned(),
        Some(String::new()),
        "the flat key must be CLEARED, not merely shadowed — a key left behind is an \
         orphaned secret"
    );
    assert_eq!(
        load_provider_key(&company(), &secrets, "exa")
            .await
            .unwrap()
            .as_deref(),
        Some("exa-key-2")
    );
}

#[tokio::test]
async fn adding_a_second_provider_does_not_touch_entry_zeros_credential() {
    // The mirror image of the test above, and the more dangerous direction: a
    // write that cleared `search/api_key` unconditionally would destroy the
    // legacy provider's key while saving somebody else's.
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[(PROVIDER_SECRET, "exa"), (API_KEY_SECRET, "exa-key")],
    )
    .await;

    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    store_provider_key(&company(), &secrets, "brave", "brave-key")
        .await
        .unwrap();

    let slugs: Vec<String> = list_providers(&company(), &secrets)
        .await
        .unwrap()
        .into_iter()
        .map(|provider| provider.slug)
        .collect();
    assert_eq!(slugs, vec!["exa".to_string(), "brave".to_string()]);

    assert_eq!(
        load_provider_key(&company(), &secrets, "exa")
            .await
            .unwrap()
            .as_deref(),
        Some("exa-key")
    );
    assert_eq!(
        load_provider_key(&company(), &secrets, "brave")
            .await
            .unwrap()
            .as_deref(),
        Some("brave-key"),
        "two providers, two credentials — this is the bug the rework exists to fix"
    );
}

#[tokio::test]
async fn a_converged_entry_zero_is_not_listed_twice() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[(PROVIDER_SECRET, "exa"), (API_KEY_SECRET, "exa-key")],
    )
    .await;
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "exa".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 1, "{providers:?}");
}

#[tokio::test]
async fn deleting_a_provider_clears_its_credential() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    store_provider_key(&company(), &secrets, "brave", "brave-key")
        .await
        .unwrap();

    delete_provider(&company(), &secrets, "brave")
        .await
        .unwrap();

    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !provider_key_configured(&company(), &secrets, "brave")
            .await
            .unwrap(),
        "re-adding a provider must not silently reuse the key it had before"
    );
}

#[tokio::test]
async fn deleting_entry_zero_clears_the_flat_keys_too() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (PROVIDER_SECRET, "searxng"),
            (ENDPOINT_SECRET, "https://search.acme.internal"),
        ],
    )
    .await;

    delete_provider(&company(), &secrets, "searxng")
        .await
        .unwrap();

    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    for key in [PROVIDER_SECRET, API_KEY_SECRET, ENDPOINT_SECRET] {
        assert_eq!(
            secrets.map.lock().unwrap().get(key).cloned(),
            Some(String::new()),
            "{key}"
        );
    }
}

/// Keys rework (#2306), decision D-never-clear-default (X14, 2026-09-15):
/// this test used to be named `disabling_the_marked_provider_clears_the_marker`
/// and asserted the opposite. Disabling the marked provider now leaves
/// `search/default` exactly as it was — a confirmed disable is one decision,
/// not a second undocumented one to also retarget the default. Resolution
/// still degrades gracefully: `resolve::active` (see its own tests) falls
/// through a marked-but-disabled slug to the first usable candidate, or to
/// managed. What changes is only that the **stored marker** is no longer
/// silently rewritten.
#[tokio::test]
async fn disabling_the_marked_provider_never_clears_the_marker() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    set_default_slug(&company(), &secrets, "brave")
        .await
        .unwrap();
    store_provider_key(&company(), &secrets, "brave", "brave-not-a-real-key")
        .await
        .unwrap();

    set_enabled(&company(), &secrets, "brave", false)
        .await
        .unwrap();

    assert_eq!(
        load_default_slug(&company(), &secrets)
            .await
            .unwrap()
            .as_deref(),
        Some("brave"),
        "a disable must not rewrite search/default"
    );
    assert!(
        !list_providers(&company(), &secrets).await.unwrap()[0].enabled,
        "disabled is not deleted — the credential and the record stay"
    );
    // `|| true` made this unconditional, which is worse than no assertion at
    // all: it read as a check on the very property the test is named for. The
    // key now has to be there to be kept, so it is stored first.
    assert!(
        provider_key_configured(&company(), &secrets, "brave")
            .await
            .unwrap(),
        "disabling must not take the credential with it"
    );
}

/// Keys rework (#2306), decision D-never-clear-default (X14, 2026-09-15):
/// this test used to be named `deleting_the_marked_provider_clears_the_marker`
/// and asserted the opposite. There is no carve-out for delete versus
/// disable — a marker naming a deleted slug is exactly the state X14 asks
/// for, on the same footing as one naming a disabled slug (see the sibling
/// test above).
#[tokio::test]
async fn deleting_the_marked_provider_never_clears_the_marker() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    set_default_slug(&company(), &secrets, "brave")
        .await
        .unwrap();

    delete_provider(&company(), &secrets, "brave")
        .await
        .unwrap();

    assert_eq!(
        load_default_slug(&company(), &secrets)
            .await
            .unwrap()
            .as_deref(),
        Some("brave"),
        "a delete must not rewrite search/default, even though no row now answers to it"
    );
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty(),
        "the row itself is still gone"
    );
}

#[tokio::test]
async fn a_self_hosted_endpoint_round_trips_through_the_index_row() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "searxng".to_string(),
            enabled: true,
            endpoint: Some("https://search.acme.internal".to_string()),
        },
    )
    .await
    .unwrap();

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers[0].endpoint.as_deref(),
        Some("https://search.acme.internal")
    );
    assert_eq!(
        stored_index(&secrets)[0]["endpoint"],
        "https://search.acme.internal"
    );
    assert_eq!(
        secrets
            .map
            .lock()
            .unwrap()
            .get(&provider_endpoint_key("searxng"))
            .cloned(),
        Some("https://search.acme.internal".to_string())
    );
}

#[tokio::test]
async fn an_index_row_without_an_endpoint_reads_the_per_slug_address() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (PROVIDER_INDEX_KEY, r#"[{"slug":"searxng","enabled":true}]"#),
            (
                &provider_endpoint_key("searxng"),
                "http://old.acme.internal",
            ),
        ],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers[0].endpoint.as_deref(),
        Some("http://old.acme.internal")
    );
}

#[tokio::test]
async fn an_index_row_without_an_endpoint_falls_back_to_the_flat_address_for_entry_zero() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (PROVIDER_SECRET, "searxng"),
            (ENDPOINT_SECRET, "http://flat.acme.internal"),
            (
                PROVIDER_INDEX_KEY,
                r#"[{"slug":"searxng","enabled":false}]"#,
            ),
        ],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 1, "{providers:?}");
    assert_eq!(
        providers[0].endpoint.as_deref(),
        Some("http://flat.acme.internal")
    );
    assert!(!providers[0].enabled);
}

#[tokio::test]
async fn the_flat_address_is_never_read_for_a_slug_that_is_not_entry_zero() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (PROVIDER_SECRET, "exa"),
            (ENDPOINT_SECRET, "http://flat.acme.internal"),
            (PROVIDER_INDEX_KEY, r#"[{"slug":"searxng","enabled":true}]"#),
        ],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    let searxng = providers
        .iter()
        .find(|provider| provider.slug == "searxng")
        .expect("searxng row");
    assert_eq!(searxng.endpoint, None);
}

#[tokio::test]
async fn the_index_row_address_wins_over_the_per_slug_one() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (
                PROVIDER_INDEX_KEY,
                r#"[{"slug":"searxng","enabled":true,"endpoint":"http://row.acme.internal"}]"#,
            ),
            (
                &provider_endpoint_key("searxng"),
                "http://slug.acme.internal",
            ),
        ],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers[0].endpoint.as_deref(),
        Some("http://row.acme.internal")
    );
}

#[tokio::test]
async fn a_blank_address_in_the_index_row_reads_as_absent() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (
                PROVIDER_INDEX_KEY,
                r#"[{"slug":"searxng","enabled":true,"endpoint":"  "}]"#,
            ),
            (
                &provider_endpoint_key("searxng"),
                "http://slug.acme.internal",
            ),
        ],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers[0].endpoint.as_deref(),
        Some("http://slug.acme.internal")
    );
}

#[tokio::test]
async fn re_addressing_writes_the_index_row_and_the_per_slug_key() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "searxng".to_string(),
            enabled: true,
            endpoint: Some("http://old.acme.internal".to_string()),
        },
    )
    .await
    .unwrap();

    assert!(
        update_endpoint_if_present(
            &company(),
            &secrets,
            "searxng",
            Some("http://new.acme.internal".to_string()),
        )
        .await
        .unwrap()
    );

    assert_eq!(
        stored_index(&secrets)[0]["endpoint"],
        "http://new.acme.internal"
    );
    assert_eq!(
        secrets
            .map
            .lock()
            .unwrap()
            .get(&provider_endpoint_key("searxng"))
            .cloned(),
        Some("http://new.acme.internal".to_string())
    );
}

#[tokio::test]
async fn an_omitted_address_survives_toggle_select_reconnect_and_a_second_provider() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "searxng".to_string(),
            enabled: true,
            endpoint: Some("http://kept.acme.internal".to_string()),
        },
    )
    .await
    .unwrap();
    // Blank the per-slug key so only the index row can be the source of the
    // address below — otherwise the per-slug fallback would hide a broken merge
    // and every assertion here would pass for the wrong reason.
    seed(&secrets, &[(&provider_endpoint_key("searxng"), "")]).await;

    async fn assert_kept(secrets: &MemSecrets) {
        let providers = list_providers(&company(), secrets).await.unwrap();
        let searxng = providers
            .iter()
            .find(|provider| provider.slug == "searxng")
            .expect("searxng row");
        assert_eq!(
            searxng.endpoint.as_deref(),
            Some("http://kept.acme.internal"),
            "{providers:?}"
        );
        let index = stored_index(secrets);
        let row = index
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["slug"] == "searxng")
            .expect("searxng row in index");
        assert_eq!(row["endpoint"], "http://kept.acme.internal");
    }

    set_enabled(&company(), &secrets, "searxng", false)
        .await
        .unwrap();
    assert_kept(&secrets).await;

    select_provider(&company(), &secrets, "searxng", None, None)
        .await
        .unwrap();
    assert_kept(&secrets).await;
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .iter()
            .find(|provider| provider.slug == "searxng")
            .unwrap()
            .enabled
    );
    assert_eq!(
        load_default_slug(&company(), &secrets)
            .await
            .unwrap()
            .as_deref(),
        Some("searxng")
    );

    assert!(
        update_endpoint_if_present(&company(), &secrets, "searxng", None)
            .await
            .unwrap()
    );
    assert_kept(&secrets).await;

    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    assert_kept(&secrets).await;

    assert!(
        claim_provider(
            &company(),
            &secrets,
            SearchProvider {
                slug: "exa".to_string(),
                enabled: true,
                endpoint: None,
            },
        )
        .await
        .unwrap()
    );
    assert_kept(&secrets).await;
}

#[tokio::test]
async fn removing_a_provider_takes_its_address_with_it() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "searxng".to_string(),
            enabled: true,
            endpoint: Some("http://gone.acme.internal".to_string()),
        },
    )
    .await
    .unwrap();

    delete_provider(&company(), &secrets, "searxng")
        .await
        .unwrap();
    assert_eq!(stored_index(&secrets), serde_json::json!([]));
    assert_eq!(
        secrets
            .map
            .lock()
            .unwrap()
            .get(&provider_endpoint_key("searxng"))
            .cloned(),
        Some(String::new())
    );

    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "searxng".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();
    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers[0].endpoint, None,
        "a re-add must not resurrect the removed address: {providers:?}"
    );
}

#[tokio::test]
async fn an_index_blob_written_before_endpoint_existed_still_parses() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[(
            PROVIDER_INDEX_KEY,
            r#"[{"slug":"brave","enabled":true},{"slug":"exa"}]"#,
        )],
    )
    .await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 2, "{providers:?}");
    let exa = providers
        .iter()
        .find(|provider| provider.slug == "exa")
        .expect("exa row");
    assert!(exa.enabled);
    assert!(providers.iter().all(|provider| provider.endpoint.is_none()));
}

#[tokio::test]
async fn a_row_with_no_address_is_stored_without_the_field() {
    let secrets = MemSecrets::default();
    put_provider(
        &company(),
        &secrets,
        SearchProvider {
            slug: "brave".to_string(),
            enabled: true,
            endpoint: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        secrets.map.lock().unwrap().get(PROVIDER_INDEX_KEY).cloned(),
        Some(r#"[{"slug":"brave","enabled":true}]"#.to_string())
    );
}

#[tokio::test]
async fn a_blob_with_endpoint_parses_as_the_row_shape_before_it() {
    // Rollback safety: a binary from before this slice must still be able to
    // parse a blob this one wrote — it simply ignores the new field.
    #[derive(Debug, serde::Deserialize)]
    struct PreviousIndexEntry {
        slug: String,
        #[serde(default = "yes")]
        #[allow(dead_code)]
        enabled: bool,
    }

    let parsed = serde_json::from_str::<Vec<PreviousIndexEntry>>(
        r#"[{"slug":"searxng","enabled":true,"endpoint":"http://row.acme.internal"}]"#,
    );
    assert!(parsed.is_ok(), "{parsed:?}");
    assert_eq!(parsed.unwrap()[0].slug, "searxng");
}

#[tokio::test]
async fn switching_off_entry_zero_copies_its_flat_address_into_the_row() {
    let secrets = MemSecrets::default();
    seed(
        &secrets,
        &[
            (PROVIDER_SECRET, "searxng"),
            (ENDPOINT_SECRET, "http://flat.acme.internal"),
        ],
    )
    .await;

    set_enabled(&company(), &secrets, "searxng", false)
        .await
        .unwrap();

    assert_eq!(
        stored_index(&secrets)[0]["endpoint"],
        "http://flat.acme.internal"
    );
    assert_eq!(
        secrets.map.lock().unwrap().get(ENDPOINT_SECRET).cloned(),
        Some("http://flat.acme.internal".to_string()),
        "the flat key is a read fallback and must not be cleared here"
    );
    assert!(
        secrets
            .map
            .lock()
            .unwrap()
            .get(&provider_endpoint_key("searxng"))
            .is_none(),
        "switching off must not write the per-slug key when the address came from the flat one"
    );
}

#[tokio::test]
async fn an_unreadable_index_is_reported_rather_than_read_as_empty() {
    // Resolving a corrupt index to "no providers" would quietly move every agent
    // onto managed search and bill the platform for it.
    let secrets = MemSecrets::default();
    seed(&secrets, &[(PROVIDER_INDEX_KEY, "{not json")]).await;
    assert!(list_providers(&company(), &secrets).await.is_err());
}

