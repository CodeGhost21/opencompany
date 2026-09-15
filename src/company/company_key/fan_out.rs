//! The account-key fan-out (keys rework, issue #2306, slice 4a): one
//! `PUT …/credential` copies [`super::KEY_KEY`] into the Composio and LLM
//! TinyHumans slots (Q7), adds the `tinyhumans` provider row when a model is
//! known, sets the company default only when none is set, and checks health —
//! all under a per-company lock so two concurrent saves cannot leave one slot
//! rotated and another still on the old value.
//!
//! See `docs/key-reworks/phase-4a-account-key-fanout.md` for the full design
//! and the data carry-over matrix (§6) every test here proves a row of.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;

use crate::Result;
use crate::company::composio;
use crate::company::inference::{self, catalogue, probe, store as inference_store};
use crate::error::OpenCompanyError;
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

use super::KEY_KEY;
use super::types::{FanOutReport, FanOutRequest, SkipReason, Slot, SlotOutcome, SlotReport};

// ---------------------------------------------------------------------------
// The per-company lock
// ---------------------------------------------------------------------------

/// One lock per company that has ever fanned out an account-key save.
///
/// A copy of the pattern `search::store`'s `INDEX_LOCKS` already uses: it
/// serialises mutations **within one process**, which is the whole of a
/// deployment (one container per tenant), and is not a distributed lock.
static SLOT_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Mutex::default);

/// Takes this company's slot lock, held until the returned guard is dropped.
///
/// The inner `std` mutex is held only long enough to clone an `Arc` — never
/// across an await — so a panicking writer cannot poison anything a later
/// request needs.
pub async fn slot_guard(company: &CompanyId) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = SLOT_LOCKS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(company.as_ref().to_string())
            .or_default()
            .clone()
    };
    lock.lock_owned().await
}

// ---------------------------------------------------------------------------
// decide_copy — the whole Q7 rule, pure
// ---------------------------------------------------------------------------

/// What [`decide_copy`] says to do to one derived slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyDecision {
    /// Write `new` into the slot (a fill or a rotation — the caller tells
    /// those apart by whether `current` was empty).
    Write,
    /// Clear the slot (it held the old account key).
    Clear,
    /// Leave the slot as it is, and it is worth saying it agreed already.
    Keep(SkipReason),
    /// Leave the slot as it is, and say why nothing happened.
    Skip(SkipReason),
}

/// The Q7 rule (`docs/key-reworks/phase-4a-account-key-fanout.md` §3.2's
/// table), decided from three already-trimmed values: `current` (what the
/// slot holds now), `old_account` (what `tinyhumans/key` held before this
/// request), `new` (what this request wants `tinyhumans/key` to hold — empty
/// means a clear).
///
/// A derived slot is written when it is empty or equal to the old account
/// key; any other value was set on that slot's own page and is left alone.
pub fn decide_copy(current: &str, old_account: &str, new: &str) -> CopyDecision {
    let old_account_is_current = !old_account.is_empty() && current == old_account;
    if new.is_empty() {
        if current.is_empty() {
            CopyDecision::Skip(SkipReason::AlreadyEmpty)
        } else if old_account_is_current {
            CopyDecision::Clear
        } else {
            CopyDecision::Keep(SkipReason::CustomKey)
        }
    } else if current == new {
        CopyDecision::Keep(SkipReason::AlreadyCurrent)
    } else if current.is_empty() || old_account_is_current {
        CopyDecision::Write
    } else {
        CopyDecision::Keep(SkipReason::CustomKey)
    }
}

// ---------------------------------------------------------------------------
// The inference probe seam
// ---------------------------------------------------------------------------

/// What [`fan_out`] asks to learn whether the LLM copy actually works.
///
/// A trait rather than a bare function so a test can answer without a
/// network — see `FakeProber` in this module's tests.
#[async_trait]
pub trait InferenceProber: Send + Sync {
    async fn probe(
        &self,
        base_url: &str,
        key: &str,
    ) -> std::result::Result<Vec<String>, probe::ProbeFailure>;
}

/// The real probe: the same read [`inference::store`]'s 2a provider-add path
/// uses for a `tinyhumans` row — one GET, paged, against the TinyHumans
/// OpenRouter proxy.
pub struct LiveProber;

