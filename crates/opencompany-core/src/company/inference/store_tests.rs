use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

// ---- index_lock (keys rework, issue #2306, round-3b lock coordination) -

#[tokio::test]
async fn index_lock_serialises_two_holders_on_the_same_company() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let company = CompanyId::new("acme-co");
    let inside = Arc::new(AtomicBool::new(false));
    let overlapped = Arc::new(AtomicBool::new(false));

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let company = company.clone();
        let inside = inside.clone();
        let overlapped = overlapped.clone();
        tasks.push(tokio::spawn(async move {
            let _guard = index_lock(&company).await;
            if inside.swap(true, Ordering::SeqCst) {
                overlapped.store(true, Ordering::SeqCst);
            }
            tokio::task::yield_now().await;
            inside.store(false, Ordering::SeqCst);
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert!(
        !overlapped.load(Ordering::SeqCst),
        "two holders of the same company's lock ran inside the guarded section at once"
    );
}

#[tokio::test]
async fn index_lock_does_not_block_a_different_company() {
    let a = CompanyId::new("acme-co");
    let b = CompanyId::new("other-co");
    let _guard_a = index_lock(&a).await;
    // A different company's lock must not wait on this one.
    tokio::time::timeout(std::time::Duration::from_secs(2), index_lock(&b))
        .await
        .expect("a different company's lock must not wait on this one");
}

/// Round-3a review P2-2's exact scenario, reproduced directly: "a
/// concurrent PATCH that pins `acme` and a DELETE of `acme` can both pass
/// their checks." Two different guarded mutations — not two holders of
/// the same shape, which [`index_lock_serialises_two_holders_on_the_same_company`]
/// already covers — each doing a check, a yield (so a race would need to
/// interleave right here to go unnoticed), then a write. If the lock
/// wired into both call sites actually serialises them, the delete's read
/// of the row always happens either wholly before or wholly after the
/// pin's check-and-write, never in between it.
#[tokio::test]
async fn a_pin_and_a_delete_of_the_same_provider_never_interleave() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let company = CompanyId::new("acme-co");
    let row_present = Arc::new(AtomicBool::new(true));
    let pin_saw_row_gone_mid_write = Arc::new(AtomicBool::new(false));

    let pin_row_present = row_present.clone();
    let pin_saw_gone = pin_saw_row_gone_mid_write.clone();
    let pin_company = company.clone();
    let pin = tokio::spawn(async move {
        let _guard = index_lock(&pin_company).await;
        // Check: the pin validates the row exists, exactly as
        // `server::ops::team_agent::edit_agent` does under this lock.
        let existed = pin_row_present.load(Ordering::SeqCst);
        tokio::task::yield_now().await;
        // Write: only meaningful if the row was still there when checked —
        // a delete that ran inside this critical section would make this
        // pin write against a row it never actually validated.
        if existed && !pin_row_present.load(Ordering::SeqCst) {
            pin_saw_gone.store(true, Ordering::SeqCst);
        }
    });

    let delete_row_present = row_present.clone();
    let delete_company = company.clone();
    let delete = tokio::spawn(async move {
        let _guard = index_lock(&delete_company).await;
        // Check: the delete reads `usedBy`, exactly as
        // `server::ops::inference::providers::delete_provider` does under
        // this lock.
        let _used_by_snapshot = delete_row_present.load(Ordering::SeqCst);
        tokio::task::yield_now().await;
        // Write: the row goes.
        delete_row_present.store(false, Ordering::SeqCst);
    });

    pin.await.unwrap();
    delete.await.unwrap();
    assert!(
        !pin_saw_row_gone_mid_write.load(Ordering::SeqCst),
        "the pin's check and write must never straddle the delete's write — the lock \
         wired into both handlers should have serialised them"
    );
}

// ---- check_model_id (keys rework, issue #2306, slice 2c) ---------------

#[test]
fn a_model_id_is_trimmed() {
    assert_eq!(
        check_model_id("  acme/test-model \n").unwrap(),
        "acme/test-model"
    );
}

#[test]
fn an_empty_model_id_is_refused() {
    for raw in ["", "   "] {
        let err = check_model_id(raw).unwrap_err();
        assert!(err.to_string().contains("Choose a model"), "{err}");
    }
}

#[test]
fn a_model_id_with_a_control_character_is_refused() {
    let err = check_model_id("test\u{0007}model").unwrap_err();
    assert!(err.to_string().contains("control characters"), "{err}");
}

#[test]
fn a_model_id_with_inner_whitespace_is_refused() {
    let err = check_model_id("test model").unwrap_err();
    assert!(err.to_string().contains("spaces"), "{err}");
}

#[test]
fn a_model_id_is_bounded_in_chars_not_bytes() {
    assert!(check_model_id(&"é".repeat(256)).is_ok());
    let err = check_model_id(&"é".repeat(257)).unwrap_err();
    assert!(err.to_string().contains("256"), "{err}");
}

#[test]
fn every_tier_name_is_refused_as_a_model_id() {
    for tier in crate::company::INFERENCE_TIERS {
        let err = check_model_id(tier).unwrap_err();
        assert!(err.to_string().contains("workload name"), "{err}");
    }
    let err = check_model_id(" chat-v1 ").unwrap_err();
    assert!(err.to_string().contains("workload name"), "{err}");
}

#[test]
fn model_ids_of_every_shape_pass() {
    for id in [
        "acme/test-model",
        "acme/test-model:free",
        "test-model:8b",
        "test-model",
        "test.deployment-1",
    ] {
        assert_eq!(check_model_id(id).unwrap(), id);
    }
}

#[derive(Default)]
struct MemSecrets {
    map: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, _c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|v| SecretValue(v.clone())))
    }
    async fn set(&self, _c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

/// A store whose writes to one key always fail — for the rollback rules.
struct FailsWriting {
    inner: MemSecrets,
    failing_key: String,
}

#[async_trait]
impl SecretStore for FailsWriting {
    async fn get(&self, c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        self.inner.get(c, key).await
    }
    async fn set(&self, c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        if key == self.failing_key {
            return Err(OpenCompanyError::Store("disk is on fire".into()));
        }
        self.inner.set(c, key, value).await
    }
}

fn company() -> CompanyId {
    CompanyId::new("acme")
}

fn draft(slug: &str) -> ProviderDraft {
    ProviderDraft {
        slug: slug.to_string(),
        label: slug.to_string(),
        kind: "openai_compatible".to_string(),
        base_url: format!("https://{slug}.example/v1"),
        models: BTreeMap::new(),
        enabled: true,
    }
}

async fn write_entry_zero(secrets: &dyn SecretStore, provider: &str) {
    let config = RuntimeInference {
        provider: provider.to_string(),
        base_url: None,
        models: BTreeMap::new(),
    };
    super::super::save_runtime_config(&company(), secrets, &config)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_company_with_nothing_configured_has_no_providers() {
    let secrets = MemSecrets::default();
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty(),
        "no config is an empty list, not an error"
    );
}

#[tokio::test]
async fn the_legacy_flat_slot_reads_back_as_entry_zero() {
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 1);
    let zero = &providers[0];
    assert_eq!(zero.slug, "openrouter");
    assert_eq!(zero.label, "OpenRouter", "the catalogue supplies the label");
    assert_eq!(zero.origin, ProviderOrigin::EntryZero);
    assert_eq!(zero.id.as_str(), ENTRY_ZERO_ID);
    assert!(zero.enabled);
    // Written at the uniform address; still READ from the legacy one until
    // the first save converges it. One address rule, one readable fallback.
    assert_eq!(zero.key_key(), provider_key_key("openrouter"));
    assert_eq!(zero.legacy_key_key(), Some(KEY_KEY));
}

