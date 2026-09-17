use std::cell::RefCell;
use std::collections::HashMap;

use async_trait::async_trait;

use crate::company::inference::probe::{ProbeClass, ProbeFailure};

pub(crate) type Outcome = std::result::Result<Vec<String>, ProbeClass>;

thread_local! {
    static MAP: RefCell<HashMap<String, Outcome>> = RefCell::new(HashMap::new());
}

pub(crate) fn set(company: &str, outcome: Outcome) {
    MAP.with(|map| {
        map.borrow_mut().insert(company.to_string(), outcome);
    });
}

/// Removes a forced outcome, restoring the real prober for that company.
pub(crate) fn clear(company: &str) {
    MAP.with(|map| {
        map.borrow_mut().remove(company);
    });
}

/// Holds a forced outcome for as long as it is alive, then clears it.
pub(crate) struct Scoped(String);

impl Scoped {
    pub(crate) fn set(company: &str, outcome: Outcome) -> Self {
        set(company, outcome);
        Self(company.to_string())
    }
}

impl Drop for Scoped {
    fn drop(&mut self) {
        clear(&self.0);
    }
}

pub(super) fn get(company: &str) -> Option<Outcome> {
    MAP.with(|map| map.borrow().get(company).cloned())
}

pub(super) struct Forced(pub(super) Outcome);

#[async_trait]
impl crate::company::company_key::InferenceProber for Forced {
    async fn probe(
        &self,
        _base_url: &str,
        _key: &str,
    ) -> std::result::Result<Vec<String>, ProbeFailure> {
        match &self.0 {
            Ok(ids) => Ok(ids.clone()),
            Err(class) => Err(ProbeFailure {
                class: *class,
                raw: "forced".to_string(),
                truncated: false,
            }),
        }
    }
}