#[async_trait]
impl InferenceProber for LiveProber {
    async fn probe(
        &self,
        base_url: &str,
        key: &str,
    ) -> std::result::Result<Vec<String>, probe::ProbeFailure> {
        probe::probe_models(
            base_url,
            Some(key),
            catalogue::auth_style_for(inference::MANAGED_SLUG),
            probe::default_policy(),
            catalogue::catalog_shape_for(inference::MANAGED_SLUG, base_url),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Small local helpers
// ---------------------------------------------------------------------------

/// One model id, pinned to every tier — the same shape
/// `ops::inference::providers::tier_overrides` builds when the console adds a
/// row. Not shared: that function is private to its module, and reaching
/// across the ops/company boundary for a four-line map is a worse coupling
/// than the duplication.
fn tier_overrides(model: &str) -> BTreeMap<String, String> {
    crate::company::INFERENCE_TIERS
        .iter()
        .map(|tier| ((*tier).to_string(), model.to_string()))
        .collect()
}

/// `now`, RFC 3339 — the same dependency-free formatter
/// `ops::inference::providers`' own `record_health` wrapper uses.
fn now_rfc3339() -> String {
    crate::server::graphql::iso8601(crate::ports::now_millis())
}

/// Writes the Composio TinyHumans slot for the fan-out specifically: the new
/// address gets `value`, and the pre-rename legacy address
/// (`composio::LEGACY_TOKEN_KEY`) is always zeroed rather than mirrored.
///
/// This is deliberately **not** [`composio::store_token`], which mirrors the
/// *same* value to both addresses (D-mirror, for an ordinary console-driven
/// token set). The fan-out's copy is not that: once this slot is under the
/// fan-out's management the legacy address stops being a live fallback for
/// it, which is what lets a company whose Composio credential was only ever
/// read through the legacy address (M14) end up with a clean new-address-only
/// state after one save, rather than a legacy address permanently mirroring
/// whatever this path last wrote.
async fn write_composio_slot(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    value: &str,
) -> Result<()> {
    secrets
        .set(
            company,
            composio::TINYHUMANS_KEY_KEY,
            SecretValue(value.to_string()),
        )
        .await?;
    secrets
        .set(
            company,
            composio::LEGACY_TOKEN_KEY,
            SecretValue(String::new()),
        )
        .await?;
    Ok(())
}

/// The [`SkipReason`] to report on the default slot when the provider slot
/// did not fill and is not `Kept(RowExists)` (handled by its own branch in
/// [`fan_out`]) — i.e. every other [`SlotOutcome`] the provider slot can end
/// up in.
///
/// [`SlotOutcome::Failed`] has no matching [`SkipReason`] of its own — §3.1's
/// list is fixed and none of its ten reasons names "the row write itself
/// failed" — so it falls back to [`SkipReason::InferenceNotWritten`]: the LLM
/// key is fine, but nothing new exists for a default to point at, which is
/// the same practical consequence.
fn provider_gate_reason(outcome: &SlotOutcome) -> SkipReason {
    match outcome {
        SlotOutcome::Kept(reason) | SlotOutcome::Skipped(reason) => *reason,
        _ => SkipReason::InferenceNotWritten,
    }
}

// ---------------------------------------------------------------------------
// slot_facts — read-only, for the account-key dialog (keys rework #2306, 4b)
// ---------------------------------------------------------------------------

/// What the account-key dialog needs to know **before** it saves, to say which
/// slots a save would actually touch (Q9).
///
/// `docs/key-reworks/phase-4b-account-dialog.md` §3.1: `*_has_own_key` is
/// deliberately not "is set" — a copy still equal to the account key is filled
/// again on rotation, so saving does fill it. Only whether the two stored
/// values are equal is revealed here, never either value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SlotFacts {
    /// The LLM TinyHumans key slot holds a key that is not the account key.
    /// Saving leaves it alone (Q7). Never the key.
    pub inference_has_own_key: bool,
    /// The same for `composio/tinyhumans/key` (with 1a's legacy read).
    pub composio_has_own_key: bool,
    /// `inference/default` is set (`ProviderOnly` or `Full`).
    pub default_set: bool,
}

/// Read-only. The same reads [`fan_out`]'s step 2 makes, so the dialog can
/// never disagree with what a save would actually do. No lock — a read races
/// nothing it could corrupt.
pub async fn slot_facts(company: &CompanyId, secrets: &dyn SecretStore) -> Result<SlotFacts> {
    let account_key = secrets
        .get(company, KEY_KEY)
        .await?
        .map(|SecretValue(v)| v.trim().to_string())
        .unwrap_or_default();
    let composio_now = composio::load_tinyhumans_key(company, secrets)
        .await?
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let inference_now =
        inference::load_managed_key(company, secrets, &inference::HarnessScope::default())
            .await?
            .trim()
            .to_string();
    let default_now = inference_store::load_default(company, secrets).await?;

    let has_own_key = |current: &str| !current.is_empty() && current != account_key;
    Ok(SlotFacts {
        inference_has_own_key: has_own_key(&inference_now),
        composio_has_own_key: has_own_key(&composio_now),
        default_set: !matches!(default_now, inference_store::DefaultChoice::Unset),
    })
}

// ---------------------------------------------------------------------------
// fan_out
// ---------------------------------------------------------------------------

/// Runs the whole account-key fan-out for one `PUT …/credential` (or the
/// key-grant's `finish_link`), under [`slot_guard`] for the whole call.
///
/// `Err` only when the pre-write reads or the account-key write itself fail —
/// nothing else has been written yet at that point. Every later failure is
/// reported as a `Failed` slot in the returned [`FanOutReport`] instead, so a
/// transient fault in, say, the row write never rolls back the key copies
/// that already succeeded.
pub async fn fan_out(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    request: FanOutRequest<'_>,
    prober: &dyn InferenceProber,
) -> Result<FanOutReport> {
    let _guard = slot_guard(company).await;

    // 1. Validate. Nothing is written yet.
    let new = request.key.trim().to_string();
    let clearing = new.is_empty();
    let model: Option<String> = match request.model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(_) if clearing => {
            return Err(OpenCompanyError::InvalidRequest(
                "A model cannot be chosen while removing the key.".to_string(),
            ));
        }
        Some(raw) => Some(inference_store::check_model_id(raw)?),
        None => None,
    };

    // 2. Read. Any error here returns `Err` — nothing has been written.
    let old_account = secrets
        .get(company, KEY_KEY)
        .await?
        .map(|SecretValue(v)| v.trim().to_string())
        .unwrap_or_default();
    let composio_now = composio::load_tinyhumans_key(company, secrets)
        .await?
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let providers = inference_store::list_providers(company, secrets).await?;
    let legacy_managed = providers.iter().any(|p| {
        p.origin == inference_store::ProviderOrigin::EntryZero && p.slug == inference::MANAGED_SLUG
    });
    let row = providers.into_iter().find(|p| {
        p.origin == inference_store::ProviderOrigin::Indexed && p.slug == inference::MANAGED_SLUG
    });
    let inference_key_key = inference_store::provider_key_key(inference::MANAGED_SLUG);
    let inference_raw_new = secrets
        .get(company, &inference_key_key)
        .await?
        .map(|SecretValue(v)| v);
    let legacy_owned = inference_store::legacy_slot_is_managed(company, secrets).await?;
    let legacy_raw = if legacy_owned {
        secrets
            .get(company, inference::KEY_KEY)
            .await?
            .map(|SecretValue(v)| v)
    } else {
        None
    };
    let inference_now =
        inference::load_managed_key(company, secrets, &inference::HarnessScope::default())
            .await?
            .trim()
            .to_string();
    let default_now = inference_store::load_default(company, secrets).await?;

    // 3. Account key. Error here also returns `Err` — the read above is the
    // last chance to fail with nothing stored.
    secrets
        .set(company, KEY_KEY, SecretValue(new.clone()))
        .await?;

    // 4. Composio. Never reads or writes `composio/mode` or
    // `composio/byok/key` — see `write_composio_slot`.
    let composio_outcome = match decide_copy(&composio_now, &old_account, &new) {
        CopyDecision::Write => match write_composio_slot(company, secrets, &new).await {
            Ok(()) => {
                if composio_now.is_empty() {
                    SlotOutcome::Filled
                } else {
                    SlotOutcome::Rotated
                }
            }
            Err(_) => SlotOutcome::Failed,
        },
        CopyDecision::Clear => match write_composio_slot(company, secrets, "").await {
            Ok(()) => SlotOutcome::Cleared,
            Err(_) => SlotOutcome::Failed,
        },
        CopyDecision::Keep(reason) => SlotOutcome::Kept(reason),
        CopyDecision::Skip(reason) => SlotOutcome::Skipped(reason),
    };

    // 5. Inference key.
    let legacy_raw_is_live = legacy_owned
        && legacy_raw
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
    let mut inference_outcome = match decide_copy(&inference_now, &old_account, &new) {
        decision @ (CopyDecision::Write | CopyDecision::Clear) => {
            let is_clear = matches!(decision, CopyDecision::Clear);
            let write_value = if is_clear { String::new() } else { new.clone() };
            match secrets
                .set(company, &inference_key_key, SecretValue(write_value))
                .await
            {
                Ok(()) => {
                    let primary = if is_clear {
                        SlotOutcome::Cleared
                    } else if inference_now.is_empty() {
                        SlotOutcome::Filled
                    } else {
                        SlotOutcome::Rotated
                    };
                    if legacy_raw_is_live {
                        match secrets
                            .set(company, inference::KEY_KEY, SecretValue(String::new()))
                            .await
                        {
                            Ok(()) => primary,
                            Err(err) if is_clear => {
                                tracing::error!(
                                    company = %company,
                                    error = %err,
                                    "keys rework: could not clear the legacy inference/key slot on an account-key clear",
                                );
                                SlotOutcome::Failed
                            }
                            Err(err) => {
                                tracing::error!(
                                    company = %company,
                                    error = %err,
                                    "keys rework: could not clear the legacy inference/key slot after copying the account key",
                                );
                                primary
                            }
                        }
                    } else {
                        primary
                    }
                }
                Err(_) => SlotOutcome::Failed,
            }
        }
        CopyDecision::Keep(reason) => SlotOutcome::Kept(reason),
        CopyDecision::Skip(reason) => SlotOutcome::Skipped(reason),
    };

    let holds_new = matches!(
        inference_outcome,
        SlotOutcome::Filled | SlotOutcome::Rotated | SlotOutcome::Kept(SkipReason::AlreadyCurrent)
    );

    // 6. Clearing stops here: rows and the default are never touched by a
    // clear (§6, case C1).
    if clearing {
        let health_outcome = if matches!(inference_outcome, SlotOutcome::Cleared) {
            if let Err(err) =
                inference_store::forget_health(company, secrets, inference::MANAGED_SLUG).await
            {
                tracing::warn!(
                    company = %company,
                    error = %err,
                    "keys rework: could not forget tinyhumans health on an account-key clear",
                );
            }
            SlotOutcome::Cleared
        } else {
            SlotOutcome::Skipped(SkipReason::KeyCleared)
        };
        return Ok(FanOutReport {
            slots: vec![
                SlotReport {
                    slot: Slot::Composio,
                    outcome: composio_outcome,
                },
                SlotReport {
                    slot: Slot::Inference,
                    outcome: inference_outcome,
                },
                SlotReport {
                    slot: Slot::Provider,
                    outcome: SlotOutcome::Skipped(SkipReason::KeyCleared),
                },
                SlotReport {
                    slot: Slot::Default,
                    outcome: SlotOutcome::Skipped(SkipReason::KeyCleared),
                },
                SlotReport {
                    slot: Slot::Health,
                    outcome: health_outcome,
                },
            ],
            needs_model: false,
            sets_default: false,
            models: Vec::new(),
        });
    }

    // 7. Health, before any row or default write (Q6 by construction).
    let mut probe_ids: Option<Vec<String>> = None;
    let health_outcome = if legacy_managed {
        SlotOutcome::Skipped(SkipReason::LegacyManagedConfig)
    } else if !holds_new {
        match inference_outcome {
            SlotOutcome::Kept(SkipReason::CustomKey) => SlotOutcome::Skipped(SkipReason::CustomKey),
            _ => SlotOutcome::Skipped(SkipReason::InferenceNotWritten),
        }
    } else {
        let base = row
            .as_ref()
            .map(|r| r.base_url.clone())
            .or_else(|| {
                catalogue::cloud_provider(inference::MANAGED_SLUG).map(|c| c.endpoint.to_string())
            })
            .unwrap_or_default();
        match prober.probe(&base, &new).await {
            Ok(ids) => {
                if let Err(err) = inference_store::record_health(
                    company,
                    secrets,
                    inference::MANAGED_SLUG,
                    "ok",
                    &now_rfc3339(),
                )
                .await
                {
                    tracing::warn!(company = %company, error = %err, "keys rework: could not record tinyhumans health");
                }
                probe_ids = Some(ids);
                SlotOutcome::HealthOk
            }
            Err(failure) => {
                if let Err(err) = inference_store::record_health(
                    company,
                    secrets,
                    inference::MANAGED_SLUG,
                    failure.class.as_str(),
                    &now_rfc3339(),
                )
                .await
                {
                    tracing::warn!(company = %company, error = %err, "keys rework: could not record tinyhumans health");
                }
                SlotOutcome::HealthFailed(failure.class)
            }
        }
    };

    // 8. Q6 rollback: an `auth` probe undoes only what THIS request wrote to
    // the LLM slots, and never touches the account key or the Composio copy.
    let auth_rejected = matches!(
        health_outcome,
        SlotOutcome::HealthFailed(probe::ProbeClass::Auth)
    );
    if auth_rejected
        && matches!(
            inference_outcome,
            SlotOutcome::Filled | SlotOutcome::Rotated
        )
    {
        let restore = inference_raw_new.clone().unwrap_or_default();
        if let Err(err) = secrets
            .set(company, &inference_key_key, SecretValue(restore))
            .await
        {
            tracing::error!(
                company = %company,
                slot = "inference",
                error = %err,
                "keys rework: could not restore the LLM TinyHumans key after an auth rollback",
            );
        }
        if legacy_raw_is_live {
            let restore_legacy = legacy_raw.clone().unwrap_or_default();
            if let Err(err) = secrets
                .set(company, inference::KEY_KEY, SecretValue(restore_legacy))
                .await
            {
                tracing::error!(
                    company = %company,
                    slot = "inference_legacy",
                    error = %err,
                    "keys rework: could not restore the legacy inference key after an auth rollback",
                );
            }
        }
        inference_outcome = SlotOutcome::RolledBack;
    }

    // 9. Provider row.
    let mut needs_model = false;
    let provider_outcome = if auth_rejected {
        SlotOutcome::Skipped(SkipReason::InferenceRejected)
    } else if legacy_managed {
        SlotOutcome::Skipped(SkipReason::LegacyManagedConfig)
    } else if !holds_new {
        match inference_outcome {
            SlotOutcome::Kept(SkipReason::CustomKey) => SlotOutcome::Skipped(SkipReason::CustomKey),
            _ => SlotOutcome::Skipped(SkipReason::InferenceNotWritten),
        }
    } else if row.is_some() {
        SlotOutcome::Kept(SkipReason::RowExists)
    } else if let Some(chosen) = model.as_deref() {
        let cat = catalogue::cloud_provider(inference::MANAGED_SLUG)
            .expect("tinyhumans is a built-in cloud provider (2a)");
        match inference_store::put_provider(
            company,
            secrets,
            inference_store::ProviderDraft {
                slug: inference::MANAGED_SLUG.to_string(),
                label: cat.label.to_string(),
                kind: inference::MANAGED_SLUG.to_string(),
                base_url: cat.endpoint.to_string(),
                models: tier_overrides(chosen),
                enabled: true,
            },
        )
        .await
        {
            Ok(_) => SlotOutcome::Filled,
            Err(_) => SlotOutcome::Failed,
        }
    } else {
        needs_model = true;
        SlotOutcome::Skipped(SkipReason::NeedsModel)
    };

    // 10. Default.
    let default_outcome = if auth_rejected {
        SlotOutcome::Skipped(SkipReason::InferenceRejected)
    } else if !matches!(default_now, inference_store::DefaultChoice::Unset) {
        SlotOutcome::Kept(SkipReason::DefaultAlreadySet)
    } else {
        match &provider_outcome {
            SlotOutcome::Filled => {
                let chosen = model.clone().expect("a filled row always had a model");
                let choice = inference_store::ModelChoice {
                    provider: inference::MANAGED_SLUG.to_string(),
                    model: chosen,
                };
                match inference_store::set_default_choice(company, secrets, &choice).await {
                    Ok(()) => SlotOutcome::Filled,
                    Err(_) => SlotOutcome::Failed,
                }
            }
            SlotOutcome::Kept(SkipReason::RowExists) => {
                let picked = match &model {
                    Some(sent) => Some(sent.clone()),
                    None => match row.as_ref().map(|r| r.model()) {
                        Some(inference_store::ModelOnRow::One(m)) => Some(m),
                        _ => None,
                    },
                };
                match picked {
                    Some(chosen) => {
                        let choice = inference_store::ModelChoice {
                            provider: inference::MANAGED_SLUG.to_string(),
                            model: chosen,
                        };
                        match inference_store::set_default_choice(company, secrets, &choice).await {
                            Ok(()) => SlotOutcome::Filled,
                            Err(_) => SlotOutcome::Failed,
                        }
                    }
                    None => {
                        needs_model = true;
                        SlotOutcome::Skipped(SkipReason::NeedsModel)
                    }
                }
            }
            other => SlotOutcome::Skipped(provider_gate_reason(other)),
        }
    };

    // 11. The two flags a follow-up (or the console) needs.
    let sets_default = needs_model && matches!(default_now, inference_store::DefaultChoice::Unset);
    let models = if needs_model {
        probe_ids
            .map(|ids| crate::server::ops::inference::providers::catalogue_offer(&ids))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(FanOutReport {
        slots: vec![
            SlotReport {
                slot: Slot::Composio,
                outcome: composio_outcome,
            },
            SlotReport {
                slot: Slot::Inference,
                outcome: inference_outcome,
            },
            SlotReport {
                slot: Slot::Provider,
                outcome: provider_outcome,
            },
            SlotReport {
                slot: Slot::Default,
                outcome: default_outcome,
            },
            SlotReport {
                slot: Slot::Health,
                outcome: health_outcome,
            },
        ],
        needs_model,
        sets_default,
        models,
    })
}

// ---------------------------------------------------------------------------
// fan_out_note — §3.4's fixed sentence table
// ---------------------------------------------------------------------------

/// The plain-words note a `PUT …/credential` answers with, built by joining
/// (with a space) exactly the sentences from §3.4's table whose condition
/// applies, in the table's order. Never contains a key.
pub fn fan_out_note(clearing: bool, report: &FanOutReport, model: Option<&str>) -> String {
    let slot = |s: Slot| report.slots.iter().find(|r| r.slot == s).map(|r| r.outcome);
    let composio = slot(Slot::Composio);
    let inference = slot(Slot::Inference);
    let provider = slot(Slot::Provider);
    let default = slot(Slot::Default);
    let health = slot(Slot::Health);

    let mut sentences: Vec<String> = Vec::new();
    sentences.push(if clearing {
        "Key removed.".to_string()
    } else {
        "Key saved.".to_string()
    });

    if matches!(
        composio,
        Some(SlotOutcome::Filled) | Some(SlotOutcome::Rotated)
    ) {
        sentences.push(
            "Composio now uses this key. A key you created by hand may lack the connections \
             permission Composio needs."
                .to_string(),
        );
    }
    if matches!(composio, Some(SlotOutcome::Kept(SkipReason::CustomKey))) {
        sentences.push("Composio keeps the key set on its own page.".to_string());
    }
    if matches!(composio, Some(SlotOutcome::Cleared)) {
        sentences.push("Composio's copy was removed too.".to_string());
    }
    if matches!(inference, Some(SlotOutcome::Kept(SkipReason::CustomKey))) {
        sentences.push("LLM keeps the TinyHumans key set on its own page.".to_string());
    }
    if let (Some(SlotOutcome::Filled), Some(m)) = (provider, model) {
        sentences.push(format!("TinyHumans is set up for LLM with {m}."));
    }
    if matches!(default, Some(SlotOutcome::Filled)) {
        sentences.push("It is now the default for new work.".to_string());
    }
    if report.needs_model {
        sentences.push("Choose a model to finish setting up TinyHumans for LLM.".to_string());
    }
    if matches!(
        provider,
        Some(SlotOutcome::Skipped(SkipReason::LegacyManagedConfig))
    ) {
        sentences.push("LLM keeps this company's existing TinyHumans setup.".to_string());
    }
    if let Some(SlotOutcome::HealthFailed(class)) = health {
        if class == probe::ProbeClass::Auth {
            sentences.push(
                "TinyHumans rejected this key for LLM, so the LLM copy was not kept.".to_string(),
            );
        } else {
            sentences.push(probe::describe(class, "TinyHumans"));
        }
    }
    if matches!(inference, Some(SlotOutcome::Cleared)) {
        sentences.push(
            "LLM's copy was removed too; TinyHumans stays on the LLM page without a key."
                .to_string(),
        );
    }
    if report
        .slots
        .iter()
        .any(|s| matches!(s.outcome, SlotOutcome::Failed))
    {
        sentences
            .push("Some copies could not be saved — check the LLM and Composio pages.".to_string());
    }

    sentences.join(" ")
}

#[cfg(test)]
mod test;