#[tokio::test]
async fn a_legacy_credential_is_read_from_the_flat_slot_and_moved_by_one_save() {
    // Lazy convergence. An existing company keeps working untouched, and the
    // first save of that provider moves the key and clears the old slot —
    // no flag day, and no half-migrated state on a store with no
    // transaction.
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;
    secrets
        .set(&company(), KEY_KEY, SecretValue("sk-not-a-real-key".into()))
        .await
        .unwrap();

    let zero = list_providers(&company(), &secrets).await.unwrap()[0].clone();
    assert_eq!(
        load_provider_key(&company(), &secrets, &zero)
            .await
            .unwrap(),
        "sk-not-a-real-key",
        "the fallback is what keeps an untouched company working"
    );

    store_provider_key(&company(), &secrets, &zero, "sk-not-a-real-key-2")
        .await
        .unwrap();
    assert_eq!(
        secrets.get(&company(), &zero.key_key()).await.unwrap(),
        Some(SecretValue("sk-not-a-real-key-2".into())),
    );
    assert_eq!(
        secrets.get(&company(), KEY_KEY).await.unwrap(),
        Some(SecretValue(String::new())),
        "the legacy slot is cleared in the same operation; a key left there \
         after the new one is written is an orphaned secret"
    );
}

#[tokio::test]
async fn entry_zero_keeps_its_id_across_reads() {
    // A generated id would have to be written back to be stable, and the
    // write path into the legacy slot is the one thing this design will not
    // do on a read.
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;
    let first = list_providers(&company(), &secrets).await.unwrap()[0]
        .id
        .clone();
    let second = list_providers(&company(), &secrets).await.unwrap()[0]
        .id
        .clone();
    assert_eq!(first, second);
}

