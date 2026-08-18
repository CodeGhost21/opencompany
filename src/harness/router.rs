//! [`HarnessRouter`]: sending each agent's turn to the harness it is bound to.
//!
//! ## Why this is a router and not a setting
//!
//! Which engine runs a turn used to be one decision per company, taken at boot
//! from "did an inference credential resolve?". That made two things impossible
//! that a company actually wants: a roster spanning a cheap model and an
//! expensive one, and a single coding agent on the operator's own Claude Code
//! while everyone else stays on the embedded loop.
//!
//! [`RunTurn`] already carries `agent_id` on all three of its methods, so the
//! dispatch point was always there — nothing had ever varied on it. This type is
//! that seam: it holds one inner [`RunTurn`] per declared harness and forwards
//! each call to the one its agent names.
//!
//! ## Resolution, and why unbound agents are not an error
//!
//! An agent naming no harness runs on the company's default. That is not
//! leniency — it is what makes named harnesses additive: every roster written
//! before this existed binds nobody, and all of them must keep working. A
//! *named* harness that does not exist is a different matter and is rejected by
//! manifest validation long before a turn is attempted.
//!
//! ## What a missing engine means
//!
//! A harness can be declared, valid, and still have no engine here — an `acp`
//! harness in a build compiled without the `acp` feature, or a `built_in` one on
//! a host that resolved no inference. Those turns fail with a message naming the
//! harness and the reason, rather than silently falling back to another agent's
//! engine. Falling back would be the worst outcome available: the turn would
//! succeed, on a model and a credential nobody chose, and the only evidence
//! would be a billing line.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;
use crate::company::steer::SteerControl;
use crate::error::OpenCompanyError;
use crate::harness::built_in::TurnOutcome;
use crate::harness::built_in::run_trace::RunTraceSink;
use crate::ports::types::CompanyId;
use crate::runtime::delegation::RunTurn;

/// Routes each agent's turn to the [`RunTurn`] of the harness it is bound to.
pub struct HarnessRouter {
    /// The harness id agents naming none run on.
    default_id: String,
    /// Agent id → harness id, for agents that named one. Agents absent from
    /// this map take [`default_id`](Self::default_id).
    by_agent: HashMap<String, String>,
    /// Harness id → the engine that serves it. A declared harness with no entry
    /// here is one this build or host cannot run; see the module docs.
    engines: HashMap<String, Arc<dyn RunTurn>>,
    /// Why a declared harness has no engine, so the failure can say which
    /// harness and what to do rather than "not found".
    unavailable: HashMap<String, String>,
}

impl HarnessRouter {
    /// A router over `default_id`, with no bindings and no engines yet.
    pub fn new(default_id: impl Into<String>) -> Self {
        Self {
            default_id: default_id.into(),
            by_agent: HashMap::new(),
            engines: HashMap::new(),
            unavailable: HashMap::new(),
        }
    }

    /// Registers the engine serving `harness_id`.
    pub fn with_engine(mut self, harness_id: impl Into<String>, engine: Arc<dyn RunTurn>) -> Self {
        self.engines.insert(harness_id.into(), engine);
        self
    }

    /// Records that `harness_id` was declared but cannot run here, and why.
    ///
    /// `reason` is shown to the operator, so it should name the fix — "this
    /// build has no `acp` feature", not "unsupported".
    pub fn with_unavailable(
        mut self,
        harness_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        self.unavailable.insert(harness_id.into(), reason.into());
        self
    }

    /// Binds `agent_id` to `harness_id`.
    pub fn bind(mut self, agent_id: impl Into<String>, harness_id: impl Into<String>) -> Self {
        self.by_agent.insert(agent_id.into(), harness_id.into());
        self
    }

    /// The harness id `agent_id` runs on.
    pub fn harness_for(&self, agent_id: &str) -> &str {
        self.by_agent
            .get(agent_id)
            .map(String::as_str)
            .unwrap_or(&self.default_id)
    }

    /// The engine for `agent_id`, or the error explaining why there is none.
    fn engine_for(&self, agent_id: &str) -> Result<&Arc<dyn RunTurn>> {
        let harness = self.harness_for(agent_id);
        if let Some(engine) = self.engines.get(harness) {
            return Ok(engine);
        }
        let detail =
            self.unavailable.get(harness).map(String::as_str).unwrap_or(
                "no engine was wired for it — this host cannot run turns on this harness",
            );
        Err(OpenCompanyError::Config(format!(
            "agent `{agent_id}` is bound to harness `{harness}`, but {detail}."
        )))
    }
}

#[async_trait]
impl RunTurn for HarnessRouter {
    async fn run(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        chat_id: Option<&str>,
    ) -> Result<TurnOutcome> {
        self.engine_for(agent_id)?
            .run(company, agent_id, message, chat_id)
            .await
    }

    async fn run_steered(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        control: &SteerControl,
        chat_id: Option<&str>,
        run_sink: Option<Arc<RunTraceSink>>,
    ) -> Result<TurnOutcome> {
        self.engine_for(agent_id)?
            .run_steered(company, agent_id, message, control, chat_id, run_sink)
            .await
    }