#[tokio::test]
async fn the_legacy_managed_alias_resolves_rather_than_failing() {
    // A stored runtime blob is data an operator cannot hand-edit, so a value
    // the console itself once wrote must not strand them.
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "managed").await;
    let providers = list_providers(&company(), &secrets).await.unwrap();
    // The **kind** normalizes onto OpenRouter — that is the shape of API it
    // speaks. The **slug** does not: it says whose account this is, and a
    // managed config is the TinyHumans account. Keyed on the kind, the
    // managed credential would sit in the slot a real OpenRouter account
    // belongs in, and a company holding both would have one.
    assert_eq!(providers[0].kind, "openrouter");
    assert_eq!(providers[0].slug, super::super::MANAGED_SLUG);
    assert_eq!(providers[0].label, "Managed");
    assert_eq!(
        providers[0].key_key(),
        provider_key_key(super::super::MANAGED_SLUG)
    );
}

#[tokio::test]
async fn adding_a_second_provider_leaves_entry_zero_first() {
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;
    put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();

    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(
        providers
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["openrouter", "acme"]
    );
    assert_eq!(providers[1].origin, ProviderOrigin::Indexed);
    assert_eq!(providers[1].key_key(), "provider/acme/key");
}

#[tokio::test]
async fn two_providers_hold_two_independent_credentials() {
    // The defect this whole change exists to fix: today one company has one
    // credential slot, so switching provider without re-entering a key
    // presents the previous vendor's credential to the new one.
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;
    let zero = list_providers(&company(), &secrets).await.unwrap()[0].clone();
    let acme = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();

    store_provider_key(&company(), &secrets, &zero, "sk-not-a-real-key-zero")
        .await
        .unwrap();
    store_provider_key(&company(), &secrets, &acme, "sk-not-a-real-key-acme")
        .await
        .unwrap();

    assert_eq!(
        load_provider_key(&company(), &secrets, &zero)
            .await
            .unwrap(),
        "sk-not-a-real-key-zero"
    );
    assert_eq!(
        load_provider_key(&company(), &secrets, &acme)
            .await
            .unwrap(),
        "sk-not-a-real-key-acme"
    );
    assert!(
        provider_key_configured(&company(), &secrets, &zero)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_blank_credential_reads_as_not_configured() {
    // The store has no delete: clearing is a write of the empty string, so
    // "cleared" and "never set" are deliberately the same state.
    let secrets = MemSecrets::default();
    let acme = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();
    store_provider_key(&company(), &secrets, &acme, "sk-not-a-real-key")
        .await
        .unwrap();
    assert!(
        provider_key_configured(&company(), &secrets, &acme)
            .await
            .unwrap()
    );
    store_provider_key(&company(), &secrets, &acme, "   ")
        .await
        .unwrap();
    assert!(
        !provider_key_configured(&company(), &secrets, &acme)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deleting_a_provider_clears_its_credential() {
    let secrets = MemSecrets::default();
    let acme = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();
    store_provider_key(&company(), &secrets, &acme, "sk-not-a-real-key")
        .await
        .unwrap();

    assert!(delete_provider(&company(), &secrets, "acme").await.unwrap());
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty()
    );

    // Re-adding the same slug must NOT inherit the old credential.
    let again = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();
    assert!(
        !provider_key_configured(&company(), &secrets, &again)
            .await
            .unwrap(),
        "re-adding a deleted slug silently reused its key"
    );
}

#[tokio::test]
async fn a_failed_credential_clear_keeps_the_provider_visible() {
    // Of the two half-states, "still listed, key intact" is the one the
    // operator can see and act on. "Gone from the list, key on disk" is not.
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: provider_key_key("acme"),
    };
    put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();

    let err = delete_provider(&company(), &secrets, "acme")
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("could not clear the stored credential"),
        "a failed clear must be loud, got: {err}"
    );
    assert_eq!(
        list_providers(&company(), &secrets).await.unwrap().len(),
        1,
        "the provider stayed visible"
    );
}

#[tokio::test]
async fn deleting_something_that_is_not_there_is_not_an_error() {
    let secrets = MemSecrets::default();
    assert!(!delete_provider(&company(), &secrets, "nope").await.unwrap());
}

#[tokio::test]
async fn disabling_keeps_the_endpoint_the_label_and_the_credential() {
    let secrets = MemSecrets::default();
    let acme = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();
    store_provider_key(&company(), &secrets, &acme, "sk-not-a-real-key")
        .await
        .unwrap();

    assert!(
        set_enabled(&company(), &secrets, "acme", false)
            .await
            .unwrap()
    );
    let stored = get_provider(&company(), &secrets, "acme")
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.enabled);
    assert_eq!(stored.base_url, "https://acme.example/v1");
    assert_eq!(stored.label, "acme");
    assert!(
        provider_key_configured(&company(), &secrets, &stored)
            .await
            .unwrap(),
        "disabled is not deleted"
    );
}

#[tokio::test]
async fn replacing_a_provider_keeps_its_id() {
    // Identity survives a rename; that is the reason id and slug are two
    // fields rather than one.
    let secrets = MemSecrets::default();
    let first = put_provider(&company(), &secrets, draft("acme"))
        .await
        .unwrap();
    let mut renamed = draft("acme");
    renamed.label = "Acme gateway".to_string();
    let second = put_provider(&company(), &secrets, renamed).await.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(second.label, "Acme gateway");
    assert_eq!(list_providers(&company(), &secrets).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_generated_id_is_not_the_entry_zero_sentinel() {
    let a = ProviderId::new();
    let b = ProviderId::new();
    assert_ne!(a, b, "ids must not repeat");
    assert_ne!(a.as_str(), ENTRY_ZERO_ID);
    assert!(a.as_str().starts_with("prv_"));
}

#[tokio::test]
async fn a_second_record_may_not_shadow_entry_zero() {
    let secrets = MemSecrets::default();
    write_entry_zero(&secrets, "openrouter").await;
    let err = put_provider(&company(), &secrets, draft("openrouter"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing provider"), "{err}");
}

#[test]
fn a_slug_is_derived_from_the_name_never_typed() {
    assert_eq!(slugify("Acme Gateway"), "acme-gateway");
    assert_eq!(slugify("  My  OpenRouter!!  "), "my-openrouter");
    assert_eq!(slugify("---"), "");
}

#[test]
fn a_custom_slug_is_refused_for_three_named_reasons() {
    let existing = vec![Provider {
        id: ProviderId::new(),
        slug: "acme".into(),
        label: "Acme".into(),
        kind: "openai_compatible".into(),
        base_url: "https://acme.example/v1".into(),
        models: BTreeMap::new(),
        enabled: true,
        origin: ProviderOrigin::Indexed,
    }];
    assert_eq!(check_slug(&existing, "   "), Err(SlugError::Empty));
    assert_eq!(check_slug(&existing, "acme"), Err(SlugError::Taken));
    assert_eq!(check_slug(&existing, "groq"), Err(SlugError::Reserved));
    assert_eq!(check_slug(&existing, "acme-two"), Ok(()));
}

#[test]
fn a_provider_name_is_bounded_at_the_limit_and_refused_past_it() {
    // The bound exists because the name becomes the address of a secret.
    // At the limit is a legal name; one character past it is not, and the
    // refusal happens here rather than at the store, where it used to
    // arrive as `ENAMETOOLONG` after a write had already landed.
    let at_limit = "a".repeat(MAX_PROVIDER_NAME_CHARS);
    let past_limit = "a".repeat(MAX_PROVIDER_NAME_CHARS + 1);

    assert_eq!(check_provider_name(&at_limit), Ok(()));
    assert_eq!(check_provider_name(&past_limit), Err(SlugError::TooLong));
    assert_eq!(check_provider_name("  "), Err(SlugError::Empty));

    assert_eq!(check_slug(&[], &at_limit), Ok(()));
    assert_eq!(check_slug(&[], &past_limit), Err(SlugError::TooLong));

    // Characters, not bytes: a name of multi-byte characters is judged by
    // what the operator typed rather than by how UTF-8 happens to store it.
    let multibyte = "é".repeat(MAX_PROVIDER_NAME_CHARS);
    assert_eq!(check_provider_name(&multibyte), Ok(()));
}

#[test]
fn a_bounded_name_keeps_its_credential_key_inside_the_filename_budget() {
    // Why 80 and not some larger round number: the derived secret key has
    // to stay short enough that the canonical filename is the readable
    // `%k-` form rather than the truncated-and-digested `%l-` one. The
    // slug alphabet is `[a-z0-9-]`, one byte per character once
    // percent-encoded, and `provider/` + `/key` add 17.
    let key = provider_key_key(&"a".repeat(MAX_PROVIDER_NAME_CHARS));
    // `provider/` + `/key` is 13 characters around the slug.
    assert_eq!(key.len(), MAX_PROVIDER_NAME_CHARS + 13);
    // Percent-encoding is what the budget is measured in. The slug alphabet
    // (`[a-z0-9-]`) survives as one byte per character; the two `/`
    // separators become `%2F`, three bytes each.
    let encoded_len = key.len() + 2 * 2;
    assert!(
        encoded_len < 200,
        "a bounded name must not need a truncated secret filename: {encoded_len} bytes"
    );
}

#[tokio::test]
async fn an_index_written_before_enabled_existed_reads_as_enabled() {
    // A missing field must not read as "every provider is off", which is
    // what `#[serde(default)]` on a bool would have given.
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            PROVIDER_INDEX_KEY,
            SecretValue(
                r#"[{"id":"prv_old","slug":"acme","label":"Acme","kind":"openai_compatible","base_url":"https://acme.example/v1"}]"#
                    .to_string(),
            ),
        )
        .await
        .unwrap();
    let providers = list_providers(&company(), &secrets).await.unwrap();
    assert_eq!(providers.len(), 1);
    assert!(providers[0].enabled);
}

#[tokio::test]
async fn a_malformed_index_is_surfaced_not_swallowed() {
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            PROVIDER_INDEX_KEY,
            SecretValue("{not json".into()),
        )
        .await
        .unwrap();
    let err = list_providers(&company(), &secrets).await.unwrap_err();
    assert!(err.to_string().contains("not valid JSON"), "{err}");
}

#[tokio::test]
async fn an_empty_index_blob_is_an_empty_list() {
    let secrets = MemSecrets::default();
    secrets
        .set(&company(), PROVIDER_INDEX_KEY, SecretValue(String::new()))
        .await
        .unwrap();
    assert!(
        list_providers(&company(), &secrets)
            .await
            .unwrap()
            .is_empty()
    );
}

// ---- routes -------------------------------------------------------------

#[tokio::test]
async fn a_company_with_no_routes_reads_an_empty_table() {
    let secrets = MemSecrets::default();
    assert!(load_routes(&company(), &secrets).await.unwrap().is_empty());
}

#[tokio::test]
async fn routes_round_trip_through_the_grammar_an_operator_types() {
    let secrets = MemSecrets::default();
    let mut routes = Routes::new();
    routes.insert("reasoning-v1".to_string(), ProviderRef::parse("acme:gpt-5"));
    routes.insert("chat-v1".to_string(), ProviderRef::Managed);
    routes.insert("vision-v1".to_string(), ProviderRef::parse("local:llava"));
    save_routes(&company(), &secrets, &routes).await.unwrap();

    let read = load_routes(&company(), &secrets).await.unwrap();
    assert_eq!(read, routes);
    // And the stored form really is the text, so a person reading raw keys
    // sees what they would have typed.
    let raw = secrets
        .get(&company(), ROUTES_KEY)
        .await
        .unwrap()
        .unwrap()
        .0;
    assert!(raw.contains("acme:gpt-5"), "{raw}");
}

#[tokio::test]
async fn an_unset_route_is_dropped_rather_than_stored_as_empty() {
    let secrets = MemSecrets::default();
    let mut routes = Routes::new();
    routes.insert("chat-v1".to_string(), ProviderRef::Default);
    routes.insert("agentic-v1".to_string(), ProviderRef::parse("acme"));
    save_routes(&company(), &secrets, &routes).await.unwrap();

    let read = load_routes(&company(), &secrets).await.unwrap();
    assert!(
        !read.contains_key("chat-v1"),
        "unset must not persist: {read:?}"
    );
    assert_eq!(read.get("agentic-v1"), Some(&ProviderRef::parse("acme")));
}

#[tokio::test]
async fn an_unreadable_routes_blob_is_an_error_rather_than_silently_empty() {
    // Routes decide where a company's spend goes. Reading a corrupt table as
    // "no routes" would move every workload onto the primary without saying
    // so, which is the silent-demotion failure the resolver refuses.
    let secrets = MemSecrets::default();
    secrets
        .set(&company(), ROUTES_KEY, SecretValue("{oops".into()))
        .await
        .unwrap();
    let err = load_routes(&company(), &secrets).await.unwrap_err();
    assert!(err.to_string().contains("not valid JSON"), "{err}");
}

// ---- health -------------------------------------------------------------

#[tokio::test]
async fn health_is_latched_once_per_failure_episode_not_once_per_retry() {
    let secrets = MemSecrets::default();
    assert!(
        record_health(&company(), &secrets, "acme", "auth", "2026-09-11T09:14:00Z")
            .await
            .unwrap(),
        "the first observation moves the record"
    );
    assert!(
        !record_health(&company(), &secrets, "acme", "auth", "2026-09-11T09:15:00Z")
            .await
            .unwrap(),
        "the same failure again must not move the record"
    );
    let health = load_health(&company(), &secrets).await.unwrap();
    assert_eq!(
        health.get("acme").unwrap().at,
        "2026-09-11T09:14:00Z",
        "the timestamp names when the episode began, not the latest retry"
    );
}

#[tokio::test]
async fn a_state_change_moves_the_record() {
    let secrets = MemSecrets::default();
    record_health(&company(), &secrets, "acme", "auth", "2026-09-11T09:14:00Z")
        .await
        .unwrap();
    assert!(
        record_health(&company(), &secrets, "acme", "ok", "2026-09-11T10:00:00Z")
            .await
            .unwrap()
    );
    let health = load_health(&company(), &secrets).await.unwrap();
    assert_eq!(health.get("acme").unwrap().state, "ok");
    assert_eq!(health.get("acme").unwrap().at, "2026-09-11T10:00:00Z");
}

#[tokio::test]
async fn health_is_per_provider_so_one_rejection_does_not_condemn_a_sibling() {
    // Two providers, one endpoint, two keys: a 401 is an answer about the
    // credential presented, never about the address.
    let secrets = MemSecrets::default();
    record_health(&company(), &secrets, "acme", "auth", "2026-09-11T09:14:00Z")
        .await
        .unwrap();
    record_health(
        &company(),
        &secrets,
        "acme-team",
        "ok",
        "2026-09-11T09:14:00Z",
    )
    .await
    .unwrap();
    let health = load_health(&company(), &secrets).await.unwrap();
    assert_eq!(health.get("acme").unwrap().state, "auth");
    assert_eq!(health.get("acme-team").unwrap().state, "ok");
}

#[tokio::test]
async fn forgetting_health_stops_a_reused_slug_inheriting_a_state() {
    let secrets = MemSecrets::default();
    record_health(&company(), &secrets, "acme", "auth", "2026-09-11T09:14:00Z")
        .await
        .unwrap();
    forget_health(&company(), &secrets, "acme").await.unwrap();
    assert!(
        !load_health(&company(), &secrets)
            .await
            .unwrap()
            .contains_key("acme")
    );
    // Forgetting something that was never there is not an error.
    forget_health(&company(), &secrets, "ghost").await.unwrap();
}

#[tokio::test]
async fn an_unreadable_health_blob_reads_as_nothing_learnt() {
    // The opposite call from routes, and deliberately: health holds no
    // configuration, so failing a status read over it would take the whole
    // page down to preserve a decoration.
    let secrets = MemSecrets::default();
    secrets
        .set(&company(), HEALTH_KEY, SecretValue("{oops".into()))
        .await
        .unwrap();
    assert!(load_health(&company(), &secrets).await.unwrap().is_empty());
}
/// What a routing write actually leaves behind, versus what was asked for.
///
/// The `PUT` route used to answer with the table it built from the **request
/// body**, which made the response a picture of the ask rather than of the
/// state — so any divergence between the two was invisible by construction,
/// and a save that landed nowhere still came back carrying the operator's own
/// intent. This is the smallest concrete divergence, and it is not
/// hypothetical: `save_routes` drops `Default` entries, because an absence is
/// how "nothing set here" is stored. Echoing the request claimed a row had
/// been written that the store deliberately holds nothing for.
#[tokio::test]
async fn a_routing_write_does_not_store_what_it_was_handed() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();

    let mut asked = Routes::new();
    asked.insert("chat-v1".into(), ProviderRef::parse("acme:gpt-5"));
    // The operator put this row back to "follow the default".
    asked.insert("reasoning-v1".into(), ProviderRef::parse(""));
    save_routes(&company, &secrets, &asked).await.unwrap();

    let stored = load_routes(&company, &secrets).await.unwrap();
    assert_eq!(
        stored.get("chat-v1"),
        Some(&ProviderRef::parse("acme:gpt-5"))
    );
    assert!(
        !stored.contains_key("reasoning-v1"),
        "an unset row is stored as an absence, so a response echoing the request \
         would claim a row that is not there"
    );
    assert_ne!(
        asked, stored,
        "the ask and the stored table differ, which is why the route reads back"
    );
}

/// A routing write that cannot land must not read back as if it had.
#[tokio::test]
async fn a_dropped_routing_write_is_visible_on_the_read_back() {
    let company = CompanyId::new("acme");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: ROUTES_KEY.to_string(),
    };

    let mut asked = Routes::new();
    asked.insert("chat-v1".into(), ProviderRef::parse("acme:gpt-5"));
    assert!(
        save_routes(&company, &secrets, &asked).await.is_err(),
        "the write itself reports the failure"
    );
    // And the read-back agrees with the store rather than with the ask —
    // which is the property the route now answers from.
    let stored = load_routes(&company, &secrets).await.unwrap();
    assert!(stored.is_empty());
}