    async fn run_steered_background(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        control: &SteerControl,
        run_sink: Option<Arc<RunTraceSink>>,
    ) -> Result<TurnOutcome> {
        self.engine_for(agent_id)?
            .run_steered_background(company, agent_id, message, control, run_sink)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// An engine that records which agent it was asked to run, so a test can
    /// assert on *which* harness served a turn rather than only that one did.
    #[derive(Default)]
    struct SpyEngine {
        label: String,
        seen: Mutex<Vec<String>>,
    }

    impl SpyEngine {
        fn new(label: &str) -> Arc<Self> {
            Arc::new(Self {
                label: label.to_string(),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl RunTurn for SpyEngine {
        async fn run(
            &self,
            _company: &CompanyId,
            agent_id: &str,
            _message: &str,
            _chat_id: Option<&str>,
        ) -> Result<TurnOutcome> {
            self.seen.lock().unwrap().push(agent_id.to_string());
            Ok(TurnOutcome {
                reply: self.label.clone(),
                steps: Vec::new(),
                hit_iteration_cap: false,
            })
        }

        async fn run_steered(
            &self,
            company: &CompanyId,
            agent_id: &str,
            message: &str,
            _control: &SteerControl,
            chat_id: Option<&str>,
            _run_sink: Option<Arc<RunTraceSink>>,
        ) -> Result<TurnOutcome> {
            self.run(company, agent_id, message, chat_id).await
        }

        async fn run_steered_background(
            &self,
            company: &CompanyId,
            agent_id: &str,
            message: &str,
            _control: &SteerControl,
            _run_sink: Option<Arc<RunTraceSink>>,
        ) -> Result<TurnOutcome> {
            self.run(company, agent_id, message, None).await
        }
    }

    fn company() -> CompanyId {
        CompanyId::new("acme")
    }

    /// The headline: two agents in one company, two engines, and each turn goes
    /// to the one its agent named.
    #[tokio::test]
    async fn each_agent_runs_on_the_harness_it_names() {
        let embedded = SpyEngine::new("embedded");
        let deep = SpyEngine::new("deep");
        let router = HarnessRouter::new("embedded")
            .with_engine("embedded", embedded.clone())
            .with_engine("deep", deep.clone())
            .bind("researcher", "deep");

        let out = router
            .run(&company(), "researcher", "hi", None)
            .await
            .unwrap();
        assert_eq!(out.reply, "deep");

        let out = router.run(&company(), "ceo", "hi", None).await.unwrap();
        assert_eq!(out.reply, "embedded", "an unbound agent takes the default");

        assert_eq!(&*deep.seen.lock().unwrap(), &["researcher".to_string()]);
        assert_eq!(&*embedded.seen.lock().unwrap(), &["ceo".to_string()]);
    }

    /// All three `RunTurn` methods route, not just the streamed one. A method
    /// that forwarded to a fixed engine would send *dispatched card* turns to
    /// the wrong model while operator chat looked correct.
    #[tokio::test]
    async fn every_run_turn_method_routes() {
        let embedded = SpyEngine::new("embedded");
        let deep = SpyEngine::new("deep");
        let router = HarnessRouter::new("embedded")
            .with_engine("embedded", embedded.clone())
            .with_engine("deep", deep.clone())
            .bind("researcher", "deep");
        let control = SteerControl::default();

        assert_eq!(
            router
                .run_steered(&company(), "researcher", "hi", &control, None, None)
                .await
                .unwrap()
                .reply,
            "deep"
        );
        assert_eq!(
            router
                .run_steered_background(&company(), "researcher", "hi", &control, None)
                .await
                .unwrap()
                .reply,
            "deep"
        );
        assert!(
            embedded.seen.lock().unwrap().is_empty(),
            "no method leaked to the default engine"
        );
    }

    /// A harness with no engine fails the turn, naming the harness and the
    /// reason. It must never quietly borrow another harness's engine: that turn
    /// would succeed on a model and a credential nobody chose.
    #[tokio::test]
    async fn a_harness_with_no_engine_fails_rather_than_falling_back() {
        let embedded = SpyEngine::new("embedded");
        let router = HarnessRouter::new("embedded")
            .with_engine("embedded", embedded.clone())
            .with_unavailable(
                "my_laptop",
                "this build was compiled without the `acp` feature",
            )
            .bind("coder", "my_laptop");

        let err = router
            .run(&company(), "coder", "hi", None)
            .await
            .expect_err("must not fall back");
        let msg = err.to_string();
        assert!(msg.contains("coder"), "{msg}");
        assert!(msg.contains("my_laptop"), "{msg}");
        assert!(msg.contains("`acp` feature"), "names the fix: {msg}");
        assert!(
            embedded.seen.lock().unwrap().is_empty(),
            "the default engine was never reached"
        );
    }

    /// A binding to a harness nobody declared still fails closed, even though
    /// manifest validation should have caught it first. Defence in depth: the
    /// router is also reachable from runtime-constructed rosters that no
    /// manifest validated.
    #[tokio::test]
    async fn an_unknown_harness_binding_fails_closed() {
        let router = HarnessRouter::new("embedded").with_engine("embedded", SpyEngine::new("e"));
        let err = router
            .run(&company(), "ghost_bound", "hi", None)
            .await
            .expect("agent is unbound, so it takes the default")
            .reply;
        assert_eq!(err, "e");

        let router = router.bind("ghost_bound", "nowhere");
        assert!(
            router
                .run(&company(), "ghost_bound", "hi", None)
                .await
                .is_err()
        );
    }
}