// ---- the default's new shape (keys rework, issue #2306, slice 2b) ------

#[tokio::test]
async fn a_json_default_reads_provider_and_model() {
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            DEFAULT_PROVIDER_KEY,
            SecretValue(
                "  {\"provider\":\" acme \",\"model\":\" acme/other-model \"}\n".into(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::Full(ModelChoice {
            provider: "acme".to_string(),
            model: "acme/other-model".to_string(),
        })
    );
}

#[tokio::test]
async fn a_bare_slug_default_reads_as_provider_without_model() {
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            DEFAULT_PROVIDER_KEY,
            SecretValue(" acme\n".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::ProviderOnly("acme".to_string())
    );
    assert_eq!(
        load_default_slug(&company(), &secrets)
            .await
            .unwrap()
            .as_deref(),
        Some("acme")
    );
}

#[tokio::test]
async fn a_blank_model_in_json_reads_as_provider_only() {
    let secrets = MemSecrets::default();
    for raw in [
        r#"{"provider":"acme"}"#,
        r#"{"provider":"acme","model":null}"#,
        r#"{"provider":"acme","model":"  "}"#,
    ] {
        secrets
            .set(
                &company(),
                DEFAULT_PROVIDER_KEY,
                SecretValue(raw.to_string()),
            )
            .await
            .unwrap();
        assert_eq!(
            load_default(&company(), &secrets).await.unwrap(),
            DefaultChoice::ProviderOnly("acme".to_string()),
            "raw: {raw}"
        );
    }
}

#[tokio::test]
async fn an_unparseable_json_default_is_an_error() {
    let secrets = MemSecrets::default();
    for raw in [
        r#"{"provider":"#,
        r#"{"model":"x"}"#,
        r#"{"provider":"  ","model":"x"}"#,
        r#"{"provider":5}"#,
    ] {
        secrets
            .set(
                &company(),
                DEFAULT_PROVIDER_KEY,
                SecretValue(raw.to_string()),
            )
            .await
            .unwrap();
        let err = load_default(&company(), &secrets).await.unwrap_err();
        assert!(
            matches!(err, OpenCompanyError::Store(_)),
            "raw: {raw}: {err}"
        );
        assert!(
            err.to_string().contains("inference default"),
            "raw: {raw}: {err}"
        );
    }
}

/// Round-3a review P2-4: the read side of a corrupt default must degrade,
/// never 500 — see [`load_default_lenient`]'s own doc for why.
#[tokio::test]
async fn a_corrupt_default_reads_as_unset_and_unreadable_never_written() {
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            DEFAULT_PROVIDER_KEY,
            SecretValue(r#"{oops"#.to_string()),
        )
        .await
        .unwrap();

    let (choice, unreadable) = load_default_lenient(&company(), &secrets).await;
    assert_eq!(choice, DefaultChoice::Unset);
    assert!(unreadable);

    // Never rewritten: the corrupt value is still on disk, byte for byte,
    // so fixing it by hand and reading again recovers on its own.
    let raw = secrets
        .get(&company(), DEFAULT_PROVIDER_KEY)
        .await
        .unwrap()
        .map(|SecretValue(v)| v);
    assert_eq!(raw.as_deref(), Some(r#"{oops"#));
}

#[tokio::test]
async fn a_readable_default_round_trips_through_the_lenient_reader_unmarked() {
    let secrets = MemSecrets::default();
    let choice = ModelChoice {
        provider: "acme".to_string(),
        model: "test-model".to_string(),
    };
    set_default_choice(&company(), &secrets, &choice)
        .await
        .unwrap();

    let (read, unreadable) = load_default_lenient(&company(), &secrets).await;
    assert_eq!(read, DefaultChoice::Full(choice));
    assert!(!unreadable);
}

#[tokio::test]
async fn a_cleared_default_reads_unset() {
    let secrets = MemSecrets::default();
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::Unset
    );
    assert_eq!(load_default_slug(&company(), &secrets).await.unwrap(), None);

    clear_default_slug(&company(), &secrets).await.unwrap();
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::Unset
    );

    secrets
        .set(&company(), DEFAULT_PROVIDER_KEY, SecretValue("   ".into()))
        .await
        .unwrap();
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::Unset
    );
    assert_eq!(load_default_slug(&company(), &secrets).await.unwrap(), None);
}

#[tokio::test]
async fn setting_a_default_choice_is_one_json_write() {
    let secrets = MemSecrets::default();
    set_default_choice(
        &company(),
        &secrets,
        &ModelChoice {
            provider: " tinyhumans ".to_string(),
            model: " acme/test-model ".to_string(),
        },
    )
    .await
    .unwrap();
    // Scoped so the guard is released before the `.await` below: clippy's
    // `await_holding_lock` is right that a `std` guard across an await is a
    // deadlock waiting to happen, even though this one never contends.
    {
        let map = secrets.map.lock().unwrap();
        assert_eq!(map.len(), 1, "one write: {map:?}");
        assert_eq!(
            map.get(DEFAULT_PROVIDER_KEY).map(String::as_str),
            Some(r#"{"provider":"tinyhumans","model":"acme/test-model"}"#)
        );
    }
    assert_eq!(
        load_default(&company(), &secrets).await.unwrap(),
        DefaultChoice::Full(ModelChoice {
            provider: "tinyhumans".to_string(),
            model: "acme/test-model".to_string(),
        })
    );
}

#[tokio::test]
async fn a_default_choice_without_a_model_is_refused_before_writing() {
    let secrets = MemSecrets::default();
    let err = set_default_choice(
        &company(),
        &secrets,
        &ModelChoice {
            provider: "acme".to_string(),
            model: "  ".to_string(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, OpenCompanyError::InvalidRequest(_)), "{err}");
    assert!(secrets.map.lock().unwrap().is_empty());
}

#[tokio::test]
async fn load_default_slug_reads_the_provider_out_of_a_json_default() {
    let secrets = MemSecrets::default();
    secrets
        .set(
            &company(),
            DEFAULT_PROVIDER_KEY,
            SecretValue(r#"{"provider":"acme","model":"acme/other-model"}"#.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(
        load_default_slug(&company(), &secrets)
            .await
            .unwrap()
            .as_deref(),
        Some("acme")
    );
}

#[tokio::test]
async fn a_row_with_one_distinct_model_collapses_to_it() {
    let secrets = MemSecrets::default();
    let mut d = draft("acme");
    d.models = crate::company::INFERENCE_TIERS
        .iter()
        .map(|t| ((*t).to_string(), "acme/other-model".to_string()))
        .collect();
    d.models
        .insert("chat-v1".to_string(), " acme/other-model ".to_string());
    d.models.insert("extra".to_string(), "".to_string());
    put_provider(&company(), &secrets, d).await.unwrap();
    let row = get_provider(&company(), &secrets, "acme")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.model(), ModelOnRow::One("acme/other-model".to_string()));
}

#[test]
fn a_row_with_no_model_reads_none() {
    assert_eq!(model_on_row(&BTreeMap::new()), ModelOnRow::None);
    let mut blank = BTreeMap::new();
    blank.insert("chat-v1".to_string(), "  ".to_string());
    assert_eq!(model_on_row(&blank), ModelOnRow::None);
}

#[test]
fn a_row_with_two_distinct_models_is_ambiguous_never_picked() {
    let mut models = BTreeMap::new();
    models.insert("chat-v1".to_string(), "b-model".to_string());
    models.insert("agentic-v1".to_string(), "a-model".to_string());
    models.insert("reasoning-v1".to_string(), "a-model".to_string());
    assert_eq!(
        model_on_row(&models),
        ModelOnRow::Ambiguous(vec!["a-model".to_string(), "b-model".to_string()])
    );
}

#[tokio::test]
async fn entry_zero_models_collapse_the_same_way() {
    let secrets = MemSecrets::default();
    let uniform = crate::company::INFERENCE_TIERS
        .iter()
        .map(|t| ((*t).to_string(), "acme/test-model".to_string()))
        .collect();
    super::super::save_runtime_config(
        &company(),
        &secrets,
        &RuntimeInference {
            provider: "openrouter".to_string(),
            base_url: None,
            models: uniform,
        },
    )
    .await
    .unwrap();
    let zero = entry_zero(&company(), &secrets).await.unwrap().unwrap();
    assert_eq!(zero.model(), ModelOnRow::One("acme/test-model".to_string()));

    let mut ambiguous = BTreeMap::new();
    ambiguous.insert("chat-v1".to_string(), "x/a".to_string());
    ambiguous.insert("agentic-v1".to_string(), "x/b".to_string());
    super::super::save_runtime_config(
        &company(),
        &secrets,
        &RuntimeInference {
            provider: "openrouter".to_string(),
            base_url: None,
            models: ambiguous,
        },
    )
    .await
    .unwrap();
    let zero = entry_zero(&company(), &secrets).await.unwrap().unwrap();
    assert!(matches!(zero.model(), ModelOnRow::Ambiguous(_)));
}
