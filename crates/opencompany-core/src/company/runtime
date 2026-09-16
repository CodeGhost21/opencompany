    /// A [`JournalStore`](crate::ports::journal::JournalStore) that refuses
    /// every append once armed — a full or read-only data volume, which is the
    /// failure mode `park_blocker`'s rollback exists for.
    ///
    /// Armed by the test rather than from birth, so boot's own journal writes
    /// still land and the runtime under test is an ordinary one that lost its
    /// volume mid-life.
    #[cfg(feature = "openhuman")]
    #[derive(Default)]
    struct RefusingJournalStore {
        inner: crate::ports::journal::MemoryJournalStore,
        armed: std::sync::atomic::AtomicBool,
        allow_before_failing: std::sync::atomic::AtomicUsize,
    }

    #[cfg(feature = "openhuman")]
    impl RefusingJournalStore {
        fn arm(&self) {
            self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        /// Lets appends land again — the volume coming back after a transient
        /// failure.
        fn disarm(&self) {
            self.armed.store(false, std::sync::atomic::Ordering::SeqCst);
        }

        /// Once armed, lets the next `n` appends land before refusing —
        /// so the failure can be aimed at a later write in the same request
        /// rather than the very first one.
        fn allow_next(&self, n: usize) {
            self.allow_before_failing
                .store(n, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[cfg(feature = "openhuman")]
    #[async_trait::async_trait]
    impl crate::ports::journal::JournalStore for RefusingJournalStore {
        async fn append_journal(
            &self,
            id: &crate::ports::types::CompanyId,
            line: &str,
            durability: crate::ports::journal::Durability,
        ) -> crate::Result<()> {
            if self.armed.load(std::sync::atomic::Ordering::SeqCst) {
                let remaining = self
                    .allow_before_failing
                    .load(std::sync::atomic::Ordering::SeqCst);
                if remaining > 0 {
                    self.allow_before_failing
                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                } else {
                    return Err(crate::error::OpenCompanyError::Store(
                        "RefusingJournalStore: the volume is full".to_string(),
                    ));
                }
            }
            self.inner.append_journal(id, line, durability).await
        }

        async fn read_journal(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<Vec<String>> {
            self.inner.read_journal(id).await
        }

        async fn journal_imported(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<bool> {
            self.inner.journal_imported(id).await
        }

        async fn complete_import(
            &self,
            id: &crate::ports::types::CompanyId,
            lines: Vec<String>,
        ) -> crate::Result<()> {
            self.inner.complete_import(id, lines).await
        }
    }
    /// A journal that lets a **competing** request run to completion inside the
    /// next append, so a race needing one caller suspended mid-`await` is
    /// exercised deterministically rather than by hoping two tasks interleave.
    #[cfg(feature = "openhuman")]
    #[derive(Default)]
    struct RacingJournalStore {
        inner: crate::ports::journal::MemoryJournalStore,
        interleave: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    #[cfg(feature = "openhuman")]
    impl RacingJournalStore {
        /// Runs `run` once, inside the next append.
        fn interleave_next(&self, run: impl FnOnce() + Send + 'static) {
            *self.interleave.lock().expect("interleave poisoned") = Some(Box::new(run));
        }
    }

    #[cfg(feature = "openhuman")]
    #[async_trait::async_trait]
    impl crate::ports::journal::JournalStore for RacingJournalStore {
        async fn append_journal(
            &self,
            id: &crate::ports::types::CompanyId,
            line: &str,
            durability: crate::ports::journal::Durability,
        ) -> crate::Result<()> {
            let racer = self.interleave.lock().expect("interleave poisoned").take();
            if let Some(racer) = racer {
                racer();
            }
            self.inner.append_journal(id, line, durability).await
        }

        async fn read_journal(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<Vec<String>> {
            self.inner.read_journal(id).await
        }

        async fn journal_imported(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<bool> {
            self.inner.journal_imported(id).await
        }

        async fn complete_import(
            &self,
            id: &crate::ports::types::CompanyId,
            lines: Vec<String>,
        ) -> crate::Result<()> {
            self.inner.complete_import(id, lines).await
        }
    }

    use super::{
        CompanyEvent, continuation_failure_notice, emergency_from_load, task_enters_in_progress,
        task_enters_planning,
    };
    use crate::ports::tasks::TaskTitle;

    /// Issue #880: which parked approvals name a workflow run, and which must
    /// not.
    ///
    /// The discrimination is the whole content of the change, because
    /// `Effect::run_id` carries two id spaces — issue #242's task attempt and
    /// the workflow run — and `generate_id` is only process-locally unique, so
    /// the value cannot be inspected to tell them apart. Getting this wrong in
    /// the permissive direction would print a task-attempt id on an approvals
    /// card as though it were a workflow run.
    #[test]
    fn only_an_unlinked_park_with_a_run_id_names_a_workflow_run() {
        use crate::ports::types::{ApprovalId, Effect, EffectGroup};
        use crate::runtime::journal::{PendingApproval, TaskLink};

        let parked = |task: Option<TaskLink>, run_id: Option<&str>| PendingApproval {
            id: ApprovalId::new("appr-1"),
            effect: Effect {
                kind: "publish_artifact".to_string(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::json!({}),
                agent: Some("ceo".to_string()),
                run_id: run_id.map(str::to_string),
            },
            at_millis: 1,
            deadline_anchor_millis: 1,
            task,
            thread: None,
            batch: None,
        };

        // A workflow park: `park_and_journal` records it explicitly unlinked
        // (#333) and the run stamps its id.
        assert_eq!(
            super::workflow_run_of(&parked(Some(TaskLink::Unlinked), Some("run-1"))),
            Some("run-1".to_string())
        );
        // A board task's attempt: same field, different id space. Must NOT be
        // reported as a workflow run.
        assert_eq!(
            super::workflow_run_of(&parked(
                Some(TaskLink::Task {
                    id: "card-1".to_string()
                }),
                Some("attempt-1")
            )),
            None
        );
        // A chat turn: unlinked, but nothing stamped a run onto it.
        assert_eq!(
            super::workflow_run_of(&parked(Some(TaskLink::Unlinked), None)),
            None
        );
        // A pre-#333 line records no link at all, so the park site is unknown.
        // Conservative rather than guessing — the same fallback rule #333 set.
        assert_eq!(super::workflow_run_of(&parked(None, Some("run-1"))), None);
    }

    /// Issue #1092: a continuation whose approval was raised in no conversation
    /// must never be journaled into the answering teammate's DM.
    ///
    /// The fallback is the whole content of the fix, so it is asserted per park
    /// site rather than through one happy path: the id it returns is what
    /// `chat_history::owns` will (or will not) resolve to a chat thread.
    #[test]
    fn a_continuation_with_no_conversation_answers_outside_every_chat() {
        use crate::runtime::journal::{ApprovalOrigin, TaskLink};

        let origin = |task: Option<TaskLink>, run_id: Option<&str>| ApprovalOrigin {
            at_millis: 1,
            kind: "web_fetch".to_string(),
            task,
            run_id: run_id.map(str::to_string),
            thread: None,
            parent: None,
            cycle: None,
        };

        // A workflow node's parked call: unlinked, with the run stamped on it.
        // The run id is the destination — the timeline the operator was already
        // watching, and a value no desk answers to.
        assert_eq!(
            super::continuation_fallback_chat_id(Some(&origin(
                Some(TaskLink::Unlinked),
                Some("run-9")
            ))),
            "run-9",
        );
        // A board card's dispatch: the card owns the work, exactly as
        // `journal_task_outcome` already records it.
        assert_eq!(
            super::continuation_fallback_chat_id(Some(&origin(
                Some(TaskLink::Task {
                    id: "card-3".to_string()
                }),
                Some("attempt-4"),
            ))),
            "card-3",
        );
        // Unlinked with nothing stamped is an unaddressed operator turn, and a
        // pre-#333 line with no link at all is unknown. Both answer in General
        // — visible to the person who approved, and never a teammate's DM.
        assert_eq!(
            super::continuation_fallback_chat_id(Some(&origin(Some(TaskLink::Unlinked), None))),
            "General",
        );
        assert_eq!(
            super::continuation_fallback_chat_id(Some(&origin(None, Some("run-9")))),
            "General",
        );
        assert_eq!(super::continuation_fallback_chat_id(None), "General");
    }

    /// Issue #1092, the property that actually matters: a workflow park's
    /// continuation must not resolve to a teammate's DM or to a desk.
    ///
    /// Asserted through `chat_history::owns` itself rather than by eyeballing
    /// the string, so a change on either side fails here instead of silently
    /// re-opening the leak. The General arm is asserted the other way round in
    /// the same breath — it is *supposed* to be readable — because a fallback
    /// that hid every continuation would pass a one-directional test and lose
    /// the operator's answer.
    #[test]
    fn a_workflow_parks_continuation_owns_no_desk_and_no_dm() {
        use crate::ports::types::CompanyEvent;
        use crate::runtime::journal::{ApprovalOrigin, TaskLink};
        use crate::server::chat_history::owns;

        let reply = |chat_id: String| CompanyEvent::AgentReply {
            audience: Vec::new(),
            mentions: Vec::new(),
            mention_depth: 0,
            parent: None,
            chat_id,
            agent_id: "copywriter".to_string(),
            text: "re-issued".to_string(),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
        };
        let origin = |task: Option<TaskLink>, run_id: Option<&str>| ApprovalOrigin {
            at_millis: 1,
            kind: "web_fetch".to_string(),
            task,
            run_id: run_id.map(str::to_string),
            thread: None,
            parent: None,
            cycle: None,
        };

        // The leak: a workflow node's park, answered into the copywriter's DM.
        let workflow = super::continuation_fallback_chat_id(Some(&origin(
            Some(TaskLink::Unlinked),
            Some("run-9"),
        )));
        for (desk_id, desk_name) in [
            ("copywriter", "Copywriter"),
            ("creative", "Creative studio"),
        ] {
            assert!(
                !owns(desk_id, desk_name, &reply(workflow.clone())),
                "`{workflow}` must not be read as the `{desk_id}` conversation",
            );
        }

        // And the other direction: an unaddressed operator turn still answers
        // somewhere the person who approved is looking.
        let unaddressed =
            super::continuation_fallback_chat_id(Some(&origin(Some(TaskLink::Unlinked), None)));
        assert!(
            owns("main", "General", &reply(unaddressed.clone())),
            "`{unaddressed}` must still be read as the operator's General line",
        );
    }

    #[cfg(feature = "openhuman")]
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "openhuman")]
    use async_trait::async_trait;

    #[cfg(feature = "openhuman")]
    #[derive(Default)]
    struct RecordingMeter {
        queried_companies: Mutex<Vec<crate::ports::types::CompanyId>>,
    }

    #[cfg(feature = "openhuman")]
    #[async_trait]
    impl crate::ports::UsageMeter for RecordingMeter {
        async fn record(
            &self,
            _company: &crate::ports::types::CompanyId,
            _sample: &crate::ports::UsageSample,
        ) -> crate::Result<()> {
            Ok(())
        }

        async fn query(
            &self,
            company: &crate::ports::types::CompanyId,
            _since_millis: u64,
        ) -> crate::Result<Vec<crate::ports::UsageSample>> {
            self.queried_companies.lock().unwrap().push(company.clone());
            Ok(Vec::new())
        }
    }

    /// `is_busy` must see **all three** sources, not just the steer registry.
    ///
    /// The first version of the busy endpoint read only `steer.any_inflight()`,
    /// which covers dispatched board cards and desk delegations. A top-level
    /// operator chat turn registers none of those — it takes `serial` and
    /// nothing else — and workflow runs live in `run_supervisor`, a separate
    /// registry. So the 15-minute turn opencompany-microservice#22 measured
    /// reported `busy: false` and got parked mid-flight, which is exactly the
    /// failure the endpoint exists to prevent.
    ///
    /// Each source is exercised idle → busy → idle independently, so dropping
    /// any one of them from `is_busy` fails here rather than silently in
    /// production. Deliberately outside any feature gate: the steer registry is
    /// only wired under `openhuman`, so a test that relied on it alone would not
    /// run in the default build at all.
    /// **B-037, the other half.** Pausing a company stops the runs it already
    /// has in flight, not only the ones it would have started next.
    ///
    /// Refusing new runs was the reported symptom; this is the promise on the
    /// same settings screen. A graph twenty nodes into thirty goes on spending
    /// until it finishes, and the operator who pressed Pause — usually because
    /// of that run — had no control that reached it.
    #[tokio::test]
    async fn pausing_a_company_stops_the_runs_already_in_flight() {
        let (runtime, _record, _home) = runtime_and_record().await;

        let (ctx, _guard) = runtime
            .run_supervisor()
            .begin("wf-1", false)
            .expect("begin a workflow run");
        assert!(
            !ctx.cancel.is_cancelled(),
            "the run starts un-cancelled, or this test proves nothing"
        );

        runtime
            .set_lifecycle(
                "paused",
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: "operator".into(),
                },
            )
            .await
            .expect("pause the company");

        assert!(
            ctx.cancel.is_cancelled(),
            "pausing must fire the stop signal on a run already executing"
        );
    }

    /// The mirror, so the sweep cannot quietly become "cancel on every
    /// transition": **resuming** must not stop the work it is resuming into.
    ///
    /// A resume runs through the same `set_lifecycle`, so a guard keyed on the
    /// wrong side of the comparison would kill runs at exactly the moment the
    /// operator asked for them to continue.
    #[tokio::test]
    async fn resuming_a_company_does_not_stop_anything() {
        let (runtime, _record, _home) = runtime_and_record().await;

        let (ctx, _guard) = runtime
            .run_supervisor()
            .begin("wf-1", false)
            .expect("begin a workflow run");

        runtime
            .set_lifecycle(
                "running",
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: "operator".into(),
                },
            )
            .await
            .expect("resume the company");

        assert!(
            !ctx.cancel.is_cancelled(),
            "resuming must leave a live run alone"
        );
    }

    #[tokio::test]
    async fn is_busy_sees_every_source_of_work() {
        let (runtime, _record, _home) = runtime_and_record().await;
        assert!(!runtime.is_busy(), "an idle runtime must not report busy");

        // 1. The cycle lock — the operator-chat case the steer registry misses.
        {
            let _cycle = runtime.serial.lock().await;
            assert!(
                runtime.is_busy(),
                "a turn holding the cycle lock must report busy"
            );
        }
        assert!(!runtime.is_busy(), "releasing the cycle lock must clear it");

        // 2. A workflow run — tracked in its own registry, invisible to both
        //    the cycle lock and the steer registry.
        {
            let (_ctx, _run) = runtime
                .run_supervisor()
                .begin("wf-1", false)
                .expect("begin a workflow run");
            assert!(runtime.is_busy(), "a live workflow run must report busy");
        }
        assert!(
            !runtime.is_busy(),
            "the run guard must clear it on drop, or the tenant never parks again"
        );

        // 3. A steerable in-flight run — the original signal, kept because a
        //    dispatched card can outlive the cycle that started it.
        {
            let _guard = runtime.steer().register(
                runtime.id(),
                crate::company::steer::InflightEntry {
                    key: "run-1".to_string(),
                    task_id: Some("run-1".to_string()),
                    kind: crate::company::steer::InflightKind::Task,
                    title: "Ship the thing".to_string(),
                    agent_id: "ceo".to_string(),
                    started_at_millis: 0,
                    pending_action: None,
                },
            );
            assert!(runtime.is_busy(), "a registered steer run must report busy");
        }
        assert!(!runtime.is_busy(), "the steer guard must clear it on drop");
    }

    /// A poisoned run supervisor must make `is_busy` report **busy**.
    ///
    /// The predicate's advertised invariant is that it fails closed, and #1133
    /// only delivered that for two of its three sources: `steer.any_inflight`
    /// was made poison-tolerant, but the `run_supervisor` arm still reached a
    /// `.expect` through `len`. `GET /healthz/busy` has no `CatchPanicLayer`, so
    /// that panic reset the connection, the manager read it as "cannot tell",
    /// and its default is to park — losing the work the endpoint exists to
    /// protect (issue #1239).
    ///
    /// Outside any feature gate on purpose, matching
    /// `is_busy_sees_every_source_of_work`: the run supervisor is wired on the
    /// default build, and this must not be a test that only CI's `openhuman`
    /// lane runs.
    #[tokio::test]
    async fn is_busy_fails_closed_on_a_poisoned_run_supervisor() {
        let (runtime, _record, _home) = runtime_and_record().await;
        assert!(!runtime.is_busy(), "an idle runtime must not report busy");

        runtime.run_supervisor().poison_for_test();

        assert!(
            runtime.is_busy(),
            "a poisoned run supervisor must report busy rather than panic in the handler"
        );
    }

    /// Codex review (#1865): "Plan first" on a bounced card is a fresh
    /// attempt exactly like a re-dispatch, so the stale bounce chip must not
    /// survive the To-do → Planning edge either.
    ///
    /// No harness/planner wired — the default shape ~200 callers use — so
    /// `plan_task`'s spawn is a no-op and this exercises only the synchronous
    /// clearing `upsert_task` does before it, matching the inert-board
    /// pattern `runtime::builder::test` already uses for the sibling
    /// dispatch edge.
    #[tokio::test]
    async fn planning_first_clears_a_stale_bounce_chip_same_as_a_redispatch() {
        use crate::ports::tasks::{COLUMN_PLANNING, COLUMN_TODO, TaskRecord};

        let home = tempfile::tempdir().expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
        )
        .expect("manifest");
        let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(crate::ports::types::CompanyId::new("acme"))
            .build()
            .await
            .expect("runtime");
        let runtime = std::sync::Arc::new(runtime);

        let card = TaskRecord {
            id: "card-1".to_string(),
            title: TaskTitle::authored("Draft the spec"),
            note: None,
            column: COLUMN_TODO.to_string(),
            priority: "medium".to_string(),
            assignee: "ceo".to_string(),
            updated_at_millis: 1,
            origin: None,
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            // A stale chip from a dispatch attempt that already bounced.
            bounced: Some("a previous run's dispatch failed".to_string()),
        };
        runtime
            .upsert_task(&card)
            .await
            .expect("seed the bounced card in To-do");

        let mut planned = card.clone();
        planned.column = COLUMN_PLANNING.to_string();
        runtime
            .upsert_task(&planned)
            .await
            .expect("drag it into Planning");

        let after = runtime
            .tasks()
            .list(runtime.id())
            .await
            .expect("list")
            .into_iter()
            .find(|t| t.id == "card-1")
            .expect("card survives");
        assert_eq!(
            after.bounced, None,
            "entering Planning must clear the previous dispatch's bounce chip, not carry it \
             through to whatever the planning pass settles next"
        );
    }

    /// Codex review on PR #1883 (comment 3874654383): `patch_task` accepts
    /// any board column on one write, so an operator can move a bounced
    /// To-do card straight to `done` — a departure that touches neither the
    /// dispatch nor the planning edge. The manual move supersedes the bounce
    /// exactly as much as a re-dispatch does, and
    /// [`crate::ports::tasks::TaskRecord::bounced`]'s own doc promises it
    /// clears "the instant the card leaves `todo` any other way" — this
    /// proves the "any other way" case, not just the two edge-fired ones the
    /// sibling test above covers.
    ///
    /// Before the fix, `upsert_task` only cleared `bounced` when
    /// `dispatch || plan`, so this direct To-do → Done transition left the
    /// stale chip in place — and it would have resurfaced if the card later
    /// came back to To-do, naming a failure the intervening manual move had
    /// already superseded.
    #[tokio::test]
    async fn a_direct_move_to_done_clears_a_stale_bounce_chip() {
        use crate::ports::tasks::{COLUMN_DONE, COLUMN_TODO, TaskRecord};

        let home = tempfile::tempdir().expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
        )
        .expect("manifest");
        let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(crate::ports::types::CompanyId::new("acme"))
            .build()
            .await
            .expect("runtime");
        let runtime = std::sync::Arc::new(runtime);

        let card = TaskRecord {
            id: "card-2".to_string(),
            title: TaskTitle::authored("Draft the spec"),
            note: None,
            column: COLUMN_TODO.to_string(),
            priority: "medium".to_string(),
            assignee: "ceo".to_string(),
            updated_at_millis: 1,
            origin: None,
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            // A stale chip from a dispatch attempt that already bounced.
            bounced: Some("a previous run's dispatch failed".to_string()),
        };
        runtime
            .upsert_task(&card)
            .await
            .expect("seed the bounced card in To-do");

        let mut done = card.clone();
        done.column = COLUMN_DONE.to_string();
        runtime
            .upsert_task(&done)
            .await
            .expect("drag it straight to Done");

        let after = runtime
            .tasks()
            .list(runtime.id())
            .await
            .expect("list")
            .into_iter()
            .find(|t| t.id == "card-2")
            .expect("card survives");
        assert_eq!(
            after.bounced, None,
            "a direct To-do → Done move must clear the stale bounce chip too — the operator's \
             manual transition supersedes the reason it named, and the chip must not resurface \
             if the card ever comes back to To-do"
        );
    }

    async fn runtime_and_record() -> (
        super::CompanyRuntime,
        crate::ports::CompanyRecord,
        tempfile::TempDir,
    ) {
        let home = tempfile::tempdir().expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(crate::ports::types::CompanyId::new("acme"))
            .build()
            .await
            .expect("runtime");
        let record = runtime
            .store()
            .load(runtime.id())
            .await
            .expect("load")
            .expect("record");
        (runtime, record, home)
    }

    /// `set_lifecycle` must serialize its load-modify-save cycle against
    /// `company_write_lock`, exactly like every other console load-modify-save
    /// (PR #1875 review finding, second round). Proven the same way
    /// `put_logo_serializes_against_the_company_write_lock`
    /// (`server/ops/company_logo.rs`) proves it for that handler: hold the
    /// lock externally, drive the real method, and demand it cannot finish
    /// while the lock is held.
    #[tokio::test]
    async fn set_lifecycle_serializes_against_the_company_write_lock() {
        let (runtime, _record, _home) = runtime_and_record().await;
        let runtime = std::sync::Arc::new(runtime);
        let id = runtime.id().clone();

        let lock = crate::ports::store::company_write_lock(&id);
        let guard = lock.lock().await;

        let runtime_for_task = runtime.clone();
        let mut task = tokio::spawn(async move {
            runtime_for_task
                .set_lifecycle(
                    "paused",
                    crate::ports::types::Actor {
                        kind: crate::ports::types::ActorKind::Operator,
                        id: "op".to_string(),
                    },
                )
                .await
        });

        // The method must be blocked behind the held lock — give it every
        // chance to (wrongly) race ahead before declaring it stuck.
        let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
            .await
            .is_ok();
        assert!(
            !raced_ahead,
            "set_lifecycle completed while company_write_lock was held \
             elsewhere — it is not serializing its load-modify-save cycle \
             against concurrent writers (e.g. a racing name-confirm PATCH)"
        );

        drop(guard);
        let from = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("set_lifecycle never resumed after the lock was released")
            .expect("task panicked")
            .expect("set_lifecycle failed");
        assert_eq!(from, "running", "the fixture starts running");
    }

    /// The shared workflow-wiring fixture, re-exported under the name these
    /// tests already use.
    #[cfg(feature = "openhuman")]
    use crate::harness::workflow_wiring_deps as wiring_deps;

    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn workflow_wiring_is_absent_without_harness_deps() {
        let (runtime, record, _home) = runtime_and_record().await;
        assert_eq!(runtime.wired_workflow_namespaces(&record).await, None);
    }

    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn workflow_wiring_keeps_the_static_capability_filter_without_a_plan() {
        let (mut runtime, record, _home) = runtime_and_record().await;
        runtime.set_workflow_harness_deps(wiring_deps(
            &runtime,
            None,
            crate::harness::toolbelt::CapabilityFilter::DenyNamespaces(
                ["web"].into_iter().collect(),
            ),
            None,
        ));
        let namespaces = runtime
            .wired_workflow_namespaces(&record)
            .await
            .expect("wiring");
        assert!(!namespaces.contains("web"));
        assert!(namespaces.contains("shell"));
    }

    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn workflow_wiring_resolves_the_plan_against_its_company_meter() {
        let (mut runtime, record, _home) = runtime_and_record().await;
        let meter = Arc::new(RecordingMeter::default());
        runtime.set_workflow_harness_deps(wiring_deps(
            &runtime,
            Some(meter.clone()),
            crate::harness::toolbelt::CapabilityFilter::AllowAll,
            Some(crate::harness::capability_budget::CapabilityPlan {
                period: crate::harness::capability_budget::BudgetPeriod::Daily,
                budgets: [("shell".to_string(), u64::MAX)].into_iter().collect(),
                total_budget: None,
            }),
        ));
        let namespaces = runtime
            .wired_workflow_namespaces(&record)
            .await
            .expect("wiring");
        assert!(namespaces.contains("shell"));
        assert!(!namespaces.contains("web"));
        assert!(!namespaces.contains("code"));
        assert_eq!(*meter.queried_companies.lock().unwrap(), vec![record.id]);
    }

    /// Issue #874: the wiring carries **why** a namespace is out, not just that
    /// it is — the two reasons `refusal_for` renders at run time, so a caller
    /// (the `tool-slugs` route) can tell an operator "no provider configured"
    /// apart from "your capability tier filtered it" before a run fails.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn workflow_wiring_names_why_each_namespace_is_unwired() {
        let (mut runtime, record, _home) = runtime_and_record().await;
        // `wiring_deps` leaves `search: None` — the staging shape in issue #874,
        // where `searchCredentialConfigured` was false — and we deny `web` on top
        // so both reasons appear in one map.
        runtime.set_workflow_harness_deps(wiring_deps(
            &runtime,
            None,
            crate::harness::toolbelt::CapabilityFilter::DenyNamespaces(
                ["web"].into_iter().collect(),
            ),
            None,
        ));
        let wiring = runtime.workflow_tool_wiring(&record).await.expect("wiring");
        assert_eq!(
            wiring.missing.get("search").copied(),
            Some(crate::workflows::caps::MissingReason::SearchBackendNotConfigured),
            "no search backend is configured: {:?}",
            wiring.missing
        );
        assert_eq!(
            wiring.missing.get("web").copied(),
            Some(crate::workflows::caps::MissingReason::CapabilityTierFiltered),
            "web is denied by the capability filter: {:?}",
            wiring.missing
        );
        assert!(
            !wiring.missing.contains_key("shell"),
            "a wired namespace carries no reason: {:?}",
            wiring.missing
        );
    }

    /// Issue #874, the staging repro at the layer the route reads: a company that
    /// explicitly grants `search` on a deployment with **no** search backend must
    /// not be offered `web_search` for grounding — it must be reported as granted
    /// but unwired instead, so the copilot cannot author a node that dies at the
    /// first run.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_granted_but_unwired_tool_is_reported_not_offered() {
        let (mut runtime, mut record, _home) = runtime_and_record().await;
        record.manifest.tools.allow.push("search".to_string());
        record.manifest.tools.allow.push("shell".to_string());
        runtime.set_workflow_harness_deps(wiring_deps(
            &runtime,
            None,
            crate::harness::toolbelt::CapabilityFilter::AllowAll,
            None,
        ));
        let wiring = runtime.workflow_tool_wiring(&record).await;
        let wired = wiring.as_ref().map(|w| &w.wired_namespaces);

        let effective = crate::company::workflow_effective_tool_slugs(&record, wired);
        let unwired = crate::company::workflow_granted_but_unwired_tool_slugs(&record, wired);
        assert!(
            !effective.iter().any(|slug| slug == "web_search"),
            "an unwired search tool is not offered for grounding: {effective:?}"
        );
        assert!(
            unwired.iter().any(|slug| slug == "web_search"),
            "…but it IS reported as granted-and-unwired: {unwired:?}"
        );
        assert!(
            effective.iter().any(|slug| slug == "shell"),
            "a granted AND wired tool is still offered: {effective:?}"
        );
        // The two lists partition the granted set: nothing may appear in both, or
        // a caller grounding on one and warning from the other contradicts itself.
        assert!(
            !effective.iter().any(|slug| unwired.contains(slug)),
            "effective {effective:?} and unwired {unwired:?} overlap"
        );
    }

    /// The other half of the honesty split: with no harness deps the wiring is
    /// *unknowable*, so every granted tool stays offered and nothing is claimed
    /// to be unwired. Reporting "all granted tools are broken" on a host that
    /// simply cannot say would be the worse failure.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn unknowable_wiring_offers_the_grant_only_set_and_reports_nothing_unwired() {
        let (runtime, mut record, _home) = runtime_and_record().await;
        record.manifest.tools.allow.push("search".to_string());
        let wiring = runtime.workflow_tool_wiring(&record).await;
        assert!(wiring.is_none(), "no harness deps means no wiring answer");
        let wired = wiring.as_ref().map(|w| &w.wired_namespaces);

        assert!(
            crate::company::workflow_effective_tool_slugs(&record, wired)
                .iter()
                .any(|slug| slug == "web_search"),
            "a granted tool is still offered when the deployment cannot be asked"
        );
        assert!(
            crate::company::workflow_granted_but_unwired_tool_slugs(&record, wired).is_empty(),
            "nothing is claimed unwired when the deployment cannot be asked"
        );
    }

    /// Issue #86: the kill switch's boot decision, including the direction it
    /// fails in.
    ///
    /// The `Err` arm is the whole point. An unreadable log must not un-pause a
    /// company an operator deliberately stopped: a company wrongly stopped is a
    /// visible problem someone fixes with one request, while a company wrongly
    /// running is exactly the outcome the endpoint exists to prevent, and
    /// nothing would surface it.
    #[test]
    fn an_unreadable_record_comes_up_stopped() {
        assert!(emergency_from_load(Err(
            crate::error::OpenCompanyError::CompanyNotFound("acme".into())
        )));
    }

    /// The other three arms, which must stay distinct from the error case.
    #[test]
    fn a_readable_record_is_taken_at_its_word() {
        // Stopped stays stopped across the restart.
        assert!(emergency_from_load(Ok(Some(true))));
        // Running stays running — the switch is not sticky by accident.
        assert!(!emergency_from_load(Ok(Some(false))));
        // Nothing known is not the same as a read failure.
        assert!(!emergency_from_load(Ok(None)));
    }

    /// Issue #337: the planning edge, on the same terms as the dispatch one.
    /// Entering the column fires; resting in it does not.
    #[test]
    fn planning_fires_only_on_entering_planning() {
        // The drag this feature exists for.
        assert!(task_enters_planning(Some("todo"), "planning"));
        // A card created straight into Planning is a genuine entry too.
        assert!(task_enters_planning(None, "planning"));
        // Already planning, re-saved — an edit, the pass's own note append, a
        // re-title. This is what makes "one pass per entry, no retry" a
        // property of the edge rather than a rule the planner has to remember,
        // and it is what stops the settle's own write re-triggering the pass.
        assert!(!task_enters_planning(Some("planning"), "planning"));
        // Leaving Planning never fires it — including the success settle.
        assert!(!task_enters_planning(Some("planning"), "in_progress"));
        assert!(!task_enters_planning(Some("planning"), "todo"));
        // No other column entry fires it.
        for column in ["todo", "in_progress", "paused", "in_review", "done"] {
            assert!(!task_enters_planning(Some("todo"), column), "{column}");
        }
    }

    /// Issue #576: a prompt-box card buys **exactly one** planning pass across
    /// its whole life — not zero, not two.
    ///
    /// The assertions above pin the edge one transition at a time. This walks
    /// the sequence a self-promoting card actually goes through and *counts*,
    /// because the two ways to get this wrong are both invisible to a
    /// single-transition test:
    ///
    /// * **Zero** — the card is created directly in `planning` rather than
    ///   moved there, so if entry required a previous column there would be no
    ///   transition to observe and the pass would never fire. The card would sit
    ///   in Planning forever, which is the one column that must never hold a
    ///   card at rest.
    /// * **Two** — the pass writes its plan back onto the card *while the card
    ///   is still in Planning* (`harness::planning`, via `upsert_task`). If
    ///   resting in the column counted as entering it, that write-back would
    ///   start a second pass, which would write back, and bill a model call each
    ///   time.
    ///
    /// A test that merely asserted "it planned" would pass in the second case.
    #[test]
    fn a_prompt_box_card_buys_exactly_one_planning_pass() {
        // The life of a card opened from the prompt box: created directly in
        // Planning, its plan written back while it rests there, then settled
        // onward by the pass itself.
        let life = [
            (None, "planning"),                // the prompt box opens it
            (Some("planning"), "planning"),    // the pass writes the plan back
            (Some("planning"), "in_progress"), // the success settle
        ];
        let fires = life
            .iter()
            .filter(|(prev, next)| task_enters_planning(*prev, next))
            .count();
        assert_eq!(
            fires, 1,
            "a prompt-box card must buy exactly one planning pass: {life:?}"
        );

        // And the failure exit, which returns the card to To-do, must not buy a
        // second one on the way out either.
        let returned = [(None, "planning"), (Some("planning"), "todo")];
        assert_eq!(
            returned
                .iter()
                .filter(|(prev, next)| task_enters_planning(*prev, next))
                .count(),
            1,
            "a pass that returned the card must still have cost exactly one"
        );
    }

    /// The two edges are mutually exclusive by construction: one write names
    /// one target column, so no upsert can both plan and dispatch a card. This
    /// is what makes the "planning happens BEFORE dispatch" ordering structural
    /// rather than a matter of which `if` runs first in `upsert_task`.
    #[test]
    fn no_single_write_both_plans_and_dispatches() {
        for prev in [None, Some("todo"), Some("planning"), Some("in_progress")] {
            for next in [
                "todo",
                "planning",
                "in_progress",
                "paused",
                "in_review",
                "done",
            ] {
                assert!(
                    !(task_enters_planning(prev, next) && task_enters_in_progress(prev, next)),
                    "{prev:?} → {next} fires both edges"
                );
            }
        }
    }

    /// The success settle's shape, pinned end to end: a pass that clears the
    /// card writes `planning → in_progress`, which is NOT a planning entry (so
    /// it cannot loop) and IS a dispatch entry (so the plan actually hands the
    /// work on). Both halves matter; either one alone would be a bug.
    #[test]
    fn a_cleared_plan_hands_the_card_on_without_replanning_it() {
        assert!(
            !task_enters_planning(Some("planning"), "in_progress"),
            "the settle must not re-enter the pass it is settling"
        );
        assert!(
            task_enters_in_progress(Some("planning"), "in_progress"),
            "the settle must fire the dispatch edge — that is why it routes \
             through upsert_task rather than the plain store port"
        );
    }

    #[test]
    fn dispatch_only_on_entering_in_progress() {
        // Fresh card created straight into `in_progress` → dispatch.
        assert!(task_enters_in_progress(None, "in_progress"));
        // The drag: todo → in_progress → dispatch.
        assert!(task_enters_in_progress(Some("todo"), "in_progress"));
        // Issue #301: planning sits before dispatch, so entering it must not
        // fire one — and leaving it for `in_progress` must.
        assert!(!task_enters_in_progress(Some("todo"), "planning"));
        assert!(task_enters_in_progress(Some("planning"), "in_progress"));
        // Already in_progress, re-saved (e.g. an edit) → no re-dispatch.
        assert!(!task_enters_in_progress(Some("in_progress"), "in_progress"));
        // Any non-in_progress target → no dispatch.
        assert!(!task_enters_in_progress(Some("in_progress"), "in_review"));
        assert!(!task_enters_in_progress(None, "todo"));
        assert!(!task_enters_in_progress(Some("in_review"), "done"));
    }

    /// Issue #246 spend gate. A card opened from chat goes through
    /// `POST …/tasks` with **no** `column`, so what stops it from spending
    /// money the operator never approved is that the server's default column is
    /// not the dispatch trigger. That is two independent facts — what the
    /// default is, and what the trigger is — living in two different modules,
    /// so a change to either alone silently opens the gate. This pins them
    /// together.
    ///
    /// The second assertion is the positive control: without it the first
    /// would still pass if `task_enters_in_progress` were broken to always
    /// return `false`, and the test would be guarding nothing.
    #[test]
    fn the_column_a_chat_created_card_defaults_to_does_not_dispatch() {
        use crate::ports::tasks::{COLUMN_IN_PROGRESS, COLUMN_TODO};

        // `create_task` (src/server/ops/tasks.rs) defaults an omitted `column`
        // to this one.
        assert!(
            !task_enters_in_progress(None, COLUMN_TODO),
            "a chat-created card must not spend an agent turn on arrival — the \
             human drag into in_progress is the approval gate"
        );
        assert!(
            task_enters_in_progress(None, COLUMN_IN_PROGRESS),
            "positive control: the trigger this test relies on is still live"
        );
    }

    /// Issue #242: the attempt row exists **before** the cycle is spawned, in
    /// [`RunStatus::Pending`], carrying the assignee it was dispatched to and a
    /// 1-based ordinal that climbs per re-dispatch. This is the whole point of
    /// minting at the choke point rather than inside the cycle — a host that
    /// dies in the gap leaves a visible orphan instead of nothing.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_dispatch_opens_a_pending_attempt_before_the_cycle_spawns() {
        use crate::ports::TaskRecord;
        use crate::ports::runs::{RunFilter, RunStatus};
        use crate::ports::tasks::COLUMN_IN_PROGRESS;

        let home = tempfile::Builder::new()
            .prefix("opencompany-run-open-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
        )
        .expect("manifest");
        let id = crate::ports::types::CompanyId::new("acme");
        let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .expect("runtime");

        let card = TaskRecord {
            id: "t-1".to_string(),
            title: TaskTitle::authored("Ship it"),
            note: None,
            column: COLUMN_IN_PROGRESS.to_string(),
            priority: "medium".to_string(),
            assignee: "ceo".to_string(),
            updated_at_millis: 0,
            origin: None,
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            bounced: None,
        };

        let first = runtime.open_run(&card).await.expect("an attempt is minted");
        let runs = runtime
            .runs()
            .list_runs(&id, &RunFilter::for_task("t-1"))
            .await
            .expect("list");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, first);
        assert_eq!(
            runs[0].status,
            RunStatus::Pending,
            "the row is written before anything runs, so it starts Pending"
        );
        assert_eq!(runs[0].attempt, 1, "the first attempt at a card is 1");
        assert_eq!(runs[0].agent_id, "ceo");
        assert!(
            runs[0].trigger_event_seq.is_none(),
            "the driving event has not been appended yet"
        );
        assert!(runs[0].started_at_millis.is_none());

        // A re-dispatch is a NEW attempt, never a resurrection of the first.
        let second = runtime.open_run(&card).await.expect("a second attempt");
        assert_ne!(second, first);
        let runs = runtime
            .runs()
            .list_runs(&id, &RunFilter::for_task("t-1"))
            .await
            .expect("list");
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs.iter()
                .find(|r| r.id == second)
                .expect("second")
                .attempt,
            2
        );
    }

    /// Issue #290 against issue #242's write path: a card dragged into
    /// `in_progress` while this runtime is being replaced must not leave an
    /// attempt row claiming to be pending forever.
    ///
    /// The board write is deliberately *not* gated on the quiesce — only cycles
    /// are — so this window is reachable, and the refusal happens before
    /// `CycleRunner` starts the run, which puts it out of reach of the cycle's
    /// own terminality backstop. A rebuild also skips the boot reaper by design,
    /// so nothing else would ever clean the row up.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_dispatch_refused_by_a_quiescing_runtime_settles_its_attempt() {
        use std::sync::Arc;

        use crate::ports::TaskRecord;
        use crate::ports::runs::{RUNTIME_REPLACED_ERROR, RunStatus};
        use crate::ports::tasks::COLUMN_IN_PROGRESS;

        let home = tempfile::Builder::new()
            .prefix("opencompany-run-quiesce-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
        )
        .expect("manifest");
        let id = crate::ports::types::CompanyId::new("acme");
        let runtime = Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(id.clone())
                .build()
                .await
                .expect("runtime"),
        );

        let card = TaskRecord {
            id: "t-1".to_string(),
            title: TaskTitle::authored("Ship it"),
            note: None,
            column: COLUMN_IN_PROGRESS.to_string(),
            priority: "medium".to_string(),
            assignee: "ceo".to_string(),
            updated_at_millis: 0,
            origin: None,
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            bounced: None,
        };

        // Positive control: on a live runtime the cycle runs, so the row is
        // settled by the backstop inside it and never reaches the path below.
        let live = runtime.open_run(&card).await.expect("an attempt");
        Arc::clone(&runtime)
            .run_dispatch_cycle(card.id.clone(), Some(live.clone()))
            .await;
        let settled = runtime
            .runs()
            .get_run(&id, &live)
            .await
            .expect("read")
            .expect("row");
        assert!(
            settled.status.is_terminal(),
            "the ordinary dispatch path still settles its own row"
        );
        assert_ne!(
            settled.error.as_deref(),
            Some(RUNTIME_REPLACED_ERROR),
            "the live path must not be settled by the quiesce handler"
        );

        // The window this test exists for.
        let stranded = runtime.open_run(&card).await.expect("an attempt");
        runtime.quiesce().await;
        Arc::clone(&runtime)
            .run_dispatch_cycle(card.id.clone(), Some(stranded.clone()))
            .await;

        let abandoned = runtime
            .runs()
            .get_run(&id, &stranded)
            .await
            .expect("read")
            .expect("row");
        assert_eq!(
            abandoned.status,
            RunStatus::Failed,
            "an attempt whose cycle was refused must not stay Pending"
        );
        assert_eq!(
            abandoned.error.as_deref(),
            Some(RUNTIME_REPLACED_ERROR),
            "and it must say the runtime was swapped, not that the host died"
        );
        assert!(
            abandoned.started_at_millis.is_none(),
            "it never started, so it has no start time"
        );
        assert!(abandoned.finished_at_millis.is_some());
    }

    /// Issue #1852 Part 1 — the discard bug and its fix, proven directly on
    /// `run_dispatch_cycle` rather than on any one `Brain`'s output shape.
    ///
    /// `RelayBrain` answers a `TaskDispatched` event with exactly the shape
    /// `relay_reply` (`harness::built_in::lifecycle`) produces: a bubble whose
    /// `reply_to` names the origin thread and whose `task_id` names the card
    /// — without standing up a real harness or LLM. Before this fix,
    /// `run_dispatch_cycle` discarded the `CycleReport` carrying it (`let
    /// Err(err) = self.run_cycle(...).await else { return; }`), which is the
    /// generic bug underneath #1852, independent of which `Brain` produced
    /// the relay: reverting `run_dispatch_cycle` to that shape reproduces the
    /// failure this test now guards — zero `AgentReply` events land in the
    /// origin thread, because nothing ever journals the discarded report.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_dispatched_cards_relay_is_journaled_into_its_origin_thread() {
        use std::sync::Arc;

        use crate::ports::Brain;
        use crate::ports::TaskRecord;
        use crate::ports::brain::CycleHost;
        use crate::ports::tasks::COLUMN_IN_PROGRESS;
        use crate::ports::types::{
            CycleRequest, CycleResult, OutboundMessage, ReplyTo, TokenUsage,
        };

        /// Answers a `TaskDispatched { task_id: "t-1" }` with a
        /// `relay_reply`-shaped bubble; silent on everything else, mirroring
        /// `EchoBrain`'s silence on `TaskDispatched`.
        struct RelayBrain;

        #[async_trait::async_trait]
        impl Brain for RelayBrain {
            async fn run_cycle(
                &self,
                req: CycleRequest,
                _host: &dyn CycleHost,
            ) -> crate::Result<CycleResult> {
                let mut channel_responses = Vec::new();
                for event in &req.events {
                    if let CompanyEvent::TaskDispatched { task_id, .. } = event
                        && task_id == "t-1"
                    {
                        channel_responses.push(OutboundMessage {
                            message_id: None,
                            task_id: Some("t-1".to_string()),
                            outputs: Vec::new(),
                            channel: "ceo".to_string(),
                            agent: None,
                            text: "\"Ship it\" is ready for review (ceo ran it).".to_string(),
                            mentions: Vec::new(),
                            reply_to: Some(ReplyTo {
                                chat_id: "strategy".to_string(),
                            }),
                            steps: Vec::new(),
                        });
                    }
                }
                Ok(CycleResult {
                    channel_responses,
                    new_traces: Vec::new(),
                    ledger_deltas: Vec::new(),
                    token_usage: TokenUsage::default(),
                })
            }
        }

        let home_dir = tempfile::Builder::new()
            .prefix("opencompany-relay-journal-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
        )
        .expect("manifest");
        let id = crate::ports::types::CompanyId::new("acme");
        let runtime = Arc::new(
            crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
                .with_id(id.clone())
                .with_brain(Arc::new(RelayBrain))
                .build()
                .await
                .expect("runtime"),
        );

        let card = TaskRecord {
            id: "t-1".to_string(),
            title: TaskTitle::authored("Ship it"),
            note: None,
            column: COLUMN_IN_PROGRESS.to_string(),
            priority: "medium".to_string(),
            assignee: "ceo".to_string(),
            updated_at_millis: 0,
            // The field the whole bug turns on: without an origin thread,
            // `relay_reply` is never called at all (a board-created card).
            origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            bounced: None,
        };

        let run_id = runtime.open_run(&card).await;
        Arc::clone(&runtime)
            .run_dispatch_cycle(card.id.clone(), run_id)
            .await;

        let events = runtime
            .events
            .read_from(&id, crate::ports::types::EventSeq::new(0), usize::MAX)
            .await
            .expect("read journal");
        let relays: Vec<_> = events
            .iter()
            .filter_map(|stored| match &stored.event {
                CompanyEvent::AgentReply { chat_id, .. } if chat_id == "strategy" => {
                    Some(&stored.event)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            relays.len(),
            1,
            "exactly one relay must land in the origin thread, found {relays:?}"
        );
        let CompanyEvent::AgentReply {
            agent_id, task_id, ..
        } = relays[0]
        else {
            unreachable!()
        };
        assert_eq!(
            agent_id, "ceo",
            "the orchestrator answers for its own roster (issue #885 fallback)"
        );
        assert_eq!(
            task_id, &None,
            "the settle already has its own card link — `DeskTaskCompleted`'s \
             \"finished → …\" pill (issue #377) — so this bubble must not carry \
             its own \"Card opened\" chip alongside it"
        );
        assert!(
            crate::server::chat_history::owns("strategy", "Strategy", relays[0]),
            "the origin desk's own history read must pick this reply up"
        );
    }

    /// A dispatched card whose origin is a teammate's **private DM** relays as
    /// that teammate, so the orchestrator never authors a second voice in a
    /// one-to-one thread — while a shared desk keeps the orchestrator's voice.
    ///
    /// The relay is produced the way `HarnessBrain::run_task` produces it:
    /// through [`relay_speaker`](crate::harness::built_in::brain::relay_speaker)
    /// + `relay_reply`, so a regression that let the orchestrator reclaim the DM
    /// voice would flip the journaled author and fail this test.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_private_dm_relay_is_authored_by_the_dm_agent_not_the_orchestrator() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::harness::built_in::brain::relay_speaker;
        use crate::harness::built_in::lifecycle::relay_reply;
        use crate::ports::Brain;
        use crate::ports::TaskRecord;
        use crate::ports::brain::CycleHost;
        use crate::ports::tasks::COLUMN_IN_REVIEW;
        use crate::ports::types::{
            CycleRequest, CycleResult, EventSeq, OutboundMessage, TokenUsage,
        };

        /// Replays a pre-built relay for each `TaskDispatched` it recognises.
        struct RelayBrain {
            replies: HashMap<String, OutboundMessage>,
        }

        #[async_trait::async_trait]
        impl Brain for RelayBrain {
            async fn run_cycle(
                &self,
                req: CycleRequest,
                _host: &dyn CycleHost,
            ) -> crate::Result<CycleResult> {
                let mut channel_responses = Vec::new();
                for event in &req.events {
                    if let CompanyEvent::TaskDispatched { task_id, .. } = event
                        && let Some(reply) = self.replies.get(task_id)
                    {
                        channel_responses.push(reply.clone());
                    }
                }
                Ok(CycleResult {
                    channel_responses,
                    new_traces: Vec::new(),
                    ledger_deltas: Vec::new(),
                    token_usage: TokenUsage::default(),
                })
            }
        }

        let manifest_toml = "[company]\nname = \"Acme\"\n\
             [policy]\nmode = \"full\"\n\
             [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
             [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\
             [[group_chat]]\nid = \"strategy\"\nname = \"Strategy\"\nmembers = [\"ceo\"]\n";
        let manifest: crate::company::CompanyManifest =
            toml::from_str(manifest_toml).expect("manifest");

        // A record standing for the same roster, to compute the relay authorship
        // exactly as `HarnessBrain` would. Journaling never reads it; it only
        // drives `relay_speaker`.
        let record = crate::ports::types::CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: crate::ports::types::CompanyId::new("acme"),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        };

        let orchestrator = "ceo";
        let card = |id: &str, origin: &str| TaskRecord {
            id: id.to_string(),
            title: TaskTitle::authored("Ship it"),
            note: None,
            column: COLUMN_IN_REVIEW.to_string(),
            priority: "medium".to_string(),
            assignee: origin.to_string(),
            updated_at_millis: 0,
            origin: crate::ports::TaskOrigin::new(Some(origin.to_string()), None),
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            bounced: None,
        };
        let dm_card = card("t-dm", "writer");
        let desk_card = card("t-desk", "strategy");

        // Built the way `run_task` builds them: the speaker is the origin DM's
        // own agent, or the orchestrator for a shared surface.
        let relay = |c: &TaskRecord| {
            let origin = c.origin_chat_id().map(str::to_string).expect("origin");
            let speaker = relay_speaker(&record, &origin, orchestrator);
            relay_reply(c, orchestrator, &speaker, origin, &[])
        };
        let replies = HashMap::from([
            ("t-dm".to_string(), relay(&dm_card)),
            ("t-desk".to_string(), relay(&desk_card)),
        ]);

        let home_dir = tempfile::Builder::new()
            .prefix("opencompany-private-dm-relay-")
            .tempdir()
            .expect("tempdir");
        let id = crate::ports::types::CompanyId::new("acme");
        let runtime = Arc::new(
            crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
                .with_id(id.clone())
                .with_brain(Arc::new(RelayBrain { replies }))
                .build()
                .await
                .expect("runtime"),
        );

        for c in [&dm_card, &desk_card] {
            let run_id = runtime.open_run(c).await;
            Arc::clone(&runtime)
                .run_dispatch_cycle(c.id.clone(), run_id)
                .await;
        }

        let events = runtime
            .events
            .read_from(&id, EventSeq::new(0), usize::MAX)
            .await
            .expect("read journal");
        let author_in = |thread: &str| {
            events.iter().find_map(|stored| match &stored.event {
                CompanyEvent::AgentReply {
                    chat_id, agent_id, ..
                } if chat_id == thread => Some(agent_id.clone()),
                _ => None,
            })
        };

        assert_eq!(
            author_in("writer").as_deref(),
            Some("writer"),
            "a relay into the writer's private DM must be authored by the writer"
        );
        assert_eq!(
            author_in("strategy").as_deref(),
            Some("ceo"),
            "a relay into a shared desk keeps the orchestrator's voice"
        );
    }

    /// Issue #1852: the gate that stops a dispatch relay from being posted
    /// twice.
    ///
    /// A response the ordinary chat-turn cycle already journals through
    /// `journal_chat_replies` (`server::operator`) never carries `reply_to` —
    /// [`relay_reply`](crate::harness::built_in::lifecycle::relay_reply) is
    /// the only producer that sets it — so gating on that field structurally
    /// cannot re-journal a bubble the inline work-card path already wrote.
    /// The same absence covers a board-created card (no `origin_chat_id`):
    /// `run_task`/`refuse_dispatch` return no relay for one at all, which is
    /// this exact "no `reply_to`" shape.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn journal_dispatch_replies_only_touches_relay_shaped_responses() {
        use crate::CycleReport;
        use crate::ports::types::{EventSeq, OutboundMessage, ReplyTo};

        let (rt, _home_dir) = runtime_with_events().await;

        let report = CycleReport {
            responses: vec![
                // An ordinary chat-turn bubble: no `reply_to`, exactly what
                // `journal_chat_replies` already owns. Must not be touched
                // here, or the inline work-card path would double-post.
                OutboundMessage {
                    message_id: None,
                    task_id: None,
                    outputs: Vec::new(),
                    channel: "operator".to_string(),
                    agent: Some("ceo".to_string()),
                    text: "already handled elsewhere".to_string(),
                    mentions: Vec::new(),
                    reply_to: None,
                    steps: Vec::new(),
                },
                // A `reply_to` naming an empty chat id — not degenerate:
                // `origin_chat_id` preserves `Some("")` for a card spawned
                // from General, and `chat_history::same_conversation` treats
                // "" as an alias for General, so this must still journal.
                OutboundMessage {
                    message_id: None,
                    task_id: Some("t-2".to_string()),
                    outputs: Vec::new(),
                    channel: "ceo".to_string(),
                    agent: None,
                    text: "General-chat relay".to_string(),
                    mentions: Vec::new(),
                    reply_to: Some(ReplyTo {
                        chat_id: String::new(),
                    }),
                    steps: Vec::new(),
                },
                // The one shape `relay_reply` actually produces.
                OutboundMessage {
                    message_id: None,
                    task_id: Some("t-1".to_string()),
                    outputs: Vec::new(),
                    channel: "ceo".to_string(),
                    agent: None,
                    text: "\"Ship it\" is ready for review.".to_string(),
                    mentions: Vec::new(),
                    reply_to: Some(ReplyTo {
                        chat_id: "strategy".to_string(),
                    }),
                    steps: Vec::new(),
                },
            ],
            ..Default::default()
        };

        rt.journal_dispatch_replies(&report).await;

        let events = rt
            .events
            .read_from(&rt.id, EventSeq::new(0), usize::MAX)
            .await
            .expect("read journal");
        let relays: Vec<_> = events
            .iter()
            .filter_map(|stored| match &stored.event {
                CompanyEvent::AgentReply { .. } => Some(&stored.event),
                _ => None,
            })
            .collect();
        assert_eq!(
            relays.len(),
            2,
            "both reply_to-shaped responses must be journaled — an empty \
             chat_id is General, not absent — found {relays:?}"
        );
        let CompanyEvent::AgentReply {
            chat_id, task_id, ..
        } = relays
            .iter()
            .find(|event| matches!(event, CompanyEvent::AgentReply { chat_id, .. } if chat_id == "strategy"))
            .expect("the named-thread relay must be present")
        else {
            unreachable!()
        };
        assert_eq!(chat_id, "strategy");
        // Not `Some("t-1")`, even though the response itself carries it:
        // `journal_task_outcome` already marked "t-1" settled with its own
        // `DeskTaskCompleted` card link into this same thread, so this bubble
        // must not add a second one. See the drop site's own comment.
        assert_eq!(task_id, &None);

        let CompanyEvent::AgentReply { chat_id, .. } = relays
            .iter()
            .find(|event| matches!(event, CompanyEvent::AgentReply { chat_id, .. } if chat_id.is_empty()))
            .expect("the empty-chat_id General relay must be present")
        else {
            unreachable!()
        };
        assert_eq!(
            chat_id, "",
            "General's own empty chat_id must be preserved verbatim"
        );
    }

    /// Issue #1890: a relayed card reports back into the conversation that
    /// raised it, not beside it.
    ///
    /// Found by hand-testing, not by a suite. A delegated request produced the
    /// orchestrator's answer inside its thread and then the delegate's reply
    /// and the relay bubble loose in the channel — three bubbles for one ask,
    /// two of them in the wrong place. Invisible while only hand-opened threads
    /// existed; obvious the moment every exchange is one.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_relayed_card_answers_in_the_thread_that_raised_it() {
        let (rt, _home_dir) = runtime_with_events().await;
        let id = rt.id().clone();

        // The root has to be *in* the journal, not merely named by the card.
        // `journal_dispatch_replies` now guards its parent through
        // `resolvable_parent` (coderabbit on #1982), which is what stops a
        // pruned root turning the delegate's answer into a reply the console
        // silently drops. A fixture that names a sequence nothing was ever
        // written at is the pruned case, so it has to write one.
        let root = rt
            .events()
            .append(
                &id,
                CompanyEvent::OperatorMessage {
                    mentions: Vec::new(),
                    text: "Draft the launch email".to_string(),
                    by: None,
                    chat: Some("general".to_string()),
                    parent: None,
                    deliverable: None,
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("the root is journaled");

        let mut card = crate::ports::tasks::TaskRecord {
            id: "t-relay".to_string(),
            title: TaskTitle::authored("Draft the launch email"),
            note: None,
            column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
            priority: "medium".to_string(),
            assignee: "writer".to_string(),
            updated_at_millis: 0,
            origin: crate::ports::TaskOrigin::new(Some("general".to_string()), Some(root)),
            parent_task_id: None,
            output: None,
            plan: None,
            planning_attempts: Vec::new(),
            deliverable: crate::ports::tasks::TaskDeliverable::Once,
            workflow_proposal: None,
            origin_run_id: None,
            origin_workflow_id: None,
            origin_message_seq: None,
            bounced: None,
        };
        rt.tasks().upsert(&id, &card).await.unwrap();

        let relay = |task: Option<&str>| crate::ports::types::OutboundMessage {
            message_id: None,
            task_id: task.map(str::to_string),
            outputs: Vec::new(),
            channel: "ceo".to_string(),
            agent: None,
            text: "the delegate finished it".to_string(),
            mentions: Vec::new(),
            reply_to: Some(crate::ports::types::ReplyTo {
                chat_id: "general".to_string(),
            }),
            steps: Vec::new(),
        };

        let report = crate::runtime::types::CycleReport {
            responses: vec![relay(Some("t-relay"))],
            ..Default::default()
        };
        rt.journal_dispatch_replies(&report).await;

        let logged = rt
            .events()
            .read_from(&id, crate::ports::types::EventSeq::new(0), usize::MAX)
            .await
            .unwrap();
        let threaded = logged.iter().rev().find_map(|e| match &e.event {
            CompanyEvent::AgentReply { parent, .. } => Some(*parent),
            _ => None,
        });
        assert_eq!(
            threaded,
            Some(Some(root)),
            "the relay joins the thread the card recorded at raise time"
        );

        // And a card raised at channel level still relays flat — `None` is the
        // channel-level conversation, not a gap.
        card.id = "t-flat".to_string();
        card.origin =
            crate::ports::TaskOrigin::new(card.origin_chat_id().map(str::to_string), None);
        rt.tasks().upsert(&id, &card).await.unwrap();
        let report = crate::runtime::types::CycleReport {
            responses: vec![relay(Some("t-flat"))],
            ..Default::default()
        };
        rt.journal_dispatch_replies(&report).await;

        let logged = rt
            .events()
            .read_from(&id, crate::ports::types::EventSeq::new(0), usize::MAX)
            .await
            .unwrap();
        let last = logged.iter().rev().find_map(|e| match &e.event {
            CompanyEvent::AgentReply { parent, .. } => Some(*parent),
            _ => None,
        });
        assert_eq!(
            last,
            Some(None),
            "a channel-level card relays into the channel"
        );
    }

    /// Issue #435: the guard that decides whether a remembered thread root is
    /// still usable, and the direction it fails in.
    ///
    /// Every arm here degrades to `None`, which means "answer in the channel".
    /// That is the issue's stated requirement and it is not merely tidy: the
    /// console drops a reply whose parent it cannot resolve in the channel
    /// rather than rendering it flat, so a stale root would make the
    /// continuation invisible — strictly worse than the bug being fixed, since
    /// today's answer at least reaches the channel.

    /// A runtime whose one agent is allowed to refer to the `design` desk, so a
    /// forward reaches the width bound instead of stopping at authorization.
    #[cfg(all(feature = "openhuman", feature = "hivemind"))]
    async fn runtime_that_may_refer() -> (crate::company::runtime::CompanyRuntime, tempfile::TempDir)
    {
        let home_dir = tempfile::Builder::new()
            .prefix("opencompany-refer-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::types::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"
            delegates_to = ["design"]

            [[agent]]
            id = "designer"
            role = "Designer"

            [[group_chat]]
            id = "design"
            name = "Design"
            members = ["designer"]

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let rt = crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
            .build()
            .await
            .expect("runtime");
        (rt, home_dir)
    }

    /// **The width bound: how many desks one pass may ask.**
    ///
    /// `max_hops` bounds how DEEP a chain runs and says nothing about how WIDE
    /// it is — the library leaves that to the host, because only a host knows
    /// what a question costs it. Here it is a full model turn on another desk.
    ///
    /// Pinned with a cap of 1 so the second forward is the one that trips it,
    /// and with distinct triggers so the idempotency marker cannot be what
    /// refuses it.
    #[cfg(all(feature = "openhuman", feature = "hivemind"))]
    #[tokio::test]
    async fn a_second_crossing_question_is_refused_once_the_width_is_spent() {
        use tinyhivemind::dispatch::{EnqueueOutcome, EnqueueRefusal};
        use tinyhivemind::referral::ReferralQueue;

        let (rt, _home) = runtime_that_may_refer().await;
        let rt = Arc::new(rt);
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        let queue =
            crate::runtime::hivemind::JournalReferralQueue::new(rt.clone(), gate, 1, 4, None);

        let forward = |trigger: u64| tinyhivemind::referral::Referral {
            key: tinyhivemind::dispatch::DispatchKey {
                trigger_sequence: trigger,
            },
            kind: tinyhivemind::referral::ReferralKind::Forward,
            source_id: "ceo".to_string(),
            target_id: "designer".to_string(),
            content: "who owns the login screen?".to_string(),
            from: tinyhivemind::dispatch::DispatchConversation {
                desk_id: "engineering".to_string(),
                thread_root: None,
            },
            to: tinyhivemind::dispatch::DispatchConversation {
                desk_id: "design".to_string(),
                thread_root: None,
            },
            origin: None,
            child_hop: 1,
        };

        assert_eq!(
            queue.enqueue_once(forward(101)).await.expect("decides"),
            EnqueueOutcome::Enqueued,
            "the first question is within the cap"
        );
        assert_eq!(
            queue.enqueue_once(forward(202)).await.expect("decides"),
            EnqueueOutcome::Refused {
                reason: EnqueueRefusal::FeatureDisabled
            },
            "a different trigger, so this is the WIDTH bound refusing it, not the marker"
        );
    }

    /// **Fail-closed, and leave nothing behind.** The fixture roster declares
    /// no `delegates_to`, so an agent may not cause a turn on another desk —
    /// referral is off until an operator opts somebody in.
    ///
    /// And a refusal writes no marker, so it cannot be mistaken for a completed
    /// enqueue on the next attempt: the second call is refused for the same
    /// reason as the first, rather than coming back `Already`.
    #[cfg(all(feature = "openhuman", feature = "hivemind"))]
    #[tokio::test]
    async fn an_unauthorized_forward_is_refused_and_leaves_no_marker() {
        use tinyhivemind::dispatch::EnqueueOutcome;
        use tinyhivemind::referral::ReferralQueue;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        // A cap high enough not to be what this test measures: the second
        // enqueue must be refused as `Already`, by the marker, not by width.
        let queue =
            crate::runtime::hivemind::JournalReferralQueue::new(rt.clone(), gate, 8, 4, None);

        let referral = tinyhivemind::referral::Referral {
            key: tinyhivemind::dispatch::DispatchKey {
                trigger_sequence: 77,
            },
            kind: tinyhivemind::referral::ReferralKind::Forward,
            source_id: "ceo".to_string(),
            target_id: "ceo".to_string(),
            content: "who owns the login screen?".to_string(),
            from: tinyhivemind::dispatch::DispatchConversation {
                desk_id: "engineering".to_string(),
                thread_root: None,
            },
            to: tinyhivemind::dispatch::DispatchConversation {
                desk_id: "design".to_string(),
                thread_root: None,
            },
            origin: None,
            child_hop: 1,
        };

        // The fixture roster declares no `delegates_to`, so the FIRST call is
        // refused on authorization — which is itself the fail-closed default
        // worth pinning: referral is off until an operator opts an agent in.
        let first = queue.enqueue_once(referral.clone()).await.expect("decides");
        assert_eq!(
            first,
            EnqueueOutcome::Refused {
                reason: tinyhivemind::dispatch::EnqueueRefusal::Unauthorized
            },
            "an agent with no `delegates_to` may not cause a turn on another desk"
        );

        // And a refusal leaves NO marker, so it is not mistaken for a
        // completed enqueue on the next attempt.
        let replay = queue.enqueue_once(referral).await.expect("decides");
        assert_eq!(
            replay,
            EnqueueOutcome::Refused {
                reason: tinyhivemind::dispatch::EnqueueRefusal::Unauthorized
            },
            "a refusal must not write the marker — otherwise a retry after the \
             operator grants permission would report `Already` and drop the work"
        );
    }

    /// A runtime with a live event log, for the thread-root tests. Returns the
    /// tempdir too: dropping it deletes the log the runtime is reading.
    async fn runtime_with_events() -> (crate::company::runtime::CompanyRuntime, tempfile::TempDir) {
        let home_dir = tempfile::Builder::new()
            .prefix("opencompany-parent-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::types::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let rt = crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
            .build()
            .await
            .expect("runtime");
        (rt, home_dir)
    }

    /// A helper effect and a manifest for the extend tests.
    fn extend_test_effect() -> crate::ports::types::Effect {
        crate::ports::types::Effect {
            kind: "payment.send".into(),
            group: crate::ports::types::EffectGroup::Spend,
            amount_usd: Some(1_200.0),
            established_thread: false,
            first_time_counterparty: false,
            payload: serde_json::json!({ "to": "vendor@example.test" }),
            agent: Some("ceo".into()),
            run_id: None,
        }
    }

    /// A **gated tool call** parked by a workflow agent node, in the shape
    /// production actually creates (Codex on the B-012 PR, second round).
    ///
    /// This is the path that leaves an attempt reading `WaitingApproval`:
    /// `caps::park_gated_calls` journals `ApprovalPolicy::effect_for`'s effect —
    /// `kind` is the **tool name**, `run_id` is `None` — under the node's
    /// `workflow-node:{run}:{node}` cycle, and the node then settles its attempt
    /// `WaitingApproval`. The earlier fixture here paired a `gate_effect` with a
    /// hand-made attempt row, a combination no parking path produces, and so
    /// reported a fix that could not fire in production as working.
    ///
    /// Returns the attempt row's id — the row the expiry has to find.
    async fn park_gated_node_call(
        rt: &std::sync::Arc<crate::company::runtime::CompanyRuntime>,
        approval: &crate::ports::types::ApprovalId,
        lineage: &str,
        node: &str,
        at_millis: u64,
        arm_continuation: bool,
    ) -> String {
        use crate::ports::runs::RunStatus;
        use crate::ports::types::{Effect, EffectGroup, EventSeq};
        use crate::runtime::journal::{ApprovalConversation, TaskLink};

        let attempt = crate::ports::generate_id();
        rt.runs()
            .create_run(
                rt.id(),
                crate::ports::NewRun::for_workflow_node(attempt.clone(), lineage, node, "ceo"),
            )
            .await
            .unwrap();
        rt.runs()
            .begin_run(rt.id(), &attempt, EventSeq::new(1))
            .await
            .unwrap();
        rt.runs()
            .finish_run(
                rt.id(),
                &attempt,
                crate::ports::runs::RunOutcome::new(RunStatus::WaitingApproval),
            )
            .await
            .unwrap();

        // `effect_for`'s shape, field for field: the tool's own name as the
        // kind, the agent stamped, and **no** `run_id`.
        let effect = Effect {
            kind: "workspace.write".into(),
            group: EffectGroup::Other,
            amount_usd: None,
            established_thread: false,
            first_time_counterparty: false,
            payload: serde_json::json!({ "path": "README.md" }),
            agent: Some("ceo".into()),
            run_id: None,
        };
        let node_turn = crate::runtime::workflow_resume::workflow_node_turn_key(lineage, node);
        if arm_continuation {
            rt.continuations.arm(&node_turn);
        }
        rt.approval_gate
            .rehydrate(approval.clone(), effect.clone(), at_millis);
        rt.journal
            .record_parked(
                approval,
                &effect,
                at_millis,
                TaskLink::Unlinked,
                ApprovalConversation::default(),
                Some(node_turn),
            )
            .await
            .unwrap();
        attempt
    }

    /// Seeds one parked approval into BOTH the live gate and the durable journal
    /// under a fixed id at `at_millis`, exactly as a real park leaves them — the
    /// gate answers "is this live?" for extend/sweep, the journal projects the
    /// deadline and replays on boot.
    async fn seed_parked(
        rt: &crate::company::runtime::CompanyRuntime,
        id: &str,
        at_millis: u64,
    ) -> crate::ports::types::ApprovalId {
        use crate::ports::types::ApprovalId;
        use crate::runtime::journal::{ApprovalConversation, TaskLink};
        let approval = ApprovalId::new(id);
        let effect = extend_test_effect();
        rt.approval_gate
            .rehydrate(approval.clone(), effect.clone(), at_millis);
        rt.journal
            .record_parked(
                &approval,
                &effect,
                at_millis,
                TaskLink::Unlinked,
                ApprovalConversation::default(),
                None,
            )
            .await
            .unwrap();
        approval
    }

    /// **B-012.** A workflow run parked on a gate stops claiming it is waiting
    /// once that approval expires.
    ///
    /// A parked run is recorded `WaitingApproval` — settled, nothing executing,
    /// and the status is the row's account of *why* it stopped. Expiry retired
    /// the approval and dropped it from the pending set, but nothing revisited
    /// the row, so it went on naming an approval no sweep would see again: the
    /// Observatory showed a run awaiting a decision while the approvals list
    /// showed nothing to decide, and neither screen was wrong about its own
    /// data.
    #[tokio::test]
    async fn an_expired_approval_settles_the_workflow_run_that_was_waiting_on_it() {
        use crate::ports::runs::RunStatus;
        use crate::ports::types::ApprovalId;
        use std::sync::Arc;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);

        // A workflow node parked on a gate: an attempt row settled
        // `WaitingApproval` and linked to the lineage, plus the gate itself
        // carrying that lineage. Parked at epoch 0 — past any TTL.
        let approval = ApprovalId::new("appr-b012");
        let attempt = park_gated_node_call(&rt, &approval, "wr-b012", "solve", 0, false).await;

        let expired = rt.sweep_expired_approvals().await.unwrap();
        assert!(
            expired.contains(&approval),
            "the sweep must find the epoch-0 park: {expired:?}"
        );

        let row = rt
            .runs()
            .get_run(rt.id(), &attempt)
            .await
            .unwrap()
            .expect("the attempt row survives the sweep");
        assert_eq!(
            row.status,
            RunStatus::Cancelled,
            "a default-denied gate leaves the attempt cancelled, not still waiting"
        );
    }

    /// **The narrowing** the settle above is scoped by. One expiry must not
    /// cancel a *sibling* node still waiting on a live decision.
    ///
    /// A graph can park two nodes on two gates, and `RunFilter` can only ask
    /// for the lineage — so "every `WaitingApproval` attempt of this run" is
    /// the obvious query and the wrong one. The node is read off the gate's own
    /// payload (`gate_node_id`) to close that gap.
    #[tokio::test]
    async fn an_expiry_leaves_a_sibling_node_still_waiting_on_a_live_gate() {
        use crate::ports::runs::RunStatus;
        use crate::ports::types::ApprovalId;
        use std::sync::Arc;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);

        // Same lineage, two nodes: one parked at epoch 0 (past any TTL), one
        // parked now (nowhere near it).
        let expiring = ApprovalId::new("appr-expiring");
        let attempt_expiring =
            park_gated_node_call(&rt, &expiring, "wr-two-gates", "solve", 0, false).await;
        let live = ApprovalId::new("appr-live");
        let attempt_live = park_gated_node_call(
            &rt,
            &live,
            "wr-two-gates",
            "review",
            crate::ports::now_millis(),
            false,
        )
        .await;

        let expired = rt.sweep_expired_approvals().await.unwrap();
        assert!(
            expired.contains(&expiring) && !expired.contains(&live),
            "only the epoch-0 park expires: {expired:?}"
        );

        let settled = rt
            .runs()
            .get_run(rt.id(), &attempt_expiring)
            .await
            .unwrap()
            .expect("the expired node's attempt survives");
        assert_eq!(settled.status, RunStatus::Cancelled);

        let sibling = rt
            .runs()
            .get_run(rt.id(), &attempt_live)
            .await
            .unwrap()
            .expect("the sibling's attempt survives");
        assert_eq!(
            sibling.status,
            RunStatus::WaitingApproval,
            "the sibling node is still waiting on a decision nobody has made"
        );
    }

    /// **An expiry settles its attempt even when it releases a continuation**
    /// (Codex on the B-012 PR, third round).
    ///
    /// The tempting reading is that a released node is "still going" and must
    /// not be settled. It is not: a continuation runs as a **new** attempt —
    /// `RunAttempts` is rebuilt per run and `caps` mints every attempt under
    /// `generate_id()` — so nothing ever writes this row again. Skipping it left
    /// exactly the stale `WaitingApproval` this issue exists to remove, and the
    /// earlier version of this test could not see that, because it asserted only
    /// that the cancellation *error* was absent and never looked at the status.
    ///
    /// The scenario is the one that makes a node's batch non-empty, since an
    /// expiry alone never does (`ContinuationQueue::decide` banks no event for
    /// one): two gated calls on one node, one answered and one expired.
    #[tokio::test]
    async fn an_expiry_settles_its_attempt_even_when_it_releases_a_continuation() {
        use crate::ports::runs::RunStatus;
        use crate::ports::types::{Actor, ActorKind, ApprovalId, Verdict};
        use std::sync::Arc;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);

        let approval = ApprovalId::new("appr-released");
        let attempt = park_gated_node_call(&rt, &approval, "wr-released", "solve", 0, true).await;

        // The node's *second* gated call, answered by the operator before the
        // first expires. Its banked event is what makes the released batch
        // non-empty, and so what makes this node continue at all.
        let node_turn =
            crate::runtime::workflow_resume::workflow_node_turn_key("wr-released", "solve");
        rt.continuations.arm(&node_turn);
        assert!(
            rt.continuations
                .decide(
                    &node_turn,
                    Some(CompanyEvent::ApprovalResolved {
                        approval_id: ApprovalId::new("appr-answered"),
                        verdict: Verdict::Approve,
                        by: Actor {
                            kind: ActorKind::Operator,
                            id: "operator".into(),
                        },
                    }),
                )
                .is_none(),
            "the node is still blocked on the gate that has not expired yet"
        );

        let expired = rt.sweep_expired_approvals().await.unwrap();
        assert!(
            expired.contains(&approval),
            "the sweep must find the epoch-0 park: {expired:?}"
        );

        let row = rt
            .runs()
            .get_run(rt.id(), &attempt)
            .await
            .unwrap()
            .expect("the attempt row survives the sweep");
        assert_eq!(
            row.status,
            RunStatus::Cancelled,
            "the released continuation runs as a NEW attempt, so this row is nobody else's \
             to settle and must not be left reading `WaitingApproval`"
        );
    }

    /// **A settle that only changes status must not erase what the attempt
    /// spent** (Codex on the B-012 PR, third round).
    ///
    /// `RunStore::finish_run` assigns `usage` and `step_count` from the outcome
    /// rather than merging, and `RunOutcome::new` zeroes both — so cancelling an
    /// expired attempt from a bare outcome silently wipes the tokens and cost it
    /// really did spend, on a row the billing surfaces read.
    #[tokio::test]
    async fn settling_an_expired_attempt_keeps_the_usage_it_recorded() {
        use crate::ports::runs::{RunOutcome, RunStatus};
        use crate::ports::types::{ApprovalId, TokenUsage};
        use std::sync::Arc;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);

        let approval = ApprovalId::new("appr-usage");
        let attempt = park_gated_node_call(&rt, &approval, "wr-usage", "solve", 0, false).await;

        // What the attempt spent before it parked. Re-settled onto the parked
        // row exactly as a real turn's trace fold would leave it.
        let usage = TokenUsage {
            input: 1_200,
            output: 340,
            cached_input: 0,
            cost_usd: 0.042,
        };
        rt.runs()
            .finish_run(
                rt.id(),
                &attempt,
                RunOutcome::new(RunStatus::WaitingApproval)
                    .with_usage(usage)
                    .with_step_count(7),
            )
            .await
            .unwrap();

        rt.sweep_expired_approvals().await.unwrap();

        let row = rt
            .runs()
            .get_run(rt.id(), &attempt)
            .await
            .unwrap()
            .expect("the attempt row survives the sweep");
        assert_eq!(row.status, RunStatus::Cancelled);
        assert_eq!(
            row.usage, usage,
            "the expiry changed the status; it must not have erased the spend"
        );
        assert_eq!(row.step_count, 7, "nor the trace it recorded");
    }

    /// Issue #1865 (Codex review on PR #1883): a late resolve that discovers
    /// an approval already past its deadline owes the SAME "expired
    /// unanswered" notification the sweep loop files when it discovers the
    /// identical deadline first.
    ///
    /// `notify_approval_expired` used to be invoked from nowhere but
    /// `sweep_expired_approvals`, so `retire_if_expired` — the path a late
    /// `resolve_approval_spawned`/`resolve_approval_amended_spawned` takes
    /// when `settle_approval` answers `ResolveReceipt::Expired` — ran the
    /// whole four-step `retire_approval` transaction and never told anybody.
    /// The exact same expiry notified when the sweeper found it and stayed
    /// silent when an operator's late click found it instead.
    #[tokio::test]
    async fn a_late_resolve_that_discovers_an_expiry_files_the_same_notification_as_the_sweep() {
        use crate::ports::types::{Actor, ActorKind, Verdict};
        use crate::runtime::grants::GrantScope;
        use std::sync::Arc;

        let (rt, _home) = runtime_with_events().await;
        let rt = Arc::new(rt);
        // Parked at epoch 0 — unambiguously past any TTL, the same trick
        // `expired_approval_is_labelled_as_an_expiry_and_carries_its_wait`
        // (src/server/ops/write_test.rs) uses.
        let id = seed_parked(&rt, "appr-late", 0).await;

        let by = Actor {
            kind: ActorKind::Operator,
            id: "owner".into(),
        };
        let (receipt, follow_up) = rt
            .resolve_approval_spawned(&id, Verdict::Approve, by, GrantScope::Once)
            .await
            .unwrap();
        assert!(
            receipt.expired(),
            "an epoch-0 park must read as expired, not approved: {receipt:?}"
        );
        super::join_follow_up(follow_up).await.unwrap();

        let notifications = rt.notifications().list(rt.id(), "owner").await.unwrap();
        assert!(
            notifications
                .iter()
                .any(|n| n.notification.kind == "approval_expired"
                    && n.notification.subject.id == id.as_ref()),
            "a late resolve that discovers an expiry must file the same \
             approval_expired notification the sweep files, got {notifications:?}"
        );
    }

    /// Issue #971 (the projection this issue builds on): a card's deadline is the
    /// deadline anchor plus the gate's TTL, resolved once at the single
    /// projection point.
    #[tokio::test]
    async fn pending_approvals_projects_deadline_as_anchor_plus_ttl() {
        let (rt, _home) = runtime_with_events().await;
        seed_parked(&rt, "appr-deadline", 5_000).await;
        let ttl = rt.approval_gate.ttl_millis();
        assert_eq!(
            rt.pending_approvals()[0].expires_at_millis,
            Some(5_000 + ttl),
            "a fresh card's deadline runs from when it was parked"
        );
    }

    /// **The load-bearing extend test (issue #1805).** Extending moves the live
    /// deadline, and — the half that a redeploy silently reverted before this —
    /// the move survives a rebuild of the runtime from the same journal, because
    /// the extension is replayed and the gate is rehydrated from the moved anchor.
    #[tokio::test]
    async fn extend_approval_moves_deadline_and_survives_replay() {
        use crate::ports::types::{Actor, ActorKind};

        let home_dir = tempfile::Builder::new()
            .prefix("opencompany-extend-replay-")
            .tempdir()
            .expect("tempdir");
        let manifest: crate::company::types::CompanyManifest =
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"supervised\"\n")
                .expect("manifest");

        // First boot: park an old approval, confirm its original deadline, extend.
        let rt1 =
            crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest.clone())
                .build()
                .await
                .expect("runtime");
        let id = seed_parked(&rt1, "appr-replay", 1_000).await;
        let ttl = rt1.approval_gate.ttl_millis();
        assert_eq!(
            rt1.pending_approvals()[0].expires_at_millis,
            Some(1_000 + ttl),
            "the fresh deadline runs from the park instant"
        );

        let new_deadline = rt1
            .extend_approval(
                &id,
                Actor {
                    kind: ActorKind::User,
                    id: "operator".into(),
                },
            )
            .await
            .expect("extend");
        assert!(
            new_deadline > 1_000 + ttl,
            "the live deadline moved out: {new_deadline} vs {}",
            1_000 + ttl
        );
        assert_eq!(
            rt1.pending_approvals()[0].expires_at_millis,
            Some(new_deadline),
            "the live projection reflects the extension immediately"
        );
        drop(rt1);

        // Second boot from the SAME journal — the redeploy the extension has to
        // survive. Without the replayed `ApprovalExtended` the deadline would
        // revert to `1_000 + ttl`.
        let rt2 = crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
            .build()
            .await
            .expect("runtime");
        let replayed = rt2.pending_approvals();
        assert_eq!(
            replayed.len(),
            1,
            "the approval is still parked after a redeploy"
        );
        assert_eq!(
            replayed[0].expires_at_millis,
            Some(new_deadline),
            "the extended deadline survived the rebuild instead of reverting to the park window"
        );
        // The rehydrated gate enforces the extended window too: a sweep one tick
        // before the new deadline leaves it parked.
        assert!(
            rt2.approval_gate.sweep_expired(new_deadline - 1).is_empty(),
            "the rehydrated gate must enforce the extension, not the original park"
        );
    }

    /// A [`JournalStore`](crate::ports::journal::JournalStore) that refuses
    /// every `ApprovalExtended` line and passes everything else through to an
    /// in-memory backend.
    struct RefusingExtendStore {
        inner: crate::ports::journal::MemoryJournalStore,
    }

    #[async_trait::async_trait]
    impl crate::ports::journal::JournalStore for RefusingExtendStore {
        async fn append_journal(
            &self,
            id: &crate::ports::types::CompanyId,
            line: &str,
            durability: crate::ports::journal::Durability,
        ) -> crate::Result<()> {
            if line.contains("ApprovalExtended") {
                return Err(crate::error::OpenCompanyError::Store(
                    "RefusingExtendStore: the volume is full".to_string(),
                ));
            }
            self.inner.append_journal(id, line, durability).await
        }

        async fn read_journal(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<Vec<String>> {
            self.inner.read_journal(id).await
        }

        async fn journal_imported(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<bool> {
            self.inner.journal_imported(id).await
        }

        async fn complete_import(
            &self,
            id: &crate::ports::types::CompanyId,
            lines: Vec<String>,
        ) -> crate::Result<()> {
            self.inner.complete_import(id, lines).await
        }
    }

    /// `extend_approval` moves the gate's live deadline **before**
    /// it journals the extension. When the journal append then fails, the
    /// caller sees the error, but the live view already reflects the later
    /// deadline — and nothing durable backs that, so a restart from the same
    /// journal comes back believing the approval was never extended at all.
    /// This pins that sequence exactly, as the real, current consequence: a
    /// caller told the extend failed still sees the live queue disagree with
    /// it until the next restart quietly settles the disagreement in the
    /// caller's favor.
    #[tokio::test]
    async fn a_failed_extend_append_leaves_a_live_extension_that_reverts_on_restart() {
        use crate::ports::types::{Actor, ActorKind};

        let manifest: crate::company::types::CompanyManifest =
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"supervised\"\n")
                .expect("manifest");
        let store = std::sync::Arc::new(RefusingExtendStore {
            inner: crate::ports::journal::MemoryJournalStore::default(),
        });
        let home_dir = tempfile::tempdir().expect("tempdir");

        let rt1 =
            crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest.clone())
                .with_journal_store(store.clone())
                .build()
                .await
                .expect("runtime");
        let id = seed_parked(&rt1, "appr-extend-fail", 1_000).await;
        let ttl = rt1.approval_gate.ttl_millis();
        let original_deadline = 1_000 + ttl;

        let extend = rt1
            .extend_approval(
                &id,
                Actor {
                    kind: ActorKind::User,
                    id: "operator".into(),
                },
            )
            .await;
        assert!(extend.is_err(), "the forced append failure must surface");
        assert!(
            rt1.pending_approvals()[0].expires_at_millis.unwrap() > original_deadline,
            "the live gate already moved the deadline even though nothing durable recorded it"
        );
        drop(rt1);

        let rt2 = crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
            .with_journal_store(store)
            .build()
            .await
            .expect("runtime");
        let replayed = rt2.pending_approvals();
        assert_eq!(replayed.len(), 1, "the approval is still parked");
        assert_eq!(
            replayed[0].expires_at_millis,
            Some(original_deadline),
            "the extension a caller was told failed must not silently revert on restart"
        );
    }

    /// A [`JournalStore`](crate::ports::journal::JournalStore) that refuses
    /// every `ApprovalExpired` line and passes everything else through.
    struct RefusingExpiredStore {
        inner: crate::ports::journal::MemoryJournalStore,
    }

    #[async_trait::async_trait]
    impl crate::ports::journal::JournalStore for RefusingExpiredStore {
        async fn append_journal(
            &self,
            id: &crate::ports::types::CompanyId,
            line: &str,
            durability: crate::ports::journal::Durability,
        ) -> crate::Result<()> {
            if line.contains("ApprovalExpired") {
                return Err(crate::error::OpenCompanyError::Store(
                    "RefusingExpiredStore: the volume is full".to_string(),
                ));
            }
            self.inner.append_journal(id, line, durability).await
        }

        async fn read_journal(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<Vec<String>> {
            self.inner.read_journal(id).await
        }

        async fn journal_imported(
            &self,
            id: &crate::ports::types::CompanyId,
        ) -> crate::Result<bool> {
            self.inner.journal_imported(id).await
        }

        async fn complete_import(
            &self,
            id: &crate::ports::types::CompanyId,
            lines: Vec<String>,
        ) -> crate::Result<()> {
            self.inner.complete_import(id, lines).await
        }
    }

    /// `sweep_expired_capped` removes every id in the batch from the
    /// live `parked` map up front, stashing each one's effect in
    /// `expired_effects` for [`CompanyRuntime::retire_approval`] to collect.
    /// `sweep_expired_approvals` then walks that batch and returns on the
    /// **first** `retire_approval` failure (`?`), so a durable-write failure
    /// partway through strands every id after it: already gone from `parked`,
    /// still sitting in `expired_effects`, and never revisited because the
    /// next sweep's scan is over `parked`, which no longer names them.
    #[tokio::test]
    async fn a_failed_retirement_mid_batch_strands_the_rest_of_the_batch() {
        use crate::ports::types::{Actor, ActorKind, Verdict};

        let manifest: crate::company::types::CompanyManifest =
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"supervised\"\n")
                .expect("manifest");
        let store = std::sync::Arc::new(RefusingExpiredStore {
            inner: crate::ports::journal::MemoryJournalStore::default(),
        });
        let home_dir = tempfile::tempdir().expect("tempdir");
        let rt = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home_dir.path().to_path_buf(), manifest)
                .with_journal_store(store)
                .build()
                .await
                .expect("runtime"),
        );

        let ttl = rt.approval_gate.ttl_millis();
        let long_expired = crate::ports::now_millis().saturating_sub(ttl + 60_000);
        // Alphabetical order matches park-time order here, so the cap sorts
        // "appr-a" first — the one whose retirement is attempted (and fails)
        // — and "appr-b" is the untouched survivor stranded behind it.
        seed_parked(&rt, "appr-a", long_expired).await;
        seed_parked(&rt, "appr-b", long_expired).await;
        assert_eq!(
            rt.pending_approvals().len(),
            2,
            "both are parked and expired"
        );

        let swept = rt.sweep_expired_approvals().await;
        assert!(
            swept.is_err(),
            "the forced ApprovalExpired failure must surface"
        );

        // `record_expired` moves its in-memory `parked` entry out **before**
        // journaling the expiry — the same optimistic-then-persist order
        // The extend case pins the same shape — so "appr-a"'s failed attempt still drops
        // it from the journal's own pending view in-memory, with nothing
        // durable behind that removal. Only "appr-b", whose retirement was
        // never even attempted, is left on the console's pending list.
        let pending = rt.pending_approvals();
        assert_eq!(
            pending.len(),
            1,
            "only the untried survivor is left on the console's pending list: {pending:?}"
        );
        assert_eq!(
            pending[0].id,
            crate::ports::types::ApprovalId::new("appr-b")
        );

        // The gate's own live `parked` map already dropped both — that is
        // what `sweep_expired_capped` did before the failing retirement ever
        // ran — so a decision on the survivor is not a decision on anything:
        // it comes back as a safe no-op, never as the operator's verdict.
        let (receipt, _handle) = rt
            .resolve_approval_spawned(
                &crate::ports::types::ApprovalId::new("appr-b"),
                Verdict::Approve,
                Actor {
                    kind: ActorKind::User,
                    id: "operator".into(),
                },
                crate::runtime::grants::GrantScope::Once,
            )
            .await
            .expect("a losing resolve is a receipt, not an error");
        assert!(
            matches!(
                receipt,
                crate::runtime::cycle::ResolveReceipt::AlreadyResolved
            ),
            "the survivor is gone from the gate's live map, so even the operator's own \
             decision on it silently no-ops instead of settling it: {receipt:?}"
        );

        // A later sweep never even reaches the still-refusing store: its scan
        // is over `parked`, which no longer names either id, so it succeeds
        // trivially with nothing to report — the stranded survivor is retired
        // by nothing and never seen again, while the console goes on listing it.
        let second_sweep = rt
            .sweep_expired_approvals()
            .await
            .expect("nothing left in `parked` to retire");
        assert!(
            second_sweep.is_empty(),
            "the stranded survivor is never retried by a later sweep: {second_sweep:?}"
        );
    }

    #[tokio::test]
    async fn an_unresolvable_thread_root_degrades_to_the_channel() {
        use crate::ports::types::{Actor, ActorKind, CompanyEvent, EventSeq};

        let (rt, _home_dir) = runtime_with_events().await;

        // A real root in `desk-finance`, and a second message elsewhere.
        let root = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::OperatorMessage {
                    mentions: Vec::new(),
                    text: "pay the invoice".into(),
                    by: None,
                    chat: Some("desk-finance".into()),
                    parent: None,
                    deliverable: None,
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("append");
        let elsewhere = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::OperatorMessage {
                    mentions: Vec::new(),
                    text: "unrelated".into(),
                    by: None,
                    chat: Some("desk-ops".into()),
                    parent: None,
                    deliverable: None,
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("append");

        // The good case: a root that exists, in the channel being answered.
        assert_eq!(
            rt.resolvable_parent(Some(root), "desk-finance").await,
            Some(root),
        );

        // No root recorded at all — the overwhelmingly common case, and the
        // pre-#435 behaviour.
        assert_eq!(rt.resolvable_parent(None, "desk-finance").await, None);

        // A root that resolves but lives in another channel. Renderable
        // nowhere, and proof the recorded pair was already inconsistent.
        assert_eq!(
            rt.resolvable_parent(Some(elsewhere), "desk-finance").await,
            None,
            "a root in another channel must not follow the answer across",
        );

        // A root that is simply GONE, with a live message after it.
        //
        // This is the case the exact-sequence check exists for, and it has to
        // be built deliberately. `read_from` returns events with sequence >=
        // the one asked for, so a vanished root comes back as its *successor*.
        // Asking past the end of the log proves nothing — that read is empty
        // and every implementation returns `None`. A genuine gap is what
        // separates "found it" from "found the next one", and the only thing
        // that makes gaps is pruning, which the events module documents as
        // leaving them by design.
        //
        // So: a prunable frame, then a real message in the channel, then a
        // pass that removes the first. Without the sequence check the message
        // answers for the hole underneath it — and it is in the right channel,
        // so the channel check waves it through.
        let doomed = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::WorkspaceChanged {
                    node_id: "n-1".into(),
                    change: "updated".into(),
                },
            )
            .await
            .expect("append");
        let after_the_hole = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::OperatorMessage {
                    mentions: Vec::new(),
                    text: "and another thing".into(),
                    by: None,
                    chat: Some("desk-finance".into()),
                    parent: None,
                    deliverable: None,
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("append");
        rt.events
            .prune(
                &rt.id,
                &crate::ports::events::RetentionPolicy {
                    max_entries_per_kind: Some(0),
                    ..Default::default()
                },
            )
            .await
            .expect("prune");
        // The hole is real, and the next event is a same-channel message.
        let successor = rt
            .events
            .read_from(&rt.id, doomed, 1)
            .await
            .expect("read")
            .into_iter()
            .next()
            .expect("the message after the hole answers the read");
        assert_eq!(
            successor.seq, after_the_hole,
            "the pruned sequence must genuinely be absent, answered by its successor",
        );
        assert_eq!(
            rt.resolvable_parent(Some(doomed), "desk-finance").await,
            None,
            "a vanished root must not be answered by the message that follows it",
        );

        // And past the end of the log, where the read is simply empty.
        let beyond = EventSeq::new(after_the_hole.value() + 500);
        assert_eq!(
            rt.resolvable_parent(Some(beyond), "desk-finance").await,
            None
        );

        // A sequence that resolves to something that is not a chat message at
        // all cannot root a thread either.
        let not_a_message = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::LifecycleChanged {
                    from: "idle".into(),
                    to: "running".into(),
                    by: Actor {
                        kind: ActorKind::Operator,
                        id: "owner".into(),
                    },
                },
            )
            .await
            .expect("append");
        assert_eq!(
            rt.resolvable_parent(Some(not_a_message), "desk-finance")
                .await,
            None,
        );
    }

    /// The General desk answers to four spellings, and a thread rooted in any
    /// of them keeps its parent (issue #435).
    ///
    /// This is the case the fix was *most* likely to be handed and originally
    /// dropped: the chat route journals an unaddressed message as
    /// `chat: None` while the console renders it under `General` and replies to
    /// it there, so the comparison arrived as `None` vs `"General"`. A raw
    /// string compare rejected it, the parent was discarded, and the
    /// continuation resumed in the channel — #435's own symptom surviving
    /// inside #435's fix, on the default path rather than an exotic one.
    #[tokio::test]
    async fn a_root_in_any_spelling_of_the_general_desk_still_resolves() {
        use crate::ports::types::CompanyEvent;

        let (rt, _home_dir) = runtime_with_events().await;

        // Three roots, one desk: the unaddressed post, the console's own
        // thread id, and the desk named outright.
        let mut roots = Vec::new();
        for chat in [None, Some("main"), Some("General")] {
            roots.push(
                rt.events
                    .append(
                        &rt.id,
                        CompanyEvent::OperatorMessage {
                            mentions: Vec::new(),
                            text: "ship it".into(),
                            by: None,
                            chat: chat.map(str::to_string),
                            parent: None,
                            deliverable: None,
                            attachments: Vec::new(),
                        },
                    )
                    .await
                    .expect("append"),
            );
        }

        // Every root resolves against every spelling of the channel it is
        // answered into — including the pair that used to fail.
        for root in &roots {
            for channel in ["General", "main", "general"] {
                assert_eq!(
                    rt.resolvable_parent(Some(*root), channel).await,
                    Some(*root),
                    "root {root} must resolve when answered into `{channel}`",
                );
            }
        }

        // …and the folding stops there. A real desk is still compared
        // verbatim, so this widening cannot pull an unrelated thread in.
        let elsewhere = rt
            .events
            .append(
                &rt.id,
                CompanyEvent::OperatorMessage {
                    mentions: Vec::new(),
                    text: "unrelated".into(),
                    by: None,
                    chat: Some("desk-ops".into()),
                    parent: None,
                    deliverable: None,
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("append");
        assert_eq!(
            rt.resolvable_parent(Some(elsewhere), "General").await,
            None,
            "a named desk is not the General desk",
        );
        assert_eq!(
            rt.resolvable_parent(Some(roots[0]), "desk-ops").await,
            None,
            "and the General desk is not a named one",
        );
    }

    /// Issue #966: the failed-continuation report is authored by the runtime.
    ///
    /// This site appends the `AgentReply` itself, so it never sees
    /// `OutboundMessage::agent` or its `channel` fallback — it has to name the
    /// author, and it used to name `OPERATOR_CHANNEL`. That made a correct
    /// system row byte-identical on disk to a reply the pre-#885 defect had
    /// damaged, which is the finding recorded on #966.
    #[test]
    fn a_failed_continuation_report_is_authored_by_the_runtime_not_the_operator() {
        let event = continuation_failure_notice("desk-general".to_string(), None);
        let CompanyEvent::AgentReply {
            agent_id, chat_id, ..
        } = event
        else {
            panic!("the notice must stay an AgentReply — the console renders it from that arm");
        };
        assert_eq!(agent_id, crate::ports::SYSTEM_AUTHOR);
        assert_ne!(
            agent_id,
            crate::runtime::channel::OPERATOR_CHANNEL,
            "a notice must not store the author a destination-overwrite produces"
        );
        assert_eq!(
            chat_id, "desk-general",
            "it still lands in the thread it answers"
        );
    }

    /// Issue #1861 (found by Codex on #1905): a gate park that lands and then
    /// fails to journal must not leave the approval decidable.
    ///
    /// # The window
    ///
    /// `park_blocker` parks on the gate first and journals second. A `?` on the
    /// journal write reported the park as failed — so `settle_blocked` returned
    /// the card to To-do — while the gate still held a live, decidable entry
    /// against it. The operator is then shown a question for a card nobody
    /// paused, which is the exact inconsistency `unpark_blocker` exists to
    /// prevent on the other side of this pair.
    ///
    /// `record_parked` also populates the projection *before* its append, so
    /// the same failure left a pending approval that no journal line would ever
    /// replay: visible until the process exits, gone after a boot.
    ///
    /// Both are asserted here, because clearing one without the other just
    /// moves the disagreement.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_blocker_that_cannot_be_journaled_leaves_no_decidable_approval() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let journal = std::sync::Arc::new(RefusingJournalStore::default());
        let runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .with_id(crate::ports::types::CompanyId::new("acme"))
            .with_journal_store(journal.clone())
            .build()
            .await
            .expect("runtime");

        // The volume goes away *after* boot, so this is an ordinary runtime.
        journal.arm();

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };

        let parked = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await;
        assert!(
            parked.is_err(),
            "an unjournaled park is reported as a failed park, so the caller returns the card"
        );

        assert!(
            runtime.approval_gate.parked_ids().is_empty(),
            "the gate entry must be rolled back — otherwise the operator can decide a blocker \
             for a card that was handed straight back to To-do"
        );
        assert!(
            runtime.pending_approvals().is_empty(),
            "and the projection row `record_parked` inserted before its append must go with it"
        );
    }

    /// **P1 review finding on PR #2038.** `claim_and_settle_blocker` claims
    /// the blocker's resolution slot before banking it durably. If the bank
    /// then fails (a transient journal write error), the claim used to stay
    /// taken with nothing behind it — so a retry lost the race against its
    /// own earlier attempt and answered `AlreadyResolved` forever, and the
    /// blocker became unanswerable for the rest of the process's life. The
    /// claim must be released on that failure so a retry can actually settle.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_transient_journal_failure_releases_the_blocker_claim_for_retry() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let journal = std::sync::Arc::new(RefusingJournalStore::default());
        let runtime = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks before the volume goes away");

        // The volume goes away *after* the park, so the claim/bank/settle
        // path is what fails, not the park itself.
        journal.arm();
        let failed = runtime
            .apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                crate::ports::blockers::BlockerVerdict::Retry,
                "",
                None,
            )
            .await;
        assert!(
            failed.is_err(),
            "the armed journal store must fail the bank and surface the error: {failed:?}"
        );

        // The volume is back. If the earlier failure left the claim taken,
        // this retry loses the race against itself and reports
        // `AlreadyResolved` without ever settling — the bug this test is for.
        journal.disarm();
        let (receipt, follow_up) = runtime
            .apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                crate::ports::blockers::BlockerVerdict::Retry,
                "",
                None,
            )
            .await
            .expect("the retry must be accepted once the volume is back");
        crate::company::runtime::join_follow_up(follow_up)
            .await
            .expect("follow-up runs");

        assert!(
            matches!(receipt, crate::runtime::cycle::ResolveReceipt::Settled(_)),
            "a transient journal failure must not permanently strand the claim — the \
             retry must actually settle the blocker, not report AlreadyResolved forever: \
             {receipt:?}"
        );
    }

    /// **Major review finding (CodeRabbit) on PR #2038.** The claim-release
    /// fix above only covers `record_blocker_resolution`'s own failure.
    /// `settle_approval` banks its own journal record right after
    /// (`record_resolved`), and a volume that dies between the two fails
    /// there instead — after the blocker's resolution is already durable,
    /// but before the approval itself settles. That path returned via `?`
    /// with the claim still taken.
    ///
    /// This asserts the claim itself (`peek_blocker_resolution`) rather than
    /// a full successful retry, because `record_resolved` (like
    /// `resolve_outcome` on the gate) removes the approval from
    /// `journal.pending()` *before* its own append can fail — so a same-
    /// process retry hits `claim_and_settle_blocker`'s independent
    /// `still_parked` guard and reports `AlreadyResolved` regardless of
    /// whether the claim was released. Releasing it here is still owed: an
    /// orphaned entry in `grants.blocker_resolutions` for an id no live
    /// resume will ever consume is exactly the state
    /// `take_blocker_resolution` exists to prevent.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_journal_failure_inside_settle_also_releases_the_blocker_claim() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let journal = std::sync::Arc::new(RefusingJournalStore::default());
        let runtime = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks before the volume goes away");

        // The volume dies after exactly one more append lands: that append
        // is `record_blocker_resolution`, so the claim's own bank succeeds
        // and the very next journal write --- `settle_approval`'s
        // `record_resolved` --- is the one that fails.
        journal.arm();
        journal.allow_next(1);
        let failed = runtime
            .apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                crate::ports::blockers::BlockerVerdict::Retry,
                "",
                None,
            )
            .await;
        assert!(
            failed.is_err(),
            "the armed journal store must fail settle_approval's own record and surface \
             the error: {failed:?}"
        );

        assert!(
            runtime.grants.peek_blocker_resolution(&id).is_none(),
            "a settle_approval failure after the claim was banked must release it too, \
             not just a record_blocker_resolution failure — otherwise \
             grants.blocker_resolutions keeps an orphaned entry for an id no live \
             resume will ever consume"
        );
    }
    /// **P1 review finding (Codex) on PR #2038.** Releasing the *live* claim
    /// when `settle_approval` fails is only half the compensation: the durable
    /// `BlockerResolved` record is already banked and survives. A boot
    /// rehydrates it onto the grant set, the approval itself is still parked —
    /// `record_resolved` never landed, so replay never saw it resolve — and
    /// nothing drives the pair. Every later answer then loses
    /// `claim_blocker_resolution` to the rehydrated entry and returns
    /// `AlreadyResolved` without settling or resuming, so the blocker is
    /// permanently unanswerable and stays that way across further restarts.
    ///
    /// `claim_and_settle_blocker`'s own doc already promised the opposite —
    /// "a crash between the two still replays as still armed and re-resumes" —
    /// and no code made that true. This is that promise, asserted: after the
    /// restart the blocker must actually leave the pending set rather than sit
    /// banked forever.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_banked_blocker_whose_settle_failed_is_driven_on_the_next_boot() {
        let home = tempfile::tempdir().expect("home");
        let manifest = || {
            toml::from_str::<crate::company::CompanyManifest>(
                r#"
                [company]
                name = "Acme"

                [[agent]]
                id = "ceo"
                role = "Chief"

                [policy]
                mode = "supervised"
                "#,
            )
            .expect("manifest")
        };
        let journal = std::sync::Arc::new(RefusingJournalStore::default());
        let booted = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = booted
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks");

        // The volume dies after exactly one more append: `record_blocker_resolution`
        // lands, so the answer is durable, and `settle_approval`'s own
        // `record_resolved` is the write that fails.
        journal.arm();
        journal.allow_next(1);
        assert!(
            booted
                .apply_blocker_reply_spawned(
                    std::slice::from_ref(&id),
                    &id,
                    crate::ports::blockers::BlockerVerdict::Retry,
                    "",
                    None,
                )
                .await
                .is_err(),
            "the armed journal store must fail settle_approval's own record"
        );
        // The volume comes back, as it would have by the time anyone restarts.
        journal.disarm();
        drop(booted);

        // The next boot: same home, same journal, replayed from scratch.
        let rebooted = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );
        assert!(
            rebooted.grants.peek_blocker_resolution(&id).is_some(),
            "the boot must rehydrate the banked answer — without that there is \
             nothing for this test to drive"
        );
        assert!(
            rebooted.journal.pending().iter().any(|p| p.id == id),
            "and the approval must still be parked, since record_resolved never landed"
        );

        rebooted.recover().await.expect("replay");

        // The resume runs on a spawned task, so give it room to land.
        let mut settled = false;
        for _ in 0..200 {
            if !rebooted.journal.pending().iter().any(|p| p.id == id) {
                settled = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            settled,
            "a banked-but-unsettled blocker answer must be driven on the next boot; it is \
             still parked with its resolution rehydrated, so claim_blocker_resolution will \
             refuse every later answer and this blocker can never be resolved by anyone"
        );
    }
    /// A blocker parked far enough in the past to be past any TTL, seeded the
    /// way `seed_parked` does so the deadline is arbitrary rather than "now".
    #[cfg(feature = "openhuman")]
    async fn park_expired_blocker(
        runtime: &Arc<super::CompanyRuntime>,
        id: &str,
        payload: &crate::ports::blockers::BlockerPayload,
    ) -> crate::ports::types::ApprovalId {
        use crate::runtime::journal::{ApprovalConversation, TaskLink};
        let approval = crate::ports::types::ApprovalId::new(id);
        let effect = crate::ports::types::Effect {
            kind: payload.effect_kind(),
            group: crate::ports::types::EffectGroup::Other,
            amount_usd: None,
            established_thread: false,
            first_time_counterparty: false,
            payload: serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
            agent: None,
            run_id: None,
        };
        runtime
            .approval_gate
            .rehydrate(approval.clone(), effect.clone(), 0);
        runtime
            .journal
            .record_parked(
                &approval,
                &effect,
                0,
                TaskLink::Unlinked,
                ApprovalConversation::default(),
                None,
            )
            .await
            .expect("seed parked blocker");
        approval
    }

    #[cfg(feature = "openhuman")]
    fn stuck_payload() -> crate::ports::blockers::BlockerPayload {
        crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        }
    }

    #[cfg(feature = "openhuman")]
    async fn blocker_runtime() -> (Arc<super::CompanyRuntime>, tempfile::TempDir) {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let runtime = Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .build()
                .await
                .expect("runtime"),
        );
        (runtime, home)
    }

    /// **Major review finding (CodeRabbit) on PR #2038.** The console fallback
    /// claims the slot before it settles, and `settle_approval` can answer
    /// `Expired` or `AlreadyResolved` rather than failing outright. Neither
    /// receipt gets a resume — `spawn_follow_up` returns early for both — so
    /// the claim it armed is left in `grants.blocker_resolutions` with nothing
    /// that will ever consume it. The four-way path already compensates this;
    /// the fallback did not.
    ///
    /// An epoch-0 park is unambiguously past any TTL, which makes the
    /// `Expired` arm reachable without racing a deadline.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_console_expiry_releases_the_blocker_claim_it_armed() {
        let (runtime, _home) = blocker_runtime().await;
        let payload = stuck_payload();
        let id = park_expired_blocker(&runtime, "appr-expired-blocker", &payload).await;

        let (receipt, _follow_up) = runtime
            .resolve_approval_spawned(
                &id,
                crate::ports::types::Verdict::Approve,
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: "owner".to_string(),
                },
                crate::runtime::grants::GrantScope::Once,
            )
            .await
            .expect("a late console click resolves as an expiry, not an error");
        assert!(
            receipt.expired(),
            "an epoch-0 park must read as expired: {receipt:?}"
        );

        assert!(
            runtime.grants.peek_blocker_resolution(&id).is_none(),
            "the console fallback armed a claim and then settled to a receipt with no \
             resume behind it; leaving the claim armed strands an entry no follow-up \
             will ever take, and blocks every later answer to the same id"
        );
    }

    /// The same finding's other half: a `settle_approval` that *fails* after the
    /// fallback has claimed and banked must release the live claim too, exactly
    /// as `settle_claimed_blocker` does for the four-way path.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_console_settle_failure_releases_the_blocker_claim_it_armed() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let journal = std::sync::Arc::new(RefusingJournalStore::default());
        let runtime = Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );
        let payload = stuck_payload();
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks");

        // One more append lands — the fallback's own `record_blocker_resolution`
        // — and `settle_approval`'s `record_resolved` is the write that fails.
        journal.arm();
        journal.allow_next(1);
        assert!(
            runtime
                .resolve_approval_spawned(
                    &id,
                    crate::ports::types::Verdict::Approve,
                    crate::ports::types::Actor {
                        kind: crate::ports::types::ActorKind::Operator,
                        id: "owner".to_string(),
                    },
                    crate::runtime::grants::GrantScope::Once,
                )
                .await
                .is_err(),
            "the armed journal store must fail settle_approval's own record"
        );

        assert!(
            runtime.grants.peek_blocker_resolution(&id).is_none(),
            "a settle failure after the console fallback banked its answer must release \
             the live claim, the same compensation claim_and_settle_blocker makes"
        );
    }
    /// **P1 review finding (Codex) on PR #2038.** The group lock and the
    /// atomic claim covered the four-way path only. The console's still-supported
    /// two-value fallback armed through `peek_blocker_resolution`, an awaited
    /// journal write, and an unconditional `arm_blocker_resolution` insert — so a
    /// legacy Approve/Deny could read the slot empty, suspend on its own journal
    /// write while a four-way request claimed the blocker, and then overwrite the
    /// winner's resolution on the way out. The approval event recorded one verdict
    /// while the resume executed another.
    ///
    /// Whoever wins `claim_blocker_resolution` owns the slot, so this asserts the
    /// two agree rather than pinning a particular winner: the interleaved request
    /// reports whether it took the slot, and the armed resolution must be that
    /// caller's either way.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn the_console_fallback_never_overwrites_a_blocker_claim_it_lost() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let journal = std::sync::Arc::new(RacingJournalStore::default());
        let runtime = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .with_journal_store(journal.clone())
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks");

        // The four-way request lands *inside* the console path's awaited journal
        // write — the exact window the peek-then-insert pair left open.
        let rival_won = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let grants = runtime.grants.clone();
            let id = id.clone();
            let rival_won = rival_won.clone();
            journal.interleave_next(move || {
                let rival = crate::ports::blockers::BlockerResolution {
                    verdict: crate::ports::blockers::BlockerVerdict::Skip,
                    answer: String::new(),
                    step: Some(crate::ports::blockers::BlockerStep::Task {
                        task_id: "t-1".to_string(),
                    }),
                };
                rival_won.store(
                    grants.claim_blocker_resolution(&id, rival),
                    std::sync::atomic::Ordering::SeqCst,
                );
            });
        }

        runtime
            .resolve_approval_spawned(
                &id,
                crate::ports::types::Verdict::Approve,
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: crate::runtime::channel::OPERATOR_CHANNEL.to_string(),
                },
                crate::runtime::grants::GrantScope::Once,
            )
            .await
            .expect("the console resolve lands");

        let armed = runtime
            .grants
            .peek_blocker_resolution(&id)
            .expect("a resolution stays armed for the resume to consume");
        let winner = rival_won.load(std::sync::atomic::Ordering::SeqCst);
        let expected = if winner {
            crate::ports::blockers::BlockerVerdict::Skip
        } else {
            crate::ports::blockers::BlockerVerdict::Retry
        };
        assert_eq!(
            armed.verdict, expected,
            "the caller that won claim_blocker_resolution must own the armed slot \
             (rival won the claim: {winner}); the console fallback overwrote a \
             resolution it did not claim, so the approval event and the resume \
             disagree about what the operator decided"
        );
    }

    /// A DM reply and a console verdict both resolve through
    /// `claim_blocker_resolution`, so they cannot both win — but until now
    /// nothing drove one of each at the same blocker and checked the loser's
    /// **own return value**, only the armed slot's content (see the test
    /// above). `resolve_approval_spawned` and `apply_blocker_reply_spawned`
    /// both hold `self.blocker_resolutions` for their claim-and-settle window,
    /// so true interleaving is impossible by construction; what remains
    /// untested is that the second caller in, whichever surface it is, is
    /// handed back `AlreadyResolved` rather than a receipt that reads like it
    /// was the one that settled the blocker.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_console_verdict_after_a_dm_reply_already_won_is_told_it_lost() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let runtime = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks");

        // The DM reply lands first and wins the claim.
        let (dm_receipt, dm_follow_up) = runtime
            .apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                crate::ports::blockers::BlockerVerdict::Retry,
                "",
                None,
            )
            .await
            .expect("the dm reply claims and settles");
        assert!(
            matches!(
                dm_receipt,
                crate::runtime::cycle::ResolveReceipt::Settled(_)
            ),
            "the dm reply must be the one that settles the blocker: {dm_receipt:?}"
        );
        drop(dm_follow_up);

        // The console verdict arrives on the same id after the claim is
        // already taken. It must not error, and it must not be told it won.
        let (console_receipt, _console_follow_up) = runtime
            .resolve_approval_spawned(
                &id,
                crate::ports::types::Verdict::Deny,
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: crate::runtime::channel::OPERATOR_CHANNEL.to_string(),
                },
                crate::runtime::grants::GrantScope::Once,
            )
            .await
            .expect("a losing resolve is a receipt, not an error");
        assert!(
            matches!(
                console_receipt,
                crate::runtime::cycle::ResolveReceipt::AlreadyResolved
            ),
            "a console verdict racing a dm reply it lost must be reported to its own \
             caller as already-resolved, not silently accepted as though it settled \
             the blocker: {console_receipt:?}"
        );
    }

    /// The mirror of the test above: the console verdict wins the claim, and a
    /// DM reply arriving after it on the same blocker must be told it lost
    /// through its own return value rather than being silently accepted.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_dm_reply_after_a_console_verdict_already_won_is_told_it_lost() {
        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            r#"
            [company]
            name = "Acme"

            [[agent]]
            id = "ceo"
            role = "Chief"

            [policy]
            mode = "supervised"
            "#,
        )
        .expect("manifest");
        let runtime = std::sync::Arc::new(
            crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                .with_id(crate::ports::types::CompanyId::new("acme"))
                .build()
                .await
                .expect("runtime"),
        );

        let payload = crate::ports::blockers::BlockerPayload {
            kind: crate::ports::blockers::BlockerKind::Infrastructure,
            source: crate::ports::blockers::BlockerSource::Provider,
            step: Some(crate::ports::blockers::BlockerStep::Task {
                task_id: "t-1".to_string(),
            }),
            reason: "the model `gpt-nonexistent` was rejected".to_string(),
            needed: "a model id this provider serves".to_string(),
            group_key: None,
        };
        let id = runtime
            .park_blocker(
                &payload,
                "t-1",
                crate::company::blocker_sender::BlockerSenderSignals::default(),
            )
            .await
            .expect("parks");

        let (console_receipt, _console_follow_up) = runtime
            .resolve_approval_spawned(
                &id,
                crate::ports::types::Verdict::Approve,
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: crate::runtime::channel::OPERATOR_CHANNEL.to_string(),
                },
                crate::runtime::grants::GrantScope::Once,
            )
            .await
            .expect("the console verdict claims and settles");
        assert!(
            matches!(
                console_receipt,
                crate::runtime::cycle::ResolveReceipt::Settled(_)
            ),
            "the console verdict must be the one that settles the blocker: {console_receipt:?}"
        );

        let (dm_receipt, dm_follow_up) = runtime
            .apply_blocker_reply_spawned(
                std::slice::from_ref(&id),
                &id,
                crate::ports::blockers::BlockerVerdict::Retry,
                "",
                None,
            )
            .await
            .expect("a losing dm reply is a receipt, not an error");
        assert!(
            matches!(
                dm_receipt,
                crate::runtime::cycle::ResolveReceipt::AlreadyResolved
            ),
            "a dm reply racing a console verdict it lost must be reported to its own \
             caller as already-resolved, not silently accepted as though it settled \
             the blocker: {dm_receipt:?}"
        );
        drop(dm_follow_up);
    }

    /// The thread-as-review-surface: a reply to a settled `in_review` dispatch
    /// card's settle pill or relay bubble routes as review feedback and re-runs
    /// the card; an Approve verdict finishes it.
    #[cfg(feature = "openhuman")]
    mod review {
        use crate::ports::TaskRecord;
        use crate::ports::tasks::{
            COLUMN_DONE, COLUMN_IN_PROGRESS, COLUMN_IN_REVIEW, TaskStore, TaskTitle,
        };
        use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};
        use std::sync::Arc;
        use tempfile::TempDir;

        type Runtime = crate::company::runtime::CompanyRuntime;

        async fn runtime() -> (Arc<Runtime>, TempDir) {
            runtime_with_tasks(None).await
        }

        /// A [`TaskStore`] whose `list` always fails, so a review lookup can be
        /// driven through the task-store-error arm rather than the "no such
        /// card" one.
        struct FailingTasks;

        #[async_trait::async_trait]
        impl TaskStore for FailingTasks {
            async fn list(&self, _company: &CompanyId) -> crate::Result<Vec<TaskRecord>> {
                Err(crate::error::OpenCompanyError::Harness(
                    "the board is unavailable".to_string(),
                ))
            }
            async fn upsert(&self, _company: &CompanyId, _task: &TaskRecord) -> crate::Result<()> {
                Ok(())
            }
            async fn update_if_column(
                &self,
                _company: &CompanyId,
                _task: &TaskRecord,
                _observed: &TaskRecord,
                _expected_column: &str,
            ) -> crate::Result<bool> {
                Ok(false)
            }
            async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
                Ok(false)
            }
        }

        async fn runtime_with_tasks(tasks: Option<Arc<dyn TaskStore>>) -> (Arc<Runtime>, TempDir) {
            runtime_with(tasks, None).await
        }

        /// An [`EventLog`](crate::ports::events::EventLog) decorator whose
        /// reads can be switched to fail after setup, so a test can seed real
        /// events through a working log and then drive the review-anchor
        /// lookup through the read-failure arm. `append`/`subscribe` always
        /// delegate to a real [`FsEventLog`](crate::store::fs::FsEventLog) so
        /// seeding never observes the failure and behaves exactly as
        /// production does.
        struct FailingReadsEventLog {
            inner: crate::store::fs::FsEventLog,
            fail_reads: std::sync::atomic::AtomicBool,
        }

        impl FailingReadsEventLog {
            fn new(inner: crate::store::fs::FsEventLog) -> Self {
                Self {
                    inner,
                    fail_reads: std::sync::atomic::AtomicBool::new(false),
                }
            }

            fn fail_reads_from_now_on(&self) {
                self.fail_reads
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        #[async_trait::async_trait]
        impl crate::ports::events::EventLog for FailingReadsEventLog {
            async fn append(&self, id: &CompanyId, event: CompanyEvent) -> crate::Result<EventSeq> {
                self.inner.append(id, event).await
            }

            async fn read_from(
                &self,
                id: &CompanyId,
                seq: EventSeq,
                limit: usize,
            ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
                if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(crate::error::OpenCompanyError::Harness(
                        "the event log is unavailable".to_string(),
                    ));
                }
                self.inner.read_from(id, seq, limit).await
            }

            async fn read_before(
                &self,
                id: &CompanyId,
                before: Option<EventSeq>,
                limit: usize,
            ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
                if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(crate::error::OpenCompanyError::Harness(
                        "the event log is unavailable".to_string(),
                    ));
                }
                self.inner.read_before(id, before, limit).await
            }

            fn subscribe(
                &self,
                id: &CompanyId,
            ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem>
            {
                self.inner.subscribe(id)
            }
        }

        async fn runtime_with(
            tasks: Option<Arc<dyn TaskStore>>,
            events: Option<Arc<dyn crate::ports::events::EventLog>>,
        ) -> (Arc<Runtime>, TempDir) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-review-")
                .tempdir()
                .expect("tempdir");
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[group_chat]]\nid = \"strategy\"\nname = \"Strategy\"\nmembers = [\"ceo\"]\n",
            )
            .expect("manifest");
            let mut builder =
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"));
            if let Some(tasks) = tasks {
                builder = builder.with_tasks(tasks);
            }
            if let Some(events) = events {
                builder = builder.with_events(events);
            }
            let runtime = Arc::new(builder.build().await.expect("runtime"));
            (runtime, home)
        }

        /// A company whose **durable record** declares an agent literally
        /// called `system` — the grandfathered shape
        /// [`CompanyRuntime::roster_declares_system_author`] exists for.
        ///
        /// Built by saving the roster over an ordinary company rather than by
        /// booting one from that manifest, because `RuntimeBuilder::build`
        /// validates with the reservation *enforced* and would refuse it. That
        /// is the point: the only way a live company carries this id is the
        /// reload path, which grandfathers it
        /// (`CompanyManifest::from_path_for_reload` passes
        /// `enforce_reserved_agent_ids: false`) and hands the runtime a record
        /// exactly like the one written here. The record is what
        /// `roster_declares_system_author` reads, so this reproduces the state
        /// under test without pretending the builder would mint it.
        async fn runtime_with_a_system_teammate() -> (Arc<Runtime>, TempDir) {
            let (runtime, home) = runtime().await;
            let mut record = runtime
                .store
                .load(runtime.id())
                .await
                .expect("load")
                .expect("record");
            record.manifest.agents[0].id = crate::ports::SYSTEM_AUTHOR.to_string();
            runtime.store.save(&record).await.expect("save");
            (runtime, home)
        }

        fn card(id: &str, origin: &str, column: &str) -> TaskRecord {
            TaskRecord {
                id: id.to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: column.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some(origin.to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            }
        }

        fn settle_pill(task_id: &str, origin: &str) -> CompanyEvent {
            CompanyEvent::DeskTaskCompleted {
                task_id: task_id.to_string(),
                desk: "ceo".to_string(),
                output: "done".to_string(),
                column: COLUMN_IN_REVIEW.to_string(),
                artifact_ids: Vec::new(),
                origin_chat_id: Some(origin.to_string()),
                origin_parent: None,
            }
        }

        fn relay_bubble(origin: &str) -> CompanyEvent {
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                chat_id: origin.to_string(),
                agent_id: "ceo".to_string(),
                text: "Here is the draft.".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
            }
        }

        /// The B-101 mention-ambiguity advisory
        /// ([`CompanyRuntime::post_mention_ambiguity_note`]) — an `AgentReply`
        /// with the identical `task_id: None` shape a relay bubble has, but
        /// authored by [`crate::ports::SYSTEM_AUTHOR`] rather than a roster
        /// agent. Used to seed the interleaving `is_relay_bubble_for` must not
        /// be fooled by (codex P2, PR #2052 fresh review round).
        fn advisory_bubble(origin: &str) -> CompanyEvent {
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                chat_id: origin.to_string(),
                agent_id: crate::ports::SYSTEM_AUTHOR.to_string(),
                text: "@sam matches two people here, so it pinged nobody.".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
            }
        }

        async fn seed(runtime: &Arc<Runtime>, c: &TaskRecord) {
            runtime.tasks().upsert(runtime.id(), c).await.expect("seed");
        }

        async fn append(runtime: &Arc<Runtime>, event: CompanyEvent) -> EventSeq {
            runtime
                .events
                .append(runtime.id(), event)
                .await
                .expect("append")
        }

        async fn stored(runtime: &Arc<Runtime>, id: &str) -> TaskRecord {
            runtime
                .tasks()
                .list(runtime.id())
                .await
                .expect("list")
                .into_iter()
                .find(|t| t.id == id)
                .expect("card survives")
        }

        #[tokio::test]
        async fn a_settle_pill_resolves_its_in_review_card() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let pill = append(&rt, settle_pill("t-1", "strategy")).await;

            let target = rt.review_feedback_target("strategy", pill).await.unwrap();
            assert_eq!(target.map(|c| c.id), Some("t-1".to_string()));
        }

        #[tokio::test]
        async fn a_relay_bubble_resolves_via_its_settle_pill() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            let bubble = append(&rt, relay_bubble("strategy")).await;

            let target = rt.review_feedback_target("strategy", bubble).await.unwrap();
            assert_eq!(
                target.map(|c| c.id),
                Some("t-1".to_string()),
                "the relay bubble carries no card link, so it anchors on the settle pill \
                 immediately before it"
            );
        }

        #[tokio::test]
        async fn the_resolver_declines_a_card_that_left_review() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_DONE)).await;
            let pill = append(&rt, settle_pill("t-1", "strategy")).await;

            assert!(
                rt.review_feedback_target("strategy", pill)
                    .await
                    .unwrap()
                    .is_none(),
                "a card already approved is not open for review"
            );
        }

        #[tokio::test]
        async fn the_resolver_declines_a_pill_from_another_conversation() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let pill = append(&rt, settle_pill("t-1", "strategy")).await;

            assert!(
                rt.review_feedback_target("marketing", pill)
                    .await
                    .unwrap()
                    .is_none(),
                "a reply in another desk must not review this desk's card"
            );
        }

        /// Codex #3903031192: a settle pill's relay bubble is the only reply
        /// target that anchors to its card. A later, unrelated `AgentReply` in
        /// the same desk — an ordinary chat turn — carries the identical
        /// `task_id: None` shape, so a reply to *that* message must not be
        /// mistaken for review feedback on the earlier card just because the
        /// pill is still the nearest one before it.
        #[tokio::test]
        async fn the_resolver_declines_a_later_ordinary_reply_that_is_not_the_relay() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            let true_relay = append(&rt, relay_bubble("strategy")).await;
            let later_ordinary_turn = append(&rt, relay_bubble("strategy")).await;

            assert_eq!(
                rt.review_feedback_target("strategy", true_relay)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "the pill's own relay bubble still anchors to its card"
            );
            assert!(
                rt.review_feedback_target("strategy", later_ordinary_turn)
                    .await
                    .unwrap()
                    .is_none(),
                "replying to a later ordinary turn must run a normal turn, not \
                 re-open the earlier card just because the pill is still the \
                 nearest one before it"
            );
        }

        /// PR #2052 fresh review round, codex P2: a dispatch that has appended
        /// its `DeskTaskCompleted` but has not yet run
        /// `journal_dispatch_replies` leaves a window in which another
        /// accepted chat's ambiguous `@name` can interleave a same-desk B-101
        /// advisory before the genuine relay lands. The advisory carries
        /// `task_id: None` exactly like a relay bubble, so it must not be
        /// mistaken for "the first `AgentReply` after the pill" — that would
        /// make the real relay's own reply fail `seq == parent` and silently
        /// run as an ordinary chat turn instead of review feedback.
        #[tokio::test]
        async fn the_resolver_skips_an_interleaved_ambiguity_advisory_to_find_the_real_relay() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            // The advisory lands between the pill and the relay: exactly the
            // interleaving window the finding describes.
            append(&rt, advisory_bubble("strategy")).await;
            let true_relay = append(&rt, relay_bubble("strategy")).await;

            assert_eq!(
                rt.review_feedback_target("strategy", true_relay)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "the real relay must still anchor to its card past an \
                 interleaved system advisory"
            );
        }

        /// The negative half: a reply to the advisory itself is not a relay
        /// bubble and must not anchor to the card either — only the genuine
        /// relay does.
        #[tokio::test]
        async fn a_reply_to_the_advisory_itself_is_not_review_feedback() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            let advisory = append(&rt, advisory_bubble("strategy")).await;
            append(&rt, relay_bubble("strategy")).await;

            assert!(
                rt.review_feedback_target("strategy", advisory)
                    .await
                    .unwrap()
                    .is_none(),
                "the advisory is not itself a relay bubble, so replying to it \
                 must run an ordinary chat turn"
            );
        }

        /// codex P2, 2026-09-04: the advisory filter above must not be a
        /// blanket ban on the *string* `system`.
        ///
        /// `SYSTEM_AUTHOR` is a reserved agent id, but the reservation is
        /// grandfathered on reload, so a company declared before it can carry
        /// a roster teammate literally called `system`. Its replies are
        /// ordinary teammate replies; skipping them would lose that company's
        /// review anchor entirely — trading the bug the filter fixes for a
        /// worse one on the companies it does not apply to.
        #[tokio::test]
        async fn a_grandfathered_system_teammate_still_anchors_its_own_relay() {
            let (rt, _home) = runtime_with_a_system_teammate().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            // Authored by the roster agent `system`: on this company that is a
            // teammate speaking, not the runtime reporting on itself.
            let relay = append(
                &rt,
                CompanyEvent::AgentReply {
                    audience: Vec::new(),
                    chat_id: "strategy".to_string(),
                    agent_id: crate::ports::SYSTEM_AUTHOR.to_string(),
                    text: "Here is the draft.".to_string(),
                    steps: Vec::new(),
                    task_id: None,
                    outputs: Vec::new(),
                    parent: None,
                    mentions: Vec::new(),
                    mention_depth: 0,
                },
            )
            .await;

            assert_eq!(
                rt.review_feedback_target("strategy", relay)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "a roster teammate whose id happens to be `system` keeps its \
                 relay bubble; the filter is for the runtime's own advisories, \
                 which this company has none of"
            );
        }

        /// Codex #3905031260: the event log is company-wide, so unrelated
        /// activity on another desk can put more events between a pill and its
        /// relay than a single scan page holds. Both `settle_pill_before`
        /// (backward, from the reply to the pill) and `is_relay_bubble_for`
        /// (forward, from the pill to the reply) must page past that, not give
        /// up at the first page and silently fall through to an ordinary turn.
        #[tokio::test]
        async fn the_relay_resolves_past_a_flood_of_another_desks_events() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let pill = append(&rt, settle_pill("t-1", "strategy")).await;
            for _ in 0..300 {
                append(&rt, relay_bubble("marketing")).await;
            }
            let true_relay = append(&rt, relay_bubble("strategy")).await;

            assert_eq!(
                rt.review_feedback_target("strategy", true_relay)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "300 unrelated marketing-desk events between the pill (seq {pill}) and its \
                 own relay must not hide either end of the scan behind one page"
            );
        }

        /// Codex #3906873605: a card that settles, is revised, and returns to
        /// `in_review` mints a fresh settle pill for the same `task_id` while
        /// the old one stays in the log. A reply anchored to that old pill —
        /// a stale client, a replayed request, or a direct API call — must be
        /// refused rather than re-dispatching the card's latest attempt.
        #[tokio::test]
        async fn the_resolver_declines_a_superseded_settle_pill() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let stale_pill = append(&rt, settle_pill("t-1", "strategy")).await;
            let fresh_pill = append(&rt, settle_pill("t-1", "strategy")).await;

            assert!(
                rt.review_feedback_target("strategy", stale_pill)
                    .await
                    .unwrap()
                    .is_none(),
                "a reply anchored to the superseded settle pill must not \
                 re-dispatch the card's latest attempt"
            );
            assert_eq!(
                rt.review_feedback_target("strategy", fresh_pill)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "the current settle marker still resolves the card"
            );
        }

        /// Same gate, reached through a settle pill's relay bubble rather than
        /// the pill itself — the relay off a superseded pill must not anchor
        /// either.
        #[tokio::test]
        async fn the_resolver_declines_a_relay_off_a_superseded_settle_pill() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            let stale_relay = append(&rt, relay_bubble("strategy")).await;
            append(&rt, settle_pill("t-1", "strategy")).await;
            let fresh_relay = append(&rt, relay_bubble("strategy")).await;

            assert!(
                rt.review_feedback_target("strategy", stale_relay)
                    .await
                    .unwrap()
                    .is_none(),
                "a relay bubble off the superseded pill must not re-dispatch \
                 the card's latest attempt"
            );
            assert_eq!(
                rt.review_feedback_target("strategy", fresh_relay)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-1".to_string()),
                "the relay off the current settle pill still resolves the card"
            );
        }

        /// The latest-pill gate is per card, not per desk: one card settling
        /// again must not invalidate a different card's still-current anchor
        /// in the same desk (guards the interaction with the earlier
        /// per-card-actionable fix).
        #[tokio::test]
        async fn a_superseded_pill_on_one_card_does_not_invalidate_a_sibling_cards_anchor() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            seed(&rt, &card("t-2", "strategy", COLUMN_IN_REVIEW)).await;
            let t1_pill = append(&rt, settle_pill("t-1", "strategy")).await;
            let t2_pill = append(&rt, settle_pill("t-2", "strategy")).await;
            append(&rt, settle_pill("t-1", "strategy")).await;

            assert!(
                rt.review_feedback_target("strategy", t1_pill)
                    .await
                    .unwrap()
                    .is_none(),
                "t-1's original pill is superseded by its own revision"
            );
            assert_eq!(
                rt.review_feedback_target("strategy", t2_pill)
                    .await
                    .unwrap()
                    .map(|c| c.id),
                Some("t-2".to_string()),
                "t-2's pill is untouched by t-1 settling again — the gate is per card"
            );
        }

        #[tokio::test]
        async fn the_resolver_declines_an_ordinary_message() {
            let (rt, _home) = runtime().await;
            seed(&rt, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let chatter = append(
                &rt,
                CompanyEvent::OperatorMessage {
                    text: "unrelated".to_string(),
                    by: None,
                    chat: Some("strategy".to_string()),
                    parent: None,
                    deliverable: None,
                    mentions: Vec::new(),
                    attachments: Vec::new(),
                },
            )
            .await;

            assert!(
                rt.review_feedback_target("strategy", chatter)
                    .await
                    .unwrap()
                    .is_none(),
                "a top-level message names no review surface and starts a normal turn"
            );
        }

        /// Codex #3905031268: a task-store read failure must surface as an
        /// error, not collapse into "no review target". Otherwise the explicit
        /// review endpoint answers a transient storage error with a misleading
        /// 404, and the threaded-feedback path falls through and runs the
        /// operator's review note as an ordinary chat turn.
        #[tokio::test]
        async fn a_task_store_failure_surfaces_as_an_error_not_a_missing_card() {
            let (rt, _home) = runtime_with_tasks(Some(Arc::new(FailingTasks))).await;
            let pill = append(&rt, settle_pill("t-1", "strategy")).await;

            let err = rt
                .review_feedback_target("strategy", pill)
                .await
                .expect_err(
                    "a storage failure must not be read as 'no review target' and fall \
                     through to an ordinary chat turn",
                );
            assert!(
                matches!(err, crate::error::OpenCompanyError::Harness(_)),
                "unexpected error: {err:?}"
            );
        }

        /// Codex #3905522633: the same gap `397807637` closed for
        /// `TaskStore::list`, one layer over — the `EventLog` reads inside
        /// `review_anchor_card`/`settle_pill_before`/`is_relay_bubble_for`
        /// must not collapse a transient read failure into "not a review
        /// anchor" and let `chat_and_emit` run the operator's review note as
        /// an ordinary chat turn.
        #[tokio::test]
        async fn an_event_log_read_failure_surfaces_as_an_error_not_a_missing_anchor() {
            let home = tempfile::Builder::new()
                .prefix("opencompany-review-events-")
                .tempdir()
                .expect("tempdir");
            let events = Arc::new(FailingReadsEventLog::new(
                crate::store::fs::FsEventLog::new(home.path().to_path_buf()),
            ));
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[group_chat]]\nid = \"strategy\"\nname = \"Strategy\"\nmembers = [\"ceo\"]\n",
            )
            .expect("manifest");
            let runtime = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"))
                    .with_events(events.clone())
                    .build()
                    .await
                    .expect("runtime"),
            );
            seed(&runtime, &card("t-1", "strategy", COLUMN_IN_REVIEW)).await;
            let pill = append(&runtime, settle_pill("t-1", "strategy")).await;

            events.fail_reads_from_now_on();

            let err = runtime
                .review_feedback_target("strategy", pill)
                .await
                .expect_err(
                    "an event-log read failure must not be read as 'not a review anchor' \
                     and fall through to an ordinary chat turn",
                );
            assert!(
                matches!(err, crate::error::OpenCompanyError::Harness(_)),
                "unexpected error: {err:?}"
            );
        }

        #[tokio::test]
        async fn feedback_appends_a_reviewer_block_and_re_enters_in_progress() {
            let (rt, _home) = runtime().await;
            let mut seeded = card("t-1", "strategy", COLUMN_IN_REVIEW);
            seeded.note = Some("[writer] first draft".to_string());
            seed(&rt, &seeded).await;

            rt.apply_review_feedback(&seeded, "tighten the intro", None)
                .await
                .expect("feedback applies");

            let after = stored(&rt, "t-1").await;
            assert_eq!(
                after.column, COLUMN_IN_PROGRESS,
                "review feedback re-runs the card through the dispatch edge"
            );
            let note = after.note.expect("note");
            assert!(note.contains("[reviewer] tighten the intro"), "{note}");
            assert!(
                note.contains("[writer] first draft"),
                "the prior note is preserved: {note}"
            );
        }

        #[tokio::test]
        async fn empty_feedback_does_not_redispatch() {
            let (rt, _home) = runtime().await;
            let mut seeded = card("t-1", "strategy", COLUMN_IN_REVIEW);
            seeded.note = Some("[writer] first draft".to_string());
            seed(&rt, &seeded).await;

            rt.apply_review_feedback(&seeded, "   ", None)
                .await
                .expect("empty feedback is accepted, not rejected");

            let after = stored(&rt, "t-1").await;
            assert_eq!(
                after.column, COLUMN_IN_REVIEW,
                "a Revise with nothing to say must not re-dispatch the card"
            );
            assert_eq!(
                after.note.as_deref(),
                Some("[writer] first draft"),
                "no reviewer block is appended when there is no feedback"
            );
        }

        #[tokio::test]
        async fn approve_finishes_the_card() {
            use crate::harness::built_in::lifecycle::ReviewDecision;
            let (rt, _home) = runtime().await;
            let seeded = card("t-1", "strategy", COLUMN_IN_REVIEW);
            seed(&rt, &seeded).await;

            rt.apply_review_decision(&seeded, ReviewDecision::Approve, None, None)
                .await
                .expect("approve applies");

            let after = stored(&rt, "t-1").await;
            assert_eq!(after.column, COLUMN_DONE);
        }
    }

    /// A blocked agent node whose whole gated-call batch is refused starts no
    /// continuation — the blocked-node twin of `resume_run`'s all-denied case
    /// — and, since PR #1991's review (`3903797619`), must also stop leaving
    /// that lineage's checkpoint on disk forever: nothing else ever comes back
    /// for a wholly refused block's thread id.
    #[cfg(feature = "openhuman")]
    #[tokio::test]
    async fn a_wholly_refused_blocked_node_prunes_its_checkpoint_lineage() {
        use tinyflows::graph::Checkpointer;

        let home = tempfile::tempdir().expect("home");
        let manifest: crate::company::CompanyManifest = toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\n\
             mode = \"full\"\n",
        )
        .expect("manifest");
        let mut runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
            .build()
            .await
            .expect("runtime");
        let checkpoints = std::sync::Arc::new(
            crate::workflows::checkpoint_store::WorkflowCheckpointStore::new(
                home.path().join("checkpoints"),
            ),
        );
        checkpoints
            .put(tinyflows::graph::Checkpoint {
                thread_id: "blocked-thread".to_string(),
                checkpoint_id: "c1".to_string(),
                run_id: Some("blocked-thread".to_string()),
                parent_checkpoint_id: None,
                namespace: Vec::new(),
                state: serde_json::json!({}),
                next_nodes: vec![tinyflows::graph::ids::NodeId::new("agent")],
                completed_tasks: Vec::new(),
                pending_writes: Vec::new(),
                interrupts: Vec::new(),
                pending_activations: None,
                barrier_arrivals: Vec::new(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("seed checkpoint");
        runtime.set_workflow_checkpoints(checkpoints.clone());

        let turn = "blocked-turn";
        runtime.blocked_nodes.arm_checkpointed(
            turn,
            "gated",
            &serde_json::json!({}),
            &crate::ports::types::StartedBy::Operator,
            Some("blocked-thread"),
            None,
        );

        runtime
            .resume_blocked_agent_node(
                &crate::ports::types::ApprovalId::new("call-1"),
                turn,
                Vec::new(),
            )
            .await
            .expect("an all-refused block does not error");

        let remaining = checkpoints
            .get_thread("blocked-thread")
            .await
            .expect("checkpoint read");
        assert!(
            remaining.is_empty(),
            "a wholly refused blocked node starts no continuation, so its checkpoint lineage \
             must be pruned: {remaining:?}"
        );
    }

    /// B-101: an `@name` that reaches two things reaches nobody — and the
    /// conversation is told so, in the conversation.
    mod ambiguous_mentions {
        use crate::company::runtime::CompanyRuntime;
        use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};
        use std::sync::Arc;
        use tempfile::TempDir;

        /// A company where one spelling reaches two different things: a roster
        /// teammate `writer` and a desk `writer`. The reported case was a
        /// teammate and a *person* sharing a name, which `mentions.rs` covers
        /// directly; the collision is the same one and this needs no user store
        /// to set up.
        async fn runtime() -> (Arc<CompanyRuntime>, TempDir) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-ambiguous-mentions-")
                .tempdir()
                .expect("tempdir");
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\
                 [[group_chat]]\nid = \"writer\"\nname = \"Writer desk\"\nmembers = [\"writer\"]\n",
            )
            .expect("manifest");
            let runtime = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"))
                    .build()
                    .await
                    .expect("runtime"),
            );
            (runtime, home)
        }

        /// Every `AgentReply` journaled so far, as `(agent, chat, text)`.
        async fn replies(runtime: &Arc<CompanyRuntime>) -> Vec<(String, String, String)> {
            runtime
                .events
                .read_from(runtime.id(), EventSeq::new(0), 500)
                .await
                .expect("events")
                .into_iter()
                .filter_map(|stored| match stored.event {
                    CompanyEvent::AgentReply {
                        agent_id,
                        chat_id,
                        text,
                        ..
                    } => Some((agent_id, chat_id, text)),
                    _ => None,
                })
                .collect()
        }

        /// The signal the founder never got: a positive line in the channel
        /// saying the ping matched two things and reached neither. It is
        /// attributed to the runtime itself, not to a teammate — the console
        /// renders `SYSTEM_AUTHOR` as a centred system pill, and putting a
        /// roster face on the runtime's own refusal would misstate who decided.
        #[tokio::test]
        async fn an_ambiguous_name_is_reported_in_the_channel_it_was_sent_to() {
            let (runtime, _home) = runtime().await;
            let resolved = runtime
                .resolve_mentions_reporting("@writer can you draft the autumn brief?", None, None)
                .await;
            assert!(
                resolved.mentions.is_empty(),
                "the ping is still refused: {:?}",
                resolved.mentions
            );
            assert_eq!(resolved.ambiguous.len(), 1, "and reported once");

            runtime
                .post_mention_ambiguity_note("main", None, &resolved.ambiguous)
                .await;

            let posted = replies(&runtime).await;
            assert_eq!(posted.len(), 1, "exactly one line: {posted:?}");
            let (agent, chat, text) = &posted[0];
            assert_eq!(agent, crate::ports::SYSTEM_AUTHOR);
            assert_eq!(chat, "main", "into the conversation it was sent to");
            assert!(text.contains("@writer"), "names the literal typed: {text}");
            assert!(
                text.contains("pinged nobody"),
                "states what happened: {text}"
            );
        }

        /// The threaded case: an ambiguous `@name` sent as a reply inside a
        /// thread must get its explanatory note posted into that same thread,
        /// not top-level in the channel — otherwise the note contradicts its own
        /// doc comment's promise to speak "in the conversation itself" the
        /// moment the operator is looking at a thread rather than the main
        /// timeline.
        #[tokio::test]
        async fn an_ambiguous_name_in_a_thread_is_reported_into_that_thread() {
            let (runtime, _home) = runtime().await;

            // Seed a root message to thread off of, the same way a real
            // threaded reply would name an existing event as its parent.
            let root = runtime
                .events
                .append(
                    runtime.id(),
                    CompanyEvent::OperatorMessage {
                        text: "kicking off a thread".to_string(),
                        by: None,
                        chat: Some("main".to_string()),
                        parent: None,
                        deliverable: None,
                        mentions: Vec::new(),
                        attachments: Vec::new(),
                    },
                )
                .await
                .expect("root event");

            let resolved = runtime
                .resolve_mentions_reporting("@writer can you draft the autumn brief?", None, None)
                .await;
            assert_eq!(resolved.ambiguous.len(), 1, "reported once");

            runtime
                .post_mention_ambiguity_note("main", Some(root), &resolved.ambiguous)
                .await;

            let threaded = runtime
                .events
                .read_from(runtime.id(), EventSeq::new(0), 500)
                .await
                .expect("events")
                .into_iter()
                .filter_map(|stored| match stored.event {
                    CompanyEvent::AgentReply {
                        parent, chat_id, ..
                    } => Some((parent, chat_id)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(threaded.len(), 1, "exactly one reply: {threaded:?}");
            let (parent, chat) = &threaded[0];
            assert_eq!(chat, "main");
            assert_eq!(
                *parent,
                Some(root),
                "the note lands in the thread the ambiguous ping was sent in, \
                 not top-level in the channel"
            );
        }

        /// The negative half, and the one that keeps the notice worth reading: a
        /// message whose names all resolve says nothing at all.
        #[tokio::test]
        async fn an_unambiguous_message_posts_nothing() {
            let (runtime, _home) = runtime().await;
            let resolved = runtime
                .resolve_mentions_reporting("@ceo can you take a look?", None, None)
                .await;
            assert_eq!(resolved.mentions.len(), 1, "the ping resolves");
            runtime
                .post_mention_ambiguity_note("main", None, &resolved.ambiguous)
                .await;
            assert!(
                replies(&runtime).await.is_empty(),
                "nothing is posted for a message that named somebody"
            );
        }
    }

    /// `notify_mentions`'s own comment calls a notification-store failure
    /// "deliberately not fatal": the mention still renders as a chip and the
    /// message still lands, only the badge is missing. Nothing forced that
    /// store to fail and checked which half of the promise actually held.
    mod notify_mentions_best_effort {
        use crate::company::runtime::CompanyRuntime;
        use crate::ports::notifications::{Notification, NotificationStore};
        use crate::ports::types::{Actor, ActorKind, CompanyId, EventSeq, Mention, MentionTarget};
        use crate::ports::users::{UserRecord, UserRole, UserStatus};
        use std::sync::Arc;
        use tempfile::TempDir;

        /// A notification store whose `append` always refuses — the store is
        /// down, not merely empty.
        struct FailingNotifications;

        #[async_trait::async_trait]
        impl NotificationStore for FailingNotifications {
            async fn append(
                &self,
                _company: &CompanyId,
                _notification: &Notification,
            ) -> crate::Result<()> {
                Err(crate::error::OpenCompanyError::Store(
                    "notification append always fails in this test".to_string(),
                ))
            }
            async fn list(
                &self,
                _company: &CompanyId,
                _user: &str,
            ) -> crate::Result<Vec<crate::ports::notifications::NotificationView>> {
                Ok(Vec::new())
            }
            async fn mark_read(
                &self,
                _company: &CompanyId,
                _user: &str,
                _ids: Option<&[String]>,
            ) -> crate::Result<u64> {
                Ok(0)
            }
        }

        /// A one-agent company with one active human collaborator, and a
        /// notification store that refuses every write.
        async fn runtime_with_failing_notifications() -> (Arc<CompanyRuntime>, TempDir, String) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-mention-notify-fail-")
                .tempdir()
                .expect("tempdir");
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
            )
            .expect("manifest");
            let runtime = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"))
                    .with_notifications(Arc::new(FailingNotifications))
                    .build()
                    .await
                    .expect("runtime"),
            );
            let user_id = crate::ports::generate_id();
            let now = crate::ports::now_millis();
            runtime
                .users()
                .upsert_user(
                    runtime.id(),
                    &UserRecord {
                        id: user_id.clone(),
                        email: "mentioned@example.test".to_string(),
                        display_name: None,
                        avatar: None,
                        role: UserRole::Member,
                        status: UserStatus::Active,
                        password_hash: None,
                        must_change_password: false,
                        created_at_millis: now,
                        last_seen_at_millis: None,
                        updated_at_millis: now,
                    },
                )
                .await
                .expect("seed user");
            (runtime, home, user_id)
        }

        /// The exact promise `notify_mentions`'s own comment makes: a
        /// notification store that will not answer must not fail somebody's
        /// message. Called directly rather than through the chat route, so the
        /// assertion lands on the one function the promise is about.
        #[tokio::test]
        async fn a_failing_notification_store_does_not_panic_or_propagate() {
            let (runtime, _home, user_id) = runtime_with_failing_notifications().await;
            let mention = Mention {
                target: MentionTarget::User { id: user_id },
                text: "@mentioned".to_string(),
                offset: 0,
                quiet: false,
            };
            // The whole assertion: `notify_mentions` returns `()`, not a
            // `Result`, and this completes without panicking even though the
            // store behind it always errors.
            runtime
                .notify_mentions(
                    runtime.id(),
                    std::slice::from_ref(&mention),
                    &EventSeq::new(1),
                    Some(&Actor {
                        kind: ActorKind::User,
                        id: "someone-else".to_string(),
                    }),
                    "main",
                )
                .await;
        }
    }

    /// Blocker DMs + reply attribution (issue #1862): a parked blocker surfaces
    /// in the responsible teammate's DM, groups by root cause, and an operator's
    /// reply routes back as a verdict.
    #[cfg(feature = "openhuman")]
    mod blocker_dms {
        use crate::company::blocker_sender::BlockerSenderSignals;
        use crate::company::runtime::{BlockerReplyPlan, CompanyRuntime};
        use crate::company::task_intent::BlockerReplyIntent;
        use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};
        use crate::ports::types::CompanyId;
        use std::sync::Arc;
        use tempfile::TempDir;

        async fn runtime() -> (Arc<CompanyRuntime>, TempDir) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-blocker-dms-")
                .tempdir()
                .expect("tempdir");
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
            )
            .expect("manifest");
            let runtime = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"))
                    .build()
                    .await
                    .expect("runtime"),
            );
            (runtime, home)
        }

        fn blocker(task_id: &str, group_key: Option<&str>) -> BlockerPayload {
            BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Tool,
                step: Some(BlockerStep::Task {
                    task_id: task_id.to_string(),
                }),
                reason: format!("could not connect to mcp server for {task_id}"),
                needed: "the integration reconnected from Apps".to_string(),
                group_key: group_key.map(str::to_string),
            }
        }

        fn assignee(id: &str) -> BlockerSenderSignals {
            BlockerSenderSignals {
                started_by: None,
                owner_desk: None,
                assignee: Some(id.to_string()),
            }
        }

        /// A blocker parks into its teammate's DM: the approval's thread is that
        /// DM, and a `blocker_parked` notification is filed pointing at it — with
        /// no payload beyond the one-line title.
        #[tokio::test]
        async fn a_blocker_surfaces_in_the_responsible_teammates_dm() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
                .await
                .expect("parks");

            let pending = runtime.pending_approvals();
            assert_eq!(pending.len(), 1);
            assert_eq!(
                pending[0].thread.as_deref(),
                Some("dm:eng"),
                "the card routes into the DM with the teammate it is attributed to"
            );

            let notes = runtime
                .notifications()
                .list(runtime.id(), "eng")
                .await
                .expect("notifications");
            let parked = notes
                .iter()
                .find(|n| n.notification.kind == "blocker_parked")
                .expect("a blocker-parked notification is filed");
            assert_eq!(parked.notification.context.as_deref(), Some("dm:eng"));
            assert!(
                parked.notification.title.contains("eng"),
                "the title names who is blocked: {}",
                parked.notification.title
            );
        }

        /// The projection names which kind of step a parked blocker stopped
        /// (issue #2028) — the console needs this to word `skip`/`cancel`
        /// honestly, since neither does the same thing to a board card that it
        /// does to a workflow node.
        #[tokio::test]
        async fn pending_approvals_names_the_stopped_steps_kind() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
                .await
                .expect("parks a task-step blocker");
            let node_payload = BlockerPayload {
                kind: BlockerKind::Information,
                source: BlockerSource::Tool,
                step: Some(BlockerStep::Node {
                    run_id: "run-1".to_string(),
                    node_id: "draft".to_string(),
                }),
                reason: "needs a model choice".to_string(),
                needed: "which model to use".to_string(),
                group_key: None,
            };
            runtime
                .park_blocker(&node_payload, "t-2", assignee("eng"))
                .await
                .expect("parks a node-step blocker");

            let pending = runtime.pending_approvals();
            assert_eq!(pending.len(), 2);
            let kinds: std::collections::HashSet<_> = pending
                .iter()
                .map(|a| a.blocker_step_kind.clone())
                .collect();
            assert_eq!(
                kinds,
                std::collections::HashSet::from([
                    Some("task".to_string()),
                    Some("node".to_string())
                ]),
                "a task-step and a node-step blocker must project distinct step kinds, not the \
                 same value: {pending:?}"
            );
        }

        /// The sender is resolved, not passed through: a park with no attribution
        /// still lands in a real DM — the orchestrator's.
        #[tokio::test]
        async fn an_unattributed_blocker_falls_to_the_orchestrator_dm() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(
                    &blocker("t-1", None),
                    "t-1",
                    BlockerSenderSignals::default(),
                )
                .await
                .expect("parks");
            assert_eq!(
                runtime.pending_approvals()[0].thread.as_deref(),
                Some("dm:ceo"),
                "with nothing named, the first (orchestrator) agent answers"
            );
        }

        /// Blockers sharing a root cause project as one group and are named by
        /// the projection's `group_key`.
        #[tokio::test]
        async fn blockers_sharing_a_cause_group_together() {
            let (runtime, _home) = runtime().await;
            for task in ["t-1", "t-2", "t-3"] {
                runtime
                    .park_blocker(
                        &blocker(task, Some("connection:slack")),
                        task,
                        assignee("eng"),
                    )
                    .await
                    .expect("parks");
            }
            let members = runtime.blocker_group_members("connection:slack", Some("task"));
            assert_eq!(
                members.len(),
                3,
                "every card on the broken connection is one group"
            );
            for summary in runtime.pending_approvals() {
                assert_eq!(summary.group_key.as_deref(), Some("connection:slack"));
            }
        }

        /// **P1 review finding on PR #2038.** A connection failure can stop
        /// both a board card and a workflow node, and both park with the same
        /// `connection:<name>` group key — but Skip means "produces nothing"
        /// to a node and "redispatch, run it again" to a task. Fanning one
        /// verdict across the two step kinds silently applies the wrong
        /// consequence to whichever wasn't addressed, so the fan-out group
        /// must split by step kind even when the root cause is shared.
        #[tokio::test]
        async fn a_shared_cause_never_fans_a_verdict_across_step_kinds() {
            let (runtime, _home) = runtime().await;
            let task_id = runtime
                .park_blocker(
                    &blocker("t-1", Some("connection:slack")),
                    "t-1",
                    assignee("eng"),
                )
                .await
                .expect("parks a task-step blocker");
            let node_payload = BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Tool,
                step: Some(BlockerStep::Node {
                    run_id: "run-1".to_string(),
                    node_id: "draft".to_string(),
                }),
                reason: "could not connect to mcp server for run-1".to_string(),
                needed: "the integration reconnected from Apps".to_string(),
                group_key: Some("connection:slack".to_string()),
            };
            let node_id = runtime
                .park_blocker(&node_payload, "t-2", assignee("eng"))
                .await
                .expect("parks a node-step blocker on the same connection");

            let fanned = runtime
                .parked_blocker_group(&task_id)
                .expect("the task blocker is still parked");
            assert_eq!(
                fanned,
                vec![task_id.clone()],
                "the task blocker's fan-out group must not include the node-step sibling \
                 just because they share a connection: {fanned:?}"
            );

            let (_, follow_up) = runtime
                .apply_blocker_reply_spawned(
                    &fanned,
                    &task_id,
                    crate::ports::blockers::BlockerVerdict::Skip,
                    "",
                    None,
                )
                .await
                .expect("resolves the task blocker alone");
            crate::company::runtime::join_follow_up(follow_up)
                .await
                .expect("follow-up runs");

            assert!(
                runtime.pending_approvals().iter().any(|p| p.id == node_id),
                "skipping the task card must not have also skipped the workflow node — \
                 it is still stalled on the same connection and still needs its own answer"
            );
        }

        /// A reply in a DM with a single pending blocker resolves it, and a
        /// grouped reply fans the verdict to every card in the group.
        #[tokio::test]
        async fn a_reply_resolves_the_whole_group_and_fans_the_verdict() {
            let (runtime, _home) = runtime().await;
            for task in ["t-1", "t-2"] {
                runtime
                    .park_blocker(
                        &blocker(task, Some("connection:slack")),
                        task,
                        assignee("eng"),
                    )
                    .await
                    .expect("parks");
            }
            let plan = runtime
                .plan_blocker_reply("dm:eng", None, "go ahead and retry")
                .await
                .expect("plan");
            let ids = match plan {
                BlockerReplyPlan::Resolve { ids, intent } => {
                    assert_eq!(intent, BlockerReplyIntent::Retry);
                    assert_eq!(ids.len(), 2, "one card, both parks");
                    ids
                }
                _ => panic!("a single group in the DM resolves"),
            };
            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "go ahead and retry", None)
                .await
                .expect("applies");
            assert!(
                runtime.pending_approvals().is_empty(),
                "the verdict fanned to every card in the group"
            );
        }

        /// Parks a blocker the way a cycle that came from **no** conversation
        /// does: `cycle_conversation` answers with a default
        /// `ApprovalConversation`, so the journal row carries `thread: None`.
        /// Every planning-pass park written before commit `26d558c92` has the
        /// same shape, and those rows survive journal replay.
        async fn park_thread_less_blocker(runtime: &Arc<CompanyRuntime>, task_id: &str) {
            use crate::ports::types::{Effect, EffectGroup};
            use crate::runtime::journal::{ApprovalConversation, TaskLink};

            let payload = blocker(task_id, None);
            let effect = Effect {
                kind: payload.effect_kind(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(&payload).expect("payload"),
                agent: None,
                run_id: None,
            };
            let id = runtime
                .approvals
                .park(runtime.id(), effect.clone())
                .await
                .expect("parks");
            runtime
                .journal
                .record_parked(
                    &id,
                    &effect,
                    super::super::now_millis(),
                    TaskLink::from_task_id(Some(task_id)),
                    ApprovalConversation::default(),
                    None,
                )
                .await
                .expect("journals");
        }

        /// A blocker that names no conversation is pending in **no**
        /// conversation — `#general` least of all.
        ///
        /// The bug (B-059): `pending_blocker_groups` matched through
        /// `same_conversation`, which reads a missing chat id as "unaddressed,
        /// therefore General". A thread-less park therefore read as pending in
        /// the company-wide line, and the founder's next top-level message there
        /// was consumed as its *answer* — accepted, settled in milliseconds with
        /// no cycle and no reply, and indistinguishable in the console from a
        /// message being worked on.
        ///
        /// All four General spellings are asserted because the fold admits all
        /// four (`is_general_chat`), so fixing only the console's `"main"` would
        /// leave the same drop reachable from a host addressing `"General"`.
        #[tokio::test]
        async fn a_thread_less_blocker_is_pending_in_no_conversation() {
            let (runtime, _home) = runtime().await;
            park_thread_less_blocker(&runtime, "t-1").await;
            assert_eq!(
                runtime.pending_approvals()[0].thread,
                None,
                "the park under test is the thread-less shape"
            );

            for desk in ["main", "general", "General", ""] {
                let plan = runtime
                    .plan_blocker_reply(desk, None, "please retry the nightly import")
                    .await
                    .expect("plan");
                assert!(
                    matches!(plan, BlockerReplyPlan::NotBlocker),
                    "a top-level message in {desk:?} must run as an ordinary turn, not settle a \
                     blocker no conversation raised: {plan:?}"
                );
            }
            assert_eq!(
                runtime.pending_approvals().len(),
                1,
                "nothing was consumed, so the blocker still pends for whoever can actually answer it"
            );
        }

        /// The carve-out is not a blanket refusal: a blocker stamped with a real
        /// thread still answers to it. Guards the fix from being "skip every
        /// blocker", which would pass the test above and break #1862 outright.
        #[tokio::test]
        async fn a_threaded_blocker_still_answers_in_its_own_dm() {
            let (runtime, _home) = runtime().await;
            park_thread_less_blocker(&runtime, "t-1").await;
            runtime
                .park_blocker(&blocker("t-2", None), "t-2", assignee("eng"))
                .await
                .expect("parks");

            let plan = runtime
                .plan_blocker_reply("dm:eng", None, "retry it")
                .await
                .expect("plan");
            match plan {
                BlockerReplyPlan::Resolve { ids, .. } => assert_eq!(
                    ids.len(),
                    1,
                    "only the blocker stamped with this DM is in scope; the thread-less one is in \
                     no conversation and must not be fanned in"
                ),
                other => panic!("the DM's own blocker still resolves: {other:?}"),
            }
        }

        /// An unrelated reply is not a verdict — it falls through to an ordinary
        /// turn rather than settling the blocker.
        #[tokio::test]
        async fn an_unrelated_reply_is_not_a_verdict() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let plan = runtime
                .plan_blocker_reply("dm:eng", None, "hey, how's it going?")
                .await
                .expect("plan");
            assert!(
                matches!(plan, BlockerReplyPlan::NotBlocker),
                "a greeting runs as a normal turn and settles nothing"
            );
            assert_eq!(
                runtime.pending_approvals().len(),
                1,
                "the blocker still pends"
            );
        }

        /// Two distinct blocked things in one DM: a bare verdict asks which; a
        /// verdict naming one resolves only that one.
        #[tokio::test]
        async fn several_blockers_disambiguate_by_name() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(
                    &blocker("t-1", Some("connection:slack")),
                    "t-1",
                    assignee("eng"),
                )
                .await
                .expect("parks");
            runtime
                .park_blocker(
                    &blocker("t-2", Some("connection:notion")),
                    "t-2",
                    assignee("eng"),
                )
                .await
                .expect("parks");

            let ambiguous = runtime
                .plan_blocker_reply("dm:eng", None, "retry it")
                .await
                .expect("plan");
            assert!(
                matches!(ambiguous, BlockerReplyPlan::AskWhich { .. }),
                "a bare verdict over two blocked things asks which"
            );

            let named = runtime
                .plan_blocker_reply("dm:eng", None, "retry slack")
                .await
                .expect("plan");
            match named {
                BlockerReplyPlan::Resolve { ids, .. } => {
                    assert_eq!(
                        ids,
                        runtime.blocker_group_members("connection:slack", Some("task"))
                    );
                }
                _ => panic!("naming the connection resolves only its group"),
            }
        }

        /// An explicit reply settles only a blocker parked in the same
        /// conversation: a verdict threaded to another DM's blocker card, sent
        /// from a desk with no blocker of its own, runs as an ordinary turn.
        #[tokio::test]
        async fn an_explicit_reply_stays_within_its_conversation() {
            let (runtime, _home) = runtime().await;
            runtime
                .park_blocker(
                    &blocker("t-1", Some("connection:slack")),
                    "t-1",
                    assignee("eng"),
                )
                .await
                .expect("parks");
            let parent = runtime
                .events
                .read_from(
                    runtime.id(),
                    crate::ports::types::EventSeq::new(0),
                    usize::MAX,
                )
                .await
                .expect("read")
                .into_iter()
                .find(|stored| {
                    matches!(
                        stored.event,
                        crate::ports::types::CompanyEvent::ApprovalParked { .. }
                    )
                })
                .expect("the park is on the log")
                .seq;

            let same = runtime
                .plan_blocker_reply("dm:eng", Some(parent), "retry")
                .await
                .expect("plan");
            assert!(
                matches!(same, BlockerReplyPlan::Resolve { .. }),
                "a reply in the blocker's own DM resolves it"
            );

            let cross = runtime
                .plan_blocker_reply("dm:ops", Some(parent), "retry")
                .await
                .expect("plan");
            assert!(
                matches!(cross, BlockerReplyPlan::NotBlocker),
                "the same verdict from another conversation settles nothing"
            );
        }

        /// Manually parks a blocker with an arbitrary `at_millis` (and
        /// therefore an arbitrary deadline), bypassing `park_blocker`'s
        /// always-now stamp — the same technique `seed_parked` uses elsewhere
        /// in this file, adapted to a real blocker payload so the group it
        /// joins is genuine.
        async fn park_blocker_at(
            runtime: &Arc<CompanyRuntime>,
            id: &str,
            payload: &BlockerPayload,
            at_millis: u64,
        ) -> crate::ports::types::ApprovalId {
            use crate::runtime::journal::{ApprovalConversation, TaskLink};
            let approval = crate::ports::types::ApprovalId::new(id);
            let effect = crate::ports::types::Effect {
                kind: payload.effect_kind(),
                group: crate::ports::types::EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
                agent: None,
                run_id: None,
            };
            runtime
                .approval_gate
                .rehydrate(approval.clone(), effect.clone(), at_millis);
            runtime
                .journal
                .record_parked(
                    &approval,
                    &effect,
                    at_millis,
                    TaskLink::Unlinked,
                    ApprovalConversation::default(),
                    None,
                )
                .await
                .expect("seed parked blocker");
            approval
        }

        /// **Issue #2028 (P2 review finding).** `blocker_group_members` is
        /// oldest-first, so the group's first receipt need not belong to the
        /// id the request addressed. An older sibling can expire mid-loop
        /// while the addressed blocker settles the requested verdict just
        /// fine; the returned receipt must describe the ADDRESSED blocker,
        /// not whichever member happens to be oldest.
        #[tokio::test]
        async fn the_addressed_members_own_outcome_is_reported_not_the_oldest_siblings() {
            let (runtime, _home) = runtime().await;
            let group = Some("connection:slack");
            // Ancient: already past its deadline against real wall-clock time.
            let old = park_blocker_at(&runtime, "old", &blocker("t-old", group), 1).await;
            // Fresh: parked now, nowhere near its deadline.
            let addressed = runtime
                .park_blocker(&blocker("t-new", group), "t-new", assignee("eng"))
                .await
                .expect("parks the addressed blocker");

            let (receipt, follow_up) = runtime
                .apply_blocker_reply_spawned(
                    &[old.clone(), addressed.clone()],
                    &addressed,
                    crate::ports::blockers::BlockerVerdict::Retry,
                    "",
                    None,
                )
                .await
                .expect("resolves the group");
            crate::company::runtime::join_follow_up(follow_up)
                .await
                .expect("follow-ups run");

            assert_eq!(
                receipt.outcome(),
                "settled",
                "the addressed blocker settled the requested verdict just fine — reporting \
                 anything else (e.g. the oldest sibling's \"expired\") tells the operator \
                 their own decision failed when it did not: {receipt:?}"
            );

            // Sanity on the test's own premise: the older sibling really did
            // expire in this same call, so a naive "receipts[0]" implementation
            // would have reported exactly that outcome instead.
            assert!(
                runtime.pending_approvals().is_empty(),
                "both members left the pending queue — one settled, one expired"
            );
        }

        /// **Issue #2028 (P2 review finding).** `parked_blocker_group` returns
        /// `None` for BOTH "never a blocker" and "was a blocker, already
        /// resolved" — `resolve_blocker` used to 400 either way. A blocker
        /// that just settled (another tab, a double-click, a sibling's fan-out
        /// beating this request) must answer the same idempotent
        /// `AlreadyResolved` an ordinary approval's double-submit gets, not a
        /// refusal that tells the operator their successful decision failed.
        #[tokio::test]
        async fn a_settled_blockers_late_request_is_already_resolved_not_refused() {
            let (runtime, _home) = runtime().await;
            let id = runtime
                .park_blocker(&blocker("t-1", None), "t-1", assignee("eng"))
                .await
                .expect("parks");
            runtime
                .apply_blocker_reply(
                    std::slice::from_ref(&id),
                    BlockerReplyIntent::Retry,
                    "go ahead",
                    None,
                )
                .await
                .expect("resolves");
            assert!(
                runtime.parked_blocker_group(&id).is_none(),
                "test setup: the blocker is no longer parked"
            );

            let (receipt, follow_up) = runtime.already_resolved_blocker_receipt(&id).expect(
                "an id that WAS a blocker must get an idempotent answer once it has \
                     resolved, not None (which the caller reads as \"never a blocker\" and \
                     refuses)",
            );
            assert!(
                matches!(
                    receipt,
                    crate::runtime::cycle::ResolveReceipt::AlreadyResolved
                ),
                "a settled blocker's late request is AlreadyResolved, not an error: {receipt:?}"
            );
            crate::company::runtime::join_follow_up(follow_up)
                .await
                .expect("the synthetic already-resolved follow-up completes cleanly");
        }

        /// The other half of the same guard: an id that was never a blocker at
        /// all — an unknown id, or an ordinary (non-blocker) approval — must
        /// still be refused. Only "was a blocker, now resolved" gets the
        /// idempotent answer.
        #[tokio::test]
        async fn an_id_that_was_never_a_blocker_gets_no_idempotent_answer() {
            let (runtime, _home) = runtime().await;
            assert!(
                runtime
                    .already_resolved_blocker_receipt(&crate::ports::types::ApprovalId::new(
                        "never-existed"
                    ))
                    .is_none(),
                "an unknown id must not be answered as a settled blocker"
            );
        }

        /// The paused card a parked blocker's approval links to.
        async fn seed_paused_card(runtime: &Arc<CompanyRuntime>, id: &str) {
            use crate::ports::tasks::{COLUMN_PAUSED, TaskDeliverable, TaskRecord, TaskTitle};

            runtime
                .ops
                .tasks
                .upsert(
                    &runtime.id,
                    &TaskRecord {
                        id: id.to_string(),
                        title: TaskTitle::authored("Draft the launch note"),
                        note: None,
                        column: COLUMN_PAUSED.to_string(),
                        priority: "medium".to_string(),
                        assignee: "eng".to_string(),
                        updated_at_millis: 1,
                        origin: crate::ports::TaskOrigin::new(Some("dm:eng".to_string()), None),
                        parent_task_id: None,
                        output: None,
                        plan: None,
                        planning_attempts: Vec::new(),
                        deliverable: TaskDeliverable::Once,
                        workflow_proposal: None,
                        origin_run_id: None,
                        origin_workflow_id: None,
                        origin_message_seq: None,
                        bounced: None,
                    },
                )
                .await
                .expect("seed card");
        }

        /// A bare agent question: `step: None`, so the resume has only the
        /// approval's task link to work from.
        fn question() -> BlockerPayload {
            BlockerPayload {
                kind: BlockerKind::Information,
                source: BlockerSource::AgentQuestion,
                step: None,
                reason: "which cluster should this deploy to?".to_string(),
                needed: "the cluster name".to_string(),
                group_key: None,
            }
        }

        /// Every verdict the durable journal banked for `id`, in append order —
        /// read off disk, not off the in-memory map a resume consumes and
        /// clears. What an operator's answer actually recorded.
        async fn banked_verdicts(
            home: &std::path::Path,
            company: &CompanyId,
            id: &crate::ports::types::ApprovalId,
        ) -> Vec<String> {
            let path = crate::store::paths::Bundle::new(home, company).journal_jsonl();
            let raw = tokio::fs::read_to_string(path).await.unwrap_or_default();
            raw.lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|line| line["record"] == "BlockerResolved" && line["id"] == id.to_string())
                .filter_map(|line| line["resolution"]["verdict"].as_str().map(str::to_string))
                .collect()
        }

        /// **Issue #2028 — a late second verdict must not overwrite the answer
        /// that already won, deterministically.** No threads: the first request
        /// is resolved and resumed to completion, and only then does a second
        /// arrive carrying a group list captured before any of it ran — exactly
        /// what a second browser tab holds, and what every caller passes
        /// (`parked_blocker_group` snapshots outside the lock).
        ///
        /// The loser must write **nothing**. Before the fix it banked its own
        /// `record_blocker_resolution` and armed its own answer before
        /// `settle_approval` told it that it had lost, so the durable journal
        /// gained a Cancel line for an approval the host had settled as Retry,
        /// and the armed Cancel was left in the side-channel with no resume left
        /// to consume it — for the next boot to re-arm and act on.
        #[tokio::test]
        async fn a_late_second_verdict_banks_nothing_over_the_answer_that_won() {
            use crate::ports::blockers::BlockerVerdict;

            let (runtime, home) = runtime().await;
            let id = runtime
                .park_blocker(&question(), "t-1", assignee("eng"))
                .await
                .expect("parks");
            // Captured BEFORE the first request runs, and reused afterwards —
            // the stale snapshot every caller holds.
            let group = runtime
                .parked_blocker_group(&id)
                .expect("the blocker is parked");

            let (winner, follow_up) = runtime
                .apply_blocker_reply_spawned(&group, &id, BlockerVerdict::Retry, "", None)
                .await
                .expect("the first request resolves");
            assert_eq!(winner.outcome(), "settled", "the first request wins");
            crate::company::runtime::join_follow_up(follow_up)
                .await
                .expect("its resume runs to completion");

            let (loser, follow_up) = runtime
                .apply_blocker_reply_spawned(&group, &id, BlockerVerdict::Cancel, "", None)
                .await
                .expect("the late request is answered, not refused");
            crate::company::runtime::join_follow_up(follow_up)
                .await
                .expect("it owes no resume");
            assert_eq!(
                loser.outcome(),
                "already_resolved",
                "the late request settled nothing: {loser:?}"
            );

            let banked = banked_verdicts(home.path(), runtime.id(), &id).await;
            assert_eq!(
                banked,
                vec!["retry".to_string()],
                "the durable journal must hold only the verdict that actually settled; a \
                 losing request that banks its own is the record disagreeing with the \
                 approval event about what the operator decided: {banked:?}"
            );
            assert!(
                runtime.grants.peek_blocker_resolution(&id).is_none(),
                "a losing request must leave nothing armed — an answer banked with no resume \
                 left to consume it is what the next boot re-arms and carries out"
            );
        }

        /// **Issue #2028 (P1 review finding) — the same race, run as a race.**
        /// Two operators resolve one blocker with different verdicts
        /// concurrently, on a multi-thread runtime so the two really interleave.
        /// Whichever verdict the durable approval event names must be the one
        /// the resume acts on, the only one banked, and the only one left armed.
        ///
        /// Repeated over fresh runtimes because the losing order is what varies:
        /// a single round can have the loser arrive after the winner's resume
        /// has already consumed the entry, which is the benign interleaving.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn concurrent_resolves_cannot_desync_the_armed_verdict_from_the_settled_one() {
            for round in 0..15 {
                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    one_concurrent_round(round),
                )
                .await
                .expect("a resolve round must not hang");
            }
        }

        async fn one_concurrent_round(round: usize) {
            use crate::ports::blockers::BlockerVerdict;

            let (runtime, home) = runtime().await;
            let payload = question();
            seed_paused_card(&runtime, "t-1").await;
            let id = runtime
                .park_blocker(&payload, "t-1", assignee("eng"))
                .await
                .expect("parks");

            // Two concurrent requests naming different verdicts for the SAME
            // id. `apply_blocker_reply_spawned` serializes internally, so this
            // is a real race on the lock, not a hand-arranged interleaving.
            let a = {
                let rt = Arc::clone(&runtime);
                let id = id.clone();
                tokio::spawn(async move {
                    rt.apply_blocker_reply_spawned(
                        std::slice::from_ref(&id),
                        &id,
                        BlockerVerdict::Retry,
                        "",
                        None,
                    )
                    .await
                })
            };
            let b = {
                let rt = Arc::clone(&runtime);
                let id = id.clone();
                tokio::spawn(async move {
                    rt.apply_blocker_reply_spawned(
                        std::slice::from_ref(&id),
                        &id,
                        BlockerVerdict::Cancel,
                        "",
                        None,
                    )
                    .await
                })
            };
            let (a, b) = tokio::join!(a, b);
            let a = a.expect("task a joins");
            let b = b.expect("task b joins");

            // Exactly one of the two racing requests actually claims the
            // approval (`settle_approval`'s atomic `resolve_outcome`); the
            // loser reads `AlreadyResolved`. Whichever wins, its verdict is
            // what both the durable event AND the armed resume must agree on.
            #[allow(clippy::type_complexity)]
            let settled = |r: &crate::Result<(
                crate::runtime::cycle::ResolveReceipt,
                tokio::task::JoinHandle<crate::Result<crate::runtime::types::CycleReport>>,
            )>| {
                matches!(
                    r,
                    Ok((crate::runtime::cycle::ResolveReceipt::Settled(_), _))
                )
            };
            let winner_verdict = match (settled(&a), settled(&b)) {
                (true, false) => BlockerVerdict::Retry,
                (false, true) => BlockerVerdict::Cancel,
                (won_a, won_b) => panic!(
                    "exactly one request must settle the approval: a settled={won_a} \
                     b settled={won_b}"
                ),
            };

            for outcome in [a, b] {
                let (_, follow_up) = outcome.expect("resolves or is already-resolved");
                crate::company::runtime::join_follow_up(follow_up)
                    .await
                    .expect("follow-up runs");
            }

            // Retry and cancel post different notes into the DM, and exactly
            // one resume runs, so the note that landed must match the winner.
            let notes: Vec<String> = runtime
                .events
                .read_from(
                    runtime.id(),
                    crate::ports::types::EventSeq::new(0),
                    usize::MAX,
                )
                .await
                .expect("read events")
                .into_iter()
                .filter_map(|stored| match stored.event {
                    crate::ports::types::CompanyEvent::AgentReply { chat_id, text, .. }
                        if chat_id == "dm:eng" =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .collect();

            let (expected, contradicting) = match winner_verdict {
                BlockerVerdict::Retry => (
                    "Got it — picking that back up now.",
                    "Okay — I've cancelled that. It's back in To-do if you want to pick it up \
                     later.",
                ),
                BlockerVerdict::Cancel => (
                    "Okay — I've cancelled that. It's back in To-do if you want to pick it up \
                     later.",
                    "Got it — picking that back up now.",
                ),
                _ => unreachable!(),
            };
            assert!(
                notes.iter().any(|n| n.as_str() == expected),
                "round {round}: the resume must post the WINNING verdict's note \
                 ({expected:?}); posted: {notes:?}"
            );
            assert!(
                !notes.iter().any(|n| n.as_str() == contradicting),
                "round {round}: the resume must never carry out the LOSING request's verdict \
                 — found its note ({contradicting:?}) even though the durable event named \
                 {winner_verdict:?}: {notes:?}"
            );

            // The note only catches the loser when it overwrote the arming
            // *before* the winner's resume consumed it, which is the narrow
            // window. The journal catches it every time: a losing request that
            // banks at all leaves a second verdict on the record for an
            // approval only one verdict ever settled.
            let banked = banked_verdicts(home.path(), runtime.id(), &id).await;
            assert_eq!(
                banked,
                vec![winner_verdict.as_str().to_string()],
                "round {round}: only the verdict that settled may be banked; the durable \
                 record must not disagree with the approval event: {banked:?}"
            );
            assert!(
                runtime.grants.peek_blocker_resolution(&id).is_none(),
                "round {round}: nothing may stay armed once the one resume this approval \
                 owed has run — a leftover answer is what the next boot re-arms and acts on"
            );
        }
    }

    /// Resuming a parked blocker (issue #1863): an operator's answer re-enters
    /// the stopped step — a task card is re-dispatched, a cancel settles it —
    /// and a blocker's inert effect is never executed.
    #[cfg(feature = "openhuman")]
    mod blocker_resume {
        use crate::company::blocker_sender::BlockerSenderSignals;
        use crate::company::runtime::CompanyRuntime;
        use crate::company::task_intent::BlockerReplyIntent;
        use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};
        use crate::ports::tasks::{
            COLUMN_IN_PROGRESS, COLUMN_IN_REVIEW, COLUMN_PAUSED, COLUMN_TODO, TaskDeliverable,
            TaskRecord, TaskTitle,
        };
        use crate::ports::types::CompanyId;
        use std::path::Path;
        use std::sync::Arc;
        use tempfile::TempDir;

        async fn build(home: &Path) -> Arc<CompanyRuntime> {
            let manifest: crate::company::CompanyManifest = toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
            )
            .expect("manifest");
            Arc::new(
                crate::runtime::RuntimeBuilder::new(home.to_path_buf(), manifest)
                    .with_id(CompanyId::new("acme"))
                    .build()
                    .await
                    .expect("runtime"),
            )
        }

        async fn runtime() -> (Arc<CompanyRuntime>, TempDir) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-blocker-resume-")
                .tempdir()
                .expect("tempdir");
            let runtime = build(home.path()).await;
            (runtime, home)
        }

        async fn runtime_with_harness() -> (Arc<CompanyRuntime>, TempDir) {
            let (mut runtime, home) = runtime().await;
            Arc::get_mut(&mut runtime)
                .expect("runtime is not shared yet")
                .set_harness(Arc::new(crate::harness::HarnessPool::new()));
            (runtime, home)
        }

        fn blocker(task_id: &str) -> BlockerPayload {
            BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Task {
                    task_id: task_id.to_string(),
                }),
                reason: format!("the model id `gpt-nope` was rejected for {task_id}"),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            }
        }

        fn assignee(id: &str) -> BlockerSenderSignals {
            BlockerSenderSignals {
                started_by: None,
                owner_desk: None,
                assignee: Some(id.to_string()),
            }
        }

        fn card(id: &str, column: &str) -> TaskRecord {
            TaskRecord {
                id: id.to_string(),
                title: TaskTitle::authored("Draft the launch note"),
                note: None,
                column: column.to_string(),
                priority: "medium".to_string(),
                assignee: "eng".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("dm:eng".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            }
        }

        async fn seed(runtime: &Arc<CompanyRuntime>, c: &TaskRecord) {
            runtime
                .ops
                .tasks
                .upsert(&runtime.id, c)
                .await
                .expect("seed card");
        }

        async fn stored(runtime: &Arc<CompanyRuntime>, id: &str) -> TaskRecord {
            runtime
                .ops
                .tasks
                .list(&runtime.id)
                .await
                .expect("list")
                .into_iter()
                .find(|t| t.id == id)
                .expect("card exists")
        }

        /// The headline of the tier: an operator's "retry" moves the paused card
        /// back into In Progress so its dispatch edge fires, and the blocker is
        /// cleared.
        #[tokio::test]
        async fn retry_redispatches_the_paused_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "go ahead and retry", None)
                .await
                .expect("resumes");

            assert!(
                runtime.pending_approvals().is_empty(),
                "the answered blocker is retired"
            );
            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_IN_PROGRESS,
                "a retry re-enters the stopped card through the dispatch edge"
            );
        }

        /// An amend carries the operator's answer onto the card so the re-run
        /// reads the correction, and re-dispatches it.
        #[tokio::test]
        async fn amend_carries_the_answer_onto_the_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(
                    &ids,
                    BlockerReplyIntent::Amend,
                    "use gpt-4o-mini instead",
                    None,
                )
                .await
                .expect("resumes");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(after.column, COLUMN_IN_PROGRESS);
            let note = after.note.expect("the answer is on the card");
            assert!(
                note.contains("use gpt-4o-mini instead"),
                "the re-run must read the operator's correction: {note}"
            );
        }

        #[tokio::test]
        async fn skip_settles_the_paused_card_without_another_run() {
            use crate::ports::runs::RunFilter;

            let (runtime, _home) = runtime_with_harness().await;
            let mut paused = card("t-1", COLUMN_PAUSED);
            paused.origin = crate::ports::TaskOrigin::new(Some("general".to_string()), None);
            paused.bounced = Some("an older attempt failed".to_string());
            seed(&runtime, &paused).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Skip, "skip it", None)
                .await
                .expect("resumes");

            let runs = runtime
                .runs()
                .list_runs(runtime.id(), &RunFilter::for_task("t-1"))
                .await
                .expect("list runs");
            assert_eq!(runs.len(), 0, "a skip must not open another attempt");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(after.column, COLUMN_IN_REVIEW);
            assert!(after.output.is_none());
            assert!(after.bounced.is_none());
            assert_eq!(after.origin_chat_id(), Some("dm:eng"));
            assert!(
                after
                    .note
                    .as_deref()
                    .is_some_and(|note| note.contains("blocker question waived by the operator"))
            );

            let replies = dm_notes(&runtime).await;
            assert!(replies.iter().any(|reply| {
                reply
                    == "Okay — I've waived that blocker. The card is in review; nothing ran again."
            }));

            let notification = runtime
                .notifications()
                .list(runtime.id(), "eng")
                .await
                .expect("notifications")
                .into_iter()
                .find(|notification| notification.notification.kind == "blocker_resumed")
                .expect("settle notification");
            assert!(notification.notification.title.contains("was waived"));
            assert!(
                !notification
                    .notification
                    .title
                    .contains("picking it back up")
            );
        }

        /// A cancel settles the card and starts nothing: it lands back in To-do
        /// carrying the reason, and — the sharpest risk — the paused card is not
        /// re-dispatched.
        #[tokio::test]
        async fn cancel_settles_the_card_to_todo() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Cancel, "cancel it", None)
                .await
                .expect("settles");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(
                after.column, COLUMN_TODO,
                "a cancel abandons the work rather than re-dispatching it"
            );
            assert!(after.bounced.is_some(), "the card is marked not-fresh");
        }

        /// What an agent's own `escalate_to_human` parks: a question with no
        /// step, because the tool holds neither a card nor a node.
        async fn dm_notes(runtime: &Arc<CompanyRuntime>) -> Vec<String> {
            runtime
                .events
                .read_from(
                    runtime.id(),
                    crate::ports::types::EventSeq::new(0),
                    usize::MAX,
                )
                .await
                .expect("read events")
                .into_iter()
                .filter_map(|stored| match stored.event {
                    crate::ports::types::CompanyEvent::AgentReply { chat_id, text, .. }
                        if chat_id == "dm:eng" =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .collect()
        }

        /// Journals an operator line in the teammate's DM and hands back its
        /// sequence, the root a reply in that DM threads off.
        async fn asked_in_dm(runtime: &Arc<CompanyRuntime>) -> crate::ports::types::EventSeq {
            runtime
                .events
                .append(
                    &runtime.id,
                    crate::ports::types::CompanyEvent::OperatorMessage {
                        text: "which brief is current?".to_string(),
                        chat: Some("dm:eng".to_string()),
                        parent: None,
                        by: None,
                        deliverable: None,
                        mentions: Vec::new(),
                        attachments: Vec::new(),
                    },
                )
                .await
                .expect("journal the question")
        }

        async fn dm_replies(
            runtime: &Arc<CompanyRuntime>,
        ) -> Vec<(String, Option<crate::ports::types::EventSeq>)> {
            runtime
                .events
                .read_from(
                    runtime.id(),
                    crate::ports::types::EventSeq::new(0),
                    usize::MAX,
                )
                .await
                .expect("read events")
                .into_iter()
                .filter_map(|stored| match stored.event {
                    crate::ports::types::CompanyEvent::AgentReply {
                        chat_id,
                        text,
                        parent,
                        ..
                    } if chat_id == "dm:eng" => Some((text, parent)),
                    _ => None,
                })
                .collect()
        }

        fn agent_question() -> BlockerPayload {
            BlockerPayload {
                kind: BlockerKind::Information,
                source: BlockerSource::AgentQuestion,
                step: None,
                reason: "which of the two briefs is the current one?".to_string(),
                needed: "an answer from you".to_string(),
                group_key: None,
            }
        }

        /// Parks a blocker the journal records as belonging to **no** card, the
        /// way a workflow node's does. `park_blocker` always links the card it
        /// is given, so the unlinked case has to be built here.
        async fn park_unlinked(
            runtime: &Arc<CompanyRuntime>,
            payload: &BlockerPayload,
        ) -> crate::ports::types::ApprovalId {
            use crate::ports::now_millis;
            use crate::ports::types::{Effect, EffectGroup};
            use crate::runtime::journal::{ApprovalConversation, TaskLink};

            let effect = Effect {
                kind: payload.effect_kind(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(payload).expect("payload"),
                agent: None,
                run_id: None,
            };
            let id = runtime
                .approvals
                .park(&runtime.id, effect.clone())
                .await
                .expect("parks");
            runtime
                .journal
                .record_parked(
                    &id,
                    &effect,
                    now_millis(),
                    TaskLink::Unlinked,
                    ApprovalConversation {
                        thread: Some("dm:eng".to_string()),
                        parent: None,
                    },
                    None,
                )
                .await
                .expect("records");
            id
        }

        /// The defect this tier was missing: a question parked with no step of
        /// its own still re-enters the card its approval is linked to, and the
        /// operator's answer rides onto it.
        #[tokio::test]
        async fn an_agent_question_re_enters_the_card_its_approval_links() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&agent_question(), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(
                    &ids,
                    BlockerReplyIntent::Amend,
                    "the second brief is current",
                    None,
                )
                .await
                .expect("resumes");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(
                after.column, COLUMN_IN_PROGRESS,
                "a stepless question resumes through its approval's task link"
            );
            assert!(
                after
                    .note
                    .as_deref()
                    .unwrap_or_default()
                    .contains("the second brief is current"),
                "the answer reaches the re-run: {:?}",
                after.note
            );
        }

        /// The same fallback settles rather than re-dispatches when the answer
        /// is a cancel — the arm that moves a card for the first time.
        #[tokio::test]
        async fn an_agent_question_cancelled_settles_the_linked_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&agent_question(), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Cancel, "drop it", None)
                .await
                .expect("settles");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(after.column, COLUMN_TODO);
            assert!(after.bounced.is_some(), "the card is marked not-fresh");
        }

        /// The negative that keeps the fallback honest: a blocker the journal
        /// records against no card touches no card, however it is answered. A
        /// fallback that reached for "whichever card was paused" would resume
        /// work nobody asked about.
        #[tokio::test]
        async fn an_unlinked_question_moves_no_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            let id = park_unlinked(&runtime, &agent_question()).await;

            runtime
                .apply_blocker_reply(&[id], BlockerReplyIntent::Retry, "go on", None)
                .await
                .expect("resumes");

            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_PAUSED,
                "an unlinked question leaves every card where it was"
            );
        }

        /// A card an operator moved on from is not the step, so the answer
        /// goes back into the conversation.
        ///
        /// Both card resumes leave a card that is no longer paused exactly
        /// where it is and return without a word, so following the link to one
        /// would deliver the answer nowhere at all while the blocker is still
        /// recorded as resumed.
        #[tokio::test]
        async fn an_agent_question_whose_card_moved_on_answers_the_conversation() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&agent_question(), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();
            seed(&runtime, &card("t-1", COLUMN_IN_PROGRESS)).await;

            runtime
                .apply_blocker_reply(
                    &ids,
                    BlockerReplyIntent::Amend,
                    "the second brief is current",
                    None,
                )
                .await
                .expect("resumes");

            let after = stored(&runtime, "t-1").await;
            assert_eq!(
                after.column, COLUMN_IN_PROGRESS,
                "a card an operator moved on is left where they put it"
            );
            let notes = dm_notes(&runtime).await;
            assert!(
                notes.iter().any(
                    |note| note == "Thanks — using that and carrying on from where it stopped."
                ),
                "the answer must reach the conversation it was asked in; posted: {notes:?}"
            );
        }

        /// Every resume acknowledgement lands in the thread the question was
        /// asked in, not at the channel root.
        ///
        /// The anchor is the one the approval recorded when it parked. Driven
        /// directly because `park_blocker` records no
        /// parent of its own: only an `escalate_to_human` park carries one, and
        /// what is under test is that each resume passes on the anchor it is
        /// handed rather than dropping it.
        #[tokio::test]
        async fn a_resume_note_threads_off_the_question_it_answers() {
            use crate::ports::blockers::{BlockerResolution, BlockerVerdict};

            for (verdict, expected) in [
                (BlockerVerdict::Retry, "Got it — picking that back up now."),
                (
                    BlockerVerdict::Cancel,
                    "Okay — I've cancelled that. It's back in To-do if you want to pick it up \
                     later.",
                ),
                (
                    BlockerVerdict::Skip,
                    "Okay — I've waived that blocker. The card is in review; nothing ran again.",
                ),
            ] {
                let (runtime, _home) = runtime().await;
                seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
                let root = asked_in_dm(&runtime).await;
                let resolution = BlockerResolution {
                    verdict,
                    answer: String::new(),
                    step: None,
                };

                match verdict {
                    BlockerVerdict::Cancel => runtime
                        .cancel_task_card("t-1", Some("dm:eng"), Some(root))
                        .await
                        .expect("cancels"),
                    BlockerVerdict::Skip => runtime
                        .skip_task_card("t-1", Some("dm:eng"), Some(root))
                        .await
                        .expect("skips"),
                    BlockerVerdict::Retry => runtime
                        .resume_task_card("t-1", &resolution, Some("dm:eng"), Some(root))
                        .await
                        .expect("resumes"),
                    BlockerVerdict::Amend => unreachable!(),
                }

                let threaded: Vec<Option<crate::ports::types::EventSeq>> = dm_replies(&runtime)
                    .await
                    .into_iter()
                    .filter(|(text, _)| text == expected)
                    .map(|(_, parent)| parent)
                    .collect();
                assert_eq!(
                    threaded,
                    vec![Some(root)],
                    "the {verdict:?} acknowledgement must hang off the question it answers"
                );
            }
        }

        /// A resume whose recorded anchor no longer exists still answers, in
        /// the channel. A root that is gone threads nothing, and the
        /// acknowledgement is owed either way.
        #[tokio::test]
        async fn a_resume_note_whose_anchor_is_gone_still_answers() {
            use crate::ports::blockers::{BlockerResolution, BlockerVerdict};

            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            let resolution = BlockerResolution {
                verdict: BlockerVerdict::Retry,
                answer: String::new(),
                step: None,
            };

            runtime
                .resume_task_card(
                    "t-1",
                    &resolution,
                    Some("dm:eng"),
                    Some(crate::ports::types::EventSeq::new(9_999)),
                )
                .await
                .expect("resumes");

            let posted = dm_replies(&runtime).await;
            assert_eq!(
                posted,
                vec![("Got it — picking that back up now.".to_string(), None)],
                "an anchor that is gone falls back to the channel rather than swallowing the \
                 acknowledgement"
            );
        }

        /// The guard the whole tier turns on: a blocker's effect is inert, so a
        /// resuming verdict (mapped to Approve) must **never** execute it. The
        /// execute path records an `EffectExecuted` key; the resume path records
        /// none.
        #[tokio::test]
        async fn a_resolved_blocker_never_executes_its_effect() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();
            let approval_id = ids[0].clone();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "retry", None)
                .await
                .expect("resumes");

            assert!(
                !runtime
                    .journal
                    .is_executed(&format!("approval:{approval_id}")),
                "a resolving blocker verdict must route to resume, never perform_effect"
            );
        }

        /// A card an operator has since dragged out of `paused` is theirs — a
        /// resume must not yank it back, exactly as the expiry mover leaves it.
        #[tokio::test]
        async fn a_skip_leaves_a_card_moved_out_of_paused_alone() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_TODO)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            runtime
                .apply_blocker_reply(&ids, BlockerReplyIntent::Skip, "skip", None)
                .await
                .expect("resumes");

            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_TODO,
                "a card the operator already moved on is left where they put it"
            );
        }

        /// Restart durability: a blocker parked before a restart is resolved
        /// after it — the runtime rebuilt from the journal still resumes.
        #[tokio::test]
        async fn a_blocker_parked_before_a_restart_still_resumes_after_it() {
            let home = tempfile::Builder::new()
                .prefix("opencompany-blocker-durable-")
                .tempdir()
                .expect("tempdir");
            let first = build(home.path()).await;
            seed(&first, &card("t-1", COLUMN_PAUSED)).await;
            first
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            drop(first);

            // A fresh runtime over the same journal and board — a restart.
            let second = build(home.path()).await;
            let ids: Vec<_> = second
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();
            assert_eq!(ids.len(), 1, "the parked blocker survived the restart");

            second
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "retry", None)
                .await
                .expect("resumes");

            assert_eq!(
                stored(&second, "t-1").await.column,
                COLUMN_IN_PROGRESS,
                "a blocker parked before the restart re-enters the card after it"
            );
        }

        fn operator() -> crate::ports::types::Actor {
            crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::Operator,
                id: "operator".to_string(),
            }
        }

        /// Issue #2008: an operator answering a blocker from the console
        /// Approvals page — not the DM — must arm the same resolution a DM reply
        /// does, so the paused card re-enters through the dispatch edge. Without
        /// the console-side arming the verdict settles but the resume fork finds
        /// nothing and the card stays `paused`.
        #[tokio::test]
        async fn console_approve_resumes_the_paused_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let id = runtime
                .pending_approvals()
                .into_iter()
                .next()
                .expect("parked")
                .id;

            runtime
                .resolve_approval(&id, crate::ports::types::Verdict::Approve, operator())
                .await
                .expect("resolves");

            assert!(
                runtime.pending_approvals().is_empty(),
                "the approved blocker is retired"
            );
            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_IN_PROGRESS,
                "a console Approve arms the blocker resolution and re-enters the paused card"
            );
        }

        /// Issue #2008: the console Approve must not execute the inert blocker
        /// effect — the #1861 never-execute guard still holds on this path, the
        /// same way it does for a DM answer.
        #[tokio::test]
        async fn console_approve_never_executes_the_blocker_effect() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let id = runtime
                .pending_approvals()
                .into_iter()
                .next()
                .expect("parked")
                .id;

            runtime
                .resolve_approval(&id, crate::ports::types::Verdict::Approve, operator())
                .await
                .expect("resolves");

            assert!(
                !runtime.journal.is_executed(&format!("approval:{id}")),
                "a console Approve of a blocker must resume, never perform_effect"
            );
        }

        /// **Codex review finding on PR #2140 (`3955615146`).** A durable
        /// blocker answer banked but not yet settled — the exact window between
        /// `arm_console_blocker_resolution` and `settle_claimed_blocker` a crash
        /// or a stop can land in — is neither an explicit continuation nor a
        /// blocked-node stash, so releasing the stop must redrive it itself
        /// rather than leaving it for the next restart.
        #[tokio::test]
        async fn releasing_the_stop_redrives_a_blocker_answer_the_stop_itself_refused() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let id = runtime
                .pending_approvals()
                .into_iter()
                .next()
                .expect("parked")
                .id;

            // Bank the operator's answer durably without settling it — the same
            // intermediate state a crash or a stop between the claim and the
            // resume leaves behind.
            runtime
                .arm_console_blocker_resolution(&id, crate::ports::types::Verdict::Approve)
                .await
                .expect("arms the resolution")
                .then_some(())
                .expect("the parked blocker must actually arm");

            runtime
                .emergency_pause(operator(), None)
                .await
                .expect("pause");

            assert!(
                runtime
                    .journal
                    .replayed_blocker_resolutions()
                    .iter()
                    .any(|(rid, _)| rid == &id),
                "the banked answer is durable and still owed a settle"
            );

            runtime
                .emergency_resume(operator(), None)
                .await
                .expect("resume");

            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while runtime
                    .journal
                    .replayed_blocker_resolutions()
                    .iter()
                    .any(|(rid, _)| rid == &id)
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!("releasing the stop must redrive the banked answer, not strand it")
            });

            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_IN_PROGRESS,
                "the redriven answer re-enters the paused card through the dispatch edge"
            );
        }

        /// Issue #2008: the resumed run's **output** must land back in the thread
        /// the blocker was answered in. The card here was raised in `general`,
        /// but its blocker parked into `dm:eng`; on resume the card's origin is
        /// re-pointed at that DM so the dispatch relay delivers the output there,
        /// and a `blocker_resumed` notification badges the same thread.
        #[tokio::test]
        async fn console_approve_routes_output_to_the_blocker_thread() {
            let (runtime, _home) = runtime().await;
            let mut seeded = card("t-1", COLUMN_PAUSED);
            seeded.origin = crate::ports::TaskOrigin::new(Some("general".to_string()), None);
            seed(&runtime, &seeded).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let id = runtime
                .pending_approvals()
                .into_iter()
                .next()
                .expect("parked")
                .id;

            runtime
                .resolve_approval(&id, crate::ports::types::Verdict::Approve, operator())
                .await
                .expect("resolves");

            assert_eq!(
                stored(&runtime, "t-1").await.origin_chat_id(),
                Some("dm:eng"),
                "the resumed run reports back into the thread the blocker was answered in"
            );
            let resumed = runtime
                .notifications()
                .list(runtime.id(), "eng")
                .await
                .expect("notifications")
                .into_iter()
                .find(|n| n.notification.kind == "blocker_resumed")
                .expect("a blocker-resumed notification is filed");
            assert_eq!(
                resumed.notification.context.as_deref(),
                Some("dm:eng"),
                "the badge lands on the DM the blocker was answered in"
            );
        }

        /// Issue #2008: a console Deny abandons the work — it maps to a cancel,
        /// which settles the card back to To-do and re-dispatches nothing.
        #[tokio::test]
        async fn console_deny_cancels_the_card() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let id = runtime
                .pending_approvals()
                .into_iter()
                .next()
                .expect("parked")
                .id;

            runtime
                .resolve_approval(&id, crate::ports::types::Verdict::Deny, operator())
                .await
                .expect("resolves");

            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_TODO,
                "a console Deny abandons the work rather than re-dispatching it"
            );
        }

        /// **Issue #2028 (finding 2).** The resume's board edit is a
        /// read-modify-write — list the card, check it is still paused, write it
        /// back — so it must serialize against every other board writer. Same
        /// shape as `review_card_serializes_against_the_task_writes_lock`: hold
        /// `task_writes` from the test and the resume must not move the card;
        /// release it and the resume must complete.
        #[tokio::test]
        async fn a_resume_waits_for_the_board_write_lock() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            let guard = runtime.task_writes.lock().await;

            let rt = Arc::clone(&runtime);
            let mut task = tokio::spawn(async move {
                rt.apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "go ahead", None)
                    .await
            });

            let raced_ahead =
                tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
                    .await
                    .is_ok();
            assert!(
                !raced_ahead,
                "a resume moved the card while task_writes was held elsewhere — its \
                 read-modify-write is not serializing against concurrent board writers"
            );
            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_PAUSED,
                "the card must not move while another writer holds the lock"
            );

            drop(guard);
            tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .expect("the resume never continued after task_writes was released")
                .expect("the resume task panicked")
                .expect("the resume completes once the lock is free");
            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_IN_PROGRESS,
                "and it re-dispatches once it has the lock"
            );
        }

        /// The operator-visible half of the same finding: a resume parked on
        /// `task_writes` must re-read the board when it resumes, not act on the
        /// snapshot it took before it blocked. An operator who drags the card
        /// out of `paused` in that window has decided where it goes, and a retry
        /// that yanks it back to In Progress overrides a person's own edit.
        #[tokio::test]
        async fn a_resume_leaves_a_card_an_operator_moved_while_it_waited() {
            let (runtime, _home) = runtime().await;
            seed(&runtime, &card("t-1", COLUMN_PAUSED)).await;
            runtime
                .park_blocker(&blocker("t-1"), "t-1", assignee("eng"))
                .await
                .expect("parks");
            let ids: Vec<_> = runtime
                .pending_approvals()
                .into_iter()
                .map(|a| a.id)
                .collect();

            let guard = runtime.task_writes.lock().await;

            let rt = Arc::clone(&runtime);
            let mut task = tokio::spawn(async move {
                rt.apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "go ahead", None)
                    .await
            });

            // The premise this test rests on: the resume really is still parked
            // on the lock when the operator's edit lands. Without that it would
            // pass trivially — the resume would have finished before the move,
            // and the final column would be the operator's either way.
            let raced_ahead =
                tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
                    .await
                    .is_ok();
            assert!(
                !raced_ahead,
                "the resume finished before the operator's edit, so this test would prove \
                 nothing about what it does with a card that moved under it"
            );

            // The operator moves the card themselves while the resume is parked
            // on the lock — the write the resume must notice.
            let mut moved = stored(&runtime, "t-1").await;
            moved.column = COLUMN_TODO.to_string();
            seed(&runtime, &moved).await;

            drop(guard);
            tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .expect("the resume never continued after task_writes was released")
                .expect("the resume task panicked")
                .expect("the resume completes");

            assert_eq!(
                stored(&runtime, "t-1").await.column,
                COLUMN_TODO,
                "a resume must re-read the board after waiting: the card is where the \
                 operator put it, and yanking it back to In Progress overrides their edit"
            );
        }
    }

    /// Issue #2005: the workflow-NODE half of the blocker resume — the answer
    /// reaching the engine's trigger reader, not just the DM.
    ///
    /// The workflow runner is the only double, on `workflow_resume`'s own
    /// reasoning: what is under test is whether a continuation run is started,
    /// with what trigger input, and how many times.
    #[cfg(feature = "openhuman")]
    mod node_blocker_resume {
        use std::sync::{Arc, Mutex};

        use async_trait::async_trait;
        use serde_json::{Value, json};

        use crate::company::CompanyManifest;
        use crate::company::runtime::CompanyRuntime;
        use crate::company::task_intent::BlockerReplyIntent;
        use crate::ports::blockers::{
            BlockerKind, BlockerPayload, BlockerSource, BlockerStep, BlockerVerdict,
        };
        use crate::ports::types::{CompanyId, Effect, EffectGroup};
        use crate::ports::{WorkflowRun, WorkflowRunContext, WorkflowRunner};
        use crate::runtime::RuntimeBuilder;
        use crate::runtime::journal::{ApprovalConversation, TaskLink};
        use crate::runtime::workflow_resume::{
            CONTINUATION_BLOCKER_KEY, blocker_answer_for, workflow_node_turn_key,
        };

        const RUN_ID: &str = "run-that-blocked";
        const NODE_ID: &str = "draft";
        const WORKFLOW_TOML: &str = r#"
id = "reporting"
name = "Reporting"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "draft"
kind = "output"
name = "Draft"
[[edge]]
from = "start"
to = "draft"
"#;

        #[derive(Clone, Debug)]
        struct StartedRun {
            workflow_id: String,
            input: Value,
        }

        #[derive(Default)]
        struct RecordingRunner {
            started: Mutex<Vec<StartedRun>>,
        }

        impl RecordingRunner {
            fn started(&self) -> Vec<StartedRun> {
                self.started.lock().expect("recording runner").clone()
            }
        }

        #[async_trait]
        impl WorkflowRunner for RecordingRunner {
            async fn run(
                &self,
                _company: &CompanyId,
                workflow: &crate::company::WorkflowFile,
                input: Value,
                _ctx: &WorkflowRunContext,
            ) -> crate::Result<WorkflowRun> {
                self.started
                    .lock()
                    .expect("recording runner")
                    .push(StartedRun {
                        workflow_id: workflow.id.clone(),
                        input,
                    });
                Ok(WorkflowRun {
                    output: json!({ "ok": true }),
                    pending_approvals: Vec::new(),
                    deliveries: Vec::new(),
                    cancelled: false,
                    nodes: Vec::new(),
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                })
            }
        }

        fn manifest() -> CompanyManifest {
            toml::from_str(
                "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n",
            )
            .expect("manifest")
        }

        fn seed_home() -> tempfile::TempDir {
            let dir = tempfile::Builder::new()
                .prefix("opencompany-node-blocker-")
                .tempdir()
                .expect("tempdir");
            let workflows = dir.path().join("workflows");
            std::fs::create_dir_all(&workflows).expect("workflows dir");
            std::fs::write(workflows.join("reporting.toml"), WORKFLOW_TOML).expect("seed graph");
            dir
        }

        async fn runtime(
            home: &std::path::Path,
            with_runner: bool,
        ) -> (Arc<CompanyRuntime>, Arc<RecordingRunner>) {
            let mut rt = RuntimeBuilder::new(home.to_path_buf(), manifest())
                .with_id(CompanyId::new("acme"))
                .with_seed_dir(home.to_path_buf())
                .build()
                .await
                .expect("runtime builds");
            let runner = Arc::new(RecordingRunner::default());
            if with_runner {
                rt.set_workflow_runner(runner.clone());
            }
            (Arc::new(rt), runner)
        }

        /// Parks a node blocker exactly as `park_node_blocker_as` does, and
        /// arms the same per-(run, node) stash its `stash_node_blocker_resume`
        /// writes at park time.
        async fn park_node_blocker(rt: &Arc<CompanyRuntime>, input: Value) -> String {
            park_node_blocker_stashed(rt, input, true, None).await
        }

        async fn park_node_blocker_stashed(
            rt: &Arc<CompanyRuntime>,
            input: Value,
            stash: bool,
            thread_id: Option<&str>,
        ) -> String {
            let payload = BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Node {
                    run_id: RUN_ID.to_string(),
                    node_id: NODE_ID.to_string(),
                }),
                reason: "the model id `gpt-nope` was rejected".to_string(),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            };
            let effect = Effect {
                kind: payload.effect_kind(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(&payload).expect("payload"),
                agent: None,
                run_id: Some(RUN_ID.to_string()),
            };
            let id = rt
                .approvals
                .park(rt.id(), effect.clone())
                .await
                .expect("parks");
            rt.journal()
                .record_parked(
                    &id,
                    &effect,
                    crate::ports::now_millis(),
                    TaskLink::Unlinked,
                    ApprovalConversation::default(),
                    None,
                )
                .await
                .expect("journals");
            if stash {
                let turn = workflow_node_turn_key(RUN_ID, NODE_ID);
                rt.blocked_nodes.arm_checkpointed(
                    &turn,
                    "reporting",
                    &input,
                    &crate::ports::types::StartedBy::from_scheduled(false),
                    thread_id,
                    None,
                );
                rt.journal()
                    .record_blocked_node_stashed_checkpointed(
                        &turn,
                        "reporting",
                        &input,
                        &crate::ports::types::StartedBy::from_scheduled(false),
                        thread_id,
                        None,
                    )
                    .await
                    .expect("stashes");
            }
            id.to_string()
        }

        /// [`park_node_blocker_stashed`], but for a caller that needs its own
        /// run id — a batch mixing a stashed and an unstashed member must not
        /// have them collide on one turn key.
        async fn park_node_blocker_on_run(
            rt: &Arc<CompanyRuntime>,
            run_id: &str,
            input: Value,
            stash: bool,
        ) -> String {
            let payload = BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Node {
                    run_id: run_id.to_string(),
                    node_id: NODE_ID.to_string(),
                }),
                reason: "the model id `gpt-nope` was rejected".to_string(),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            };
            let effect = Effect {
                kind: payload.effect_kind(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(&payload).expect("payload"),
                agent: None,
                run_id: Some(run_id.to_string()),
            };
            let id = rt
                .approvals
                .park(rt.id(), effect.clone())
                .await
                .expect("parks");
            rt.journal()
                .record_parked(
                    &id,
                    &effect,
                    crate::ports::now_millis(),
                    TaskLink::Unlinked,
                    ApprovalConversation::default(),
                    None,
                )
                .await
                .expect("journals");
            if stash {
                let turn = workflow_node_turn_key(run_id, NODE_ID);
                rt.blocked_nodes.arm_checkpointed(
                    &turn,
                    "reporting",
                    &input,
                    &crate::ports::types::StartedBy::from_scheduled(false),
                    None,
                    None,
                );
                rt.journal()
                    .record_blocked_node_stashed_checkpointed(
                        &turn,
                        "reporting",
                        &input,
                        &crate::ports::types::StartedBy::from_scheduled(false),
                        None,
                        None,
                    )
                    .await
                    .expect("stashes");
            }
            id.to_string()
        }

        async fn answer(
            rt: &Arc<CompanyRuntime>,
            id: &str,
            intent: BlockerReplyIntent,
            text: &str,
        ) {
            let ids = vec![crate::ports::types::ApprovalId::from(id.to_string())];
            rt.apply_blocker_reply(&ids, intent, text, None)
                .await
                .expect("applies");
        }

        /// The acceptance headline: a workflow parked at a failed node and
        /// answered `retry` re-runs, and the answer is on the trigger input the
        /// re-run carries — not banked in the DM and dropped.
        #[tokio::test]
        async fn retry_re_runs_the_node_with_the_answer_in_the_trigger_input() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;

            answer(&rt, &id, BlockerReplyIntent::Retry, "go ahead and retry").await;

            let started = runner.started();
            assert_eq!(started.len(), 1, "a retry re-enters the node exactly once");
            assert_eq!(started[0].workflow_id, "reporting");
            assert_eq!(
                started[0].input["topic"], "quarterly numbers",
                "the blocked run's own trigger input is replayed"
            );
            let carried = blocker_answer_for(&started[0].input, NODE_ID)
                .expect("readable")
                .expect("the answer rides the trigger input");
            assert_eq!(carried.verdict, BlockerVerdict::Retry);
        }

        /// An amend carries the operator's words into the re-run, the workflow
        /// twin of the card path's note append.
        #[tokio::test]
        async fn amend_carries_the_operators_words_into_the_re_run() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;

            answer(
                &rt,
                &id,
                BlockerReplyIntent::Amend,
                "use gpt-4o-mini instead",
            )
            .await;

            let started = runner.started();
            assert_eq!(started.len(), 1);
            let carried = blocker_answer_for(&started[0].input, NODE_ID)
                .expect("readable")
                .expect("the answer rides the trigger input");
            assert_eq!(carried.verdict, BlockerVerdict::Amend);
            assert_eq!(
                carried.answer, "use gpt-4o-mini instead",
                "the correction must reach the node, or the re-run repeats the failure"
            );
        }

        /// A skip proceeds past the node: the run is re-entered carrying a
        /// verdict the node reads as "waived", rather than being abandoned.
        #[tokio::test]
        async fn skip_continues_the_run_past_the_node() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;

            answer(&rt, &id, BlockerReplyIntent::Skip, "skip it").await;

            let started = runner.started();
            assert_eq!(started.len(), 1, "a skip still continues the run");
            let carried = blocker_answer_for(&started[0].input, NODE_ID)
                .expect("readable")
                .expect("the answer rides the trigger input");
            assert_eq!(carried.verdict, BlockerVerdict::Skip);
        }

        /// The one verdict that starts nothing.
        #[tokio::test]
        async fn cancel_starts_no_run() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;

            answer(&rt, &id, BlockerReplyIntent::Cancel, "cancel that").await;

            assert!(
                runner.started().is_empty(),
                "a cancel abandons the work rather than re-entering it"
            );
        }

        /// The ledgers ride the same trigger input the answer is threaded onto,
        /// so a continuation still knows what the blocked run already sent
        /// (issues #438 / #846 / #978).
        #[tokio::test]
        async fn a_skip_continuation_still_carries_what_the_run_already_sent() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let input = json!({
                "topic": "quarterly numbers",
                crate::runtime::workflow_resume::CONTINUATION_DELIVERED_KEY: [
                    { "node": "report", "kind": "owner" }
                ],
                crate::runtime::workflow_resume::CONTINUATION_PERFORMED_KEY: [
                    { "node": "post", "tool": "send", "result": { "ok": true } }
                ],
                crate::runtime::workflow_resume::CONTINUATION_DENIED_KEY: ["refused-gate"],
            });
            let id = park_node_blocker(&rt, input).await;

            answer(&rt, &id, BlockerReplyIntent::Skip, "skip it").await;

            let started = runner.started();
            assert_eq!(started.len(), 1);
            let carried = &started[0].input;
            assert_eq!(
                crate::runtime::workflow_resume::delivered_in_input(carried).len(),
                1,
                "the report the blocked run already delivered must not be sent twice"
            );
            assert_eq!(
                crate::runtime::workflow_resume::performed_in_input(carried).len(),
                1,
                "the call the blocked run already made must not be made twice"
            );
            assert_eq!(
                crate::runtime::workflow_resume::denied_in_input(carried),
                vec!["refused-gate".to_string()],
                "a gate the operator already refused must not be asked about again"
            );
        }

        /// Answering twice re-enters the node once: the second decision finds
        /// the continuation already dispatched and launches nothing.
        #[tokio::test]
        async fn a_node_is_re_entered_once_however_many_answers_land() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let first = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;
            answer(&rt, &first, BlockerReplyIntent::Retry, "retry").await;

            let second = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;
            answer(&rt, &second, BlockerReplyIntent::Retry, "retry").await;

            assert_eq!(
                runner.started().len(),
                1,
                "the dispatch marker is what stops a second continuation for one node"
            );
        }

        /// Two blocker cards on one node share one stash: only the first park
        /// arms it. Resolving the first dispatches and retires that shared
        /// stash — the second card's answer must then find the dispatch
        /// marker and be acknowledged, not read the now-missing stash as "this
        /// host no longer holds the run".
        #[tokio::test]
        async fn a_second_card_on_the_same_node_is_acknowledged_once_the_first_dispatches() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let first = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;
            let second = park_node_blocker_stashed(
                &rt,
                json!({ "topic": "quarterly numbers" }),
                false,
                None,
            )
            .await;

            answer(&rt, &first, BlockerReplyIntent::Retry, "retry").await;
            assert_eq!(
                runner.started().len(),
                1,
                "the first answer dispatches the node"
            );

            let ids = vec![crate::ports::types::ApprovalId::from(second)];
            let outcome = rt
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "retry", None)
                .await;

            assert!(
                outcome.is_ok(),
                "the second card's answer must be acknowledged once the dispatch marker is set, \
                 not returned as an error: {outcome:?}"
            );
            assert_eq!(
                runner.started().len(),
                1,
                "the dispatch marker must still stop a second continuation for this node"
            );
        }

        /// A host that no longer holds the run's stash reports it rather than
        /// acknowledging a resume that did not happen.
        #[tokio::test]
        async fn an_answer_with_no_run_to_re_enter_is_reported_not_swallowed() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker_stashed(
                &rt,
                json!({ "topic": "quarterly numbers" }),
                false,
                None,
            )
            .await;

            let ids = vec![crate::ports::types::ApprovalId::from(id)];
            let outcome = rt
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "retry", None)
                .await;

            assert!(
                outcome.is_err(),
                "an answer that reached no run must not read as a resume"
            );
            assert!(runner.started().is_empty());
        }

        /// A batch's members are independent: one failing to resume must not
        /// stop the rest from getting their own resume attempt.
        #[tokio::test]
        async fn a_batch_follow_up_continues_past_one_members_failure() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let failing_id = park_node_blocker_on_run(
                &rt,
                RUN_ID,
                json!({ "topic": "quarterly numbers" }),
                false,
            )
            .await;
            let ok_id = park_node_blocker_on_run(
                &rt,
                "run-ok",
                json!({ "topic": "quarterly numbers" }),
                true,
            )
            .await;

            let ids = vec![
                crate::ports::types::ApprovalId::from(failing_id),
                crate::ports::types::ApprovalId::from(ok_id),
            ];
            let outcome = rt
                .apply_blocker_reply(&ids, BlockerReplyIntent::Retry, "retry", None)
                .await;

            assert!(
                outcome.is_err(),
                "the batch must still surface the failing member's error"
            );
            assert_eq!(
                runner.started().len(),
                1,
                "the member after the failing one must still get its resume, not be skipped \
                 because an earlier member's follow-up errored"
            );
        }

        /// The reserved key is never written for a verdict that starts no run.
        #[tokio::test]
        async fn a_cancelled_answer_never_reaches_a_trigger_input() {
            let home = seed_home();
            let (rt, runner) = runtime(home.path(), true).await;
            let id = park_node_blocker(&rt, json!({ "topic": "quarterly numbers" })).await;

            answer(&rt, &id, BlockerReplyIntent::Cancel, "cancel that").await;

            assert!(
                runner
                    .started()
                    .iter()
                    .all(|run| run.input.get(CONTINUATION_BLOCKER_KEY).is_none()),
                "a cancel writes no answer onto any trigger input"
            );
        }

        /// A cancel must retire the per-node stash it parked with — in memory
        /// and durably — and prune the checkpoint lineage that stash names,
        /// the same as every other terminal outcome on this queue.
        #[cfg(feature = "openhuman")]
        #[tokio::test]
        async fn a_cancelled_answer_retires_the_stash_and_prunes_its_checkpoint() {
            use tinyflows::graph::Checkpointer;

            let home = seed_home();
            let mut rt = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                .with_id(CompanyId::new("acme"))
                .with_seed_dir(home.path().to_path_buf())
                .build()
                .await
                .expect("runtime builds");
            let checkpoints = std::sync::Arc::new(
                crate::workflows::checkpoint_store::WorkflowCheckpointStore::new(
                    home.path().join("checkpoints"),
                ),
            );
            checkpoints
                .put(tinyflows::graph::Checkpoint {
                    thread_id: RUN_ID.to_string(),
                    checkpoint_id: "c1".to_string(),
                    run_id: Some(RUN_ID.to_string()),
                    parent_checkpoint_id: None,
                    namespace: Vec::new(),
                    state: json!({}),
                    next_nodes: vec![tinyflows::graph::ids::NodeId::new(NODE_ID)],
                    completed_tasks: Vec::new(),
                    pending_writes: Vec::new(),
                    interrupts: Vec::new(),
                    pending_activations: None,
                    barrier_arrivals: Vec::new(),
                    metadata: Value::Null,
                })
                .await
                .expect("seed checkpoint");
            rt.set_workflow_checkpoints(checkpoints.clone());
            let rt = Arc::new(rt);

            let payload = BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Node {
                    run_id: RUN_ID.to_string(),
                    node_id: NODE_ID.to_string(),
                }),
                reason: "the model id `gpt-nope` was rejected".to_string(),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            };
            let effect = Effect {
                kind: payload.effect_kind(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::to_value(&payload).expect("payload"),
                agent: None,
                run_id: Some(RUN_ID.to_string()),
            };
            let id = rt
                .approvals
                .park(rt.id(), effect.clone())
                .await
                .expect("parks");
            rt.journal()
                .record_parked(
                    &id,
                    &effect,
                    crate::ports::now_millis(),
                    TaskLink::Unlinked,
                    ApprovalConversation::default(),
                    None,
                )
                .await
                .expect("journals");
            let turn = workflow_node_turn_key(RUN_ID, NODE_ID);
            let input = json!({ "topic": "quarterly numbers" });
            rt.blocked_nodes.arm_checkpointed(
                &turn,
                "reporting",
                &input,
                &crate::ports::types::StartedBy::from_scheduled(false),
                Some(RUN_ID),
                None,
            );
            rt.journal()
                .record_blocked_node_stashed_checkpointed(
                    &turn,
                    "reporting",
                    &input,
                    &crate::ports::types::StartedBy::from_scheduled(false),
                    Some(RUN_ID),
                    None,
                )
                .await
                .expect("stashes");

            rt.apply_blocker_reply(&[id], BlockerReplyIntent::Cancel, "cancel that", None)
                .await
                .expect("applies");

            assert!(
                !rt.blocked_nodes.is_armed(&turn),
                "a cancel must retire the stash it parked with, not leave it stranded until \
                 restart"
            );
            assert!(
                rt.journal()
                    .blocked_stashes()
                    .iter()
                    .all(|(recorded_turn, ..)| recorded_turn != &turn),
                "the durable stash mirror must be retired too"
            );
            let remaining = checkpoints
                .get_thread(RUN_ID)
                .await
                .expect("checkpoint read");
            assert!(
                remaining.is_empty(),
                "a cancel must prune the checkpoint lineage its stash named, the same as every \
                 other terminal outcome: {remaining:?}"
            );
        }

        /// The restart reconciler's own retire path for an unapproved stranded
        /// stash (a cancel that crashed before its own cleanup ran, or any
        /// other resolved-with-nothing-approved shape) must prune that stash's
        /// checkpoint lineage too, not only release the stash.
        #[cfg(feature = "openhuman")]
        #[tokio::test]
        async fn reconcile_stranded_blocked_nodes_prunes_checkpoint_lineage_for_an_unapproved_stash()
         {
            use tinyflows::graph::Checkpointer;

            let home = seed_home();
            let mut rt = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                .with_id(CompanyId::new("acme"))
                .with_seed_dir(home.path().to_path_buf())
                .build()
                .await
                .expect("runtime builds");
            let checkpoints = std::sync::Arc::new(
                crate::workflows::checkpoint_store::WorkflowCheckpointStore::new(
                    home.path().join("checkpoints"),
                ),
            );
            checkpoints
                .put(tinyflows::graph::Checkpoint {
                    thread_id: RUN_ID.to_string(),
                    checkpoint_id: "c1".to_string(),
                    run_id: Some(RUN_ID.to_string()),
                    parent_checkpoint_id: None,
                    namespace: Vec::new(),
                    state: json!({}),
                    next_nodes: vec![tinyflows::graph::ids::NodeId::new(NODE_ID)],
                    completed_tasks: Vec::new(),
                    pending_writes: Vec::new(),
                    interrupts: Vec::new(),
                    pending_activations: None,
                    barrier_arrivals: Vec::new(),
                    metadata: Value::Null,
                })
                .await
                .expect("seed checkpoint");
            rt.set_workflow_checkpoints(checkpoints.clone());

            // Stashed but never parked (or already resolved with nothing left
            // in the journal's live set) — the same "stranded, unapproved"
            // shape a crash mid-cleanup leaves behind.
            let turn = workflow_node_turn_key(RUN_ID, NODE_ID);
            rt.blocked_nodes.arm_checkpointed(
                &turn,
                "reporting",
                &json!({ "topic": "quarterly numbers" }),
                &crate::ports::types::StartedBy::from_scheduled(false),
                Some(RUN_ID),
                None,
            );

            rt.reconcile_stranded_blocked_nodes().await;

            assert!(
                !rt.blocked_nodes.is_armed(&turn),
                "an unapproved stranded stash must be retired"
            );
            let remaining = checkpoints
                .get_thread(RUN_ID)
                .await
                .expect("checkpoint read");
            assert!(
                remaining.is_empty(),
                "the reconciler must prune the checkpoint lineage an unapproved stranded stash \
                 names, not only release the stash: {remaining:?}"
            );
        }

        /// Codex review finding on PR #2140 (`3951723403`): a stash stranded by
        /// [`CompanyRuntime::reconcile_stranded_blocked_nodes`]'s own
        /// emergency-stop guard while the company was stopped previously stayed
        /// armed until the next full restart — that function's own doc says
        /// "this runs again on the boot after the release", true only because
        /// nothing ran it any sooner. `emergency_resume` now runs it itself, so
        /// releasing the stop catches this up on the still-live process instead
        /// of requiring an operator to restart the host.
        #[tokio::test]
        async fn emergency_resume_reconciles_a_stranded_stash_without_a_restart() {
            let home = seed_home();
            let rt = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                    .with_id(CompanyId::new("acme"))
                    .with_seed_dir(home.path().to_path_buf())
                    .build()
                    .await
                    .expect("runtime builds"),
            );

            let operator = crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::Operator,
                id: "owner".into(),
            };
            rt.emergency_pause(operator.clone(), None)
                .await
                .expect("pause");

            // Stashed but never approved — the same "stranded, unapproved" shape
            // a crash mid-cleanup leaves behind, here left behind by the stop
            // instead of a restart.
            let turn = workflow_node_turn_key(RUN_ID, NODE_ID);
            rt.blocked_nodes.arm_checkpointed(
                &turn,
                "reporting",
                &json!({ "topic": "quarterly numbers" }),
                &crate::ports::types::StartedBy::from_scheduled(false),
                Some(RUN_ID),
                None,
            );
            assert!(
                rt.blocked_nodes.is_armed(&turn),
                "the stash exists while the company is stopped"
            );

            rt.emergency_resume(operator, None).await.expect("resume");

            assert!(
                !rt.blocked_nodes.is_armed(&turn),
                "releasing the stop must reconcile the stranded stash immediately, without \
                 waiting for a restart"
            );
        }
    }

    /// What an engaged emergency stop has to actually stop.
    ///
    /// Every assertion here is on a **mechanism** — the turn that did not run,
    /// the tool body that never executed, the inference sample that was never
    /// written — never on `is_emergency_paused()` reading `true`. The flag can
    /// read `true` on a company that is still taking turns and still spending,
    /// so asserting it proves nothing about the halt.
    mod emergency_stop {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use super::super::{CompanyEvent, CompanyRuntime};
        use crate::ports::Brain;
        use crate::ports::brain::CycleHost;
        use crate::ports::types::{
            Actor, ActorKind, CycleRequest, CycleResult, OutboundMessage, TokenUsage,
        };

        /// A brain that does the three things a stopped company must not do:
        /// take a turn, run a tool, and bill for the inference.
        ///
        /// The "tool" is a recorded line rather than a real dispatcher because
        /// the assertion is that the turn body never ran at all — a real tool
        /// would be reached through the same `run_cycle` that is not called.
        #[derive(Default)]
        struct WorkingBrain {
            turns: AtomicUsize,
            tool_log: Mutex<Vec<String>>,
        }

        impl WorkingBrain {
            fn turns(&self) -> usize {
                self.turns.load(Ordering::SeqCst)
            }

            fn tool_calls(&self) -> Vec<String> {
                self.tool_log.lock().expect("tool log poisoned").clone()
            }
        }

        #[async_trait::async_trait]
        impl Brain for WorkingBrain {
            async fn run_cycle(
                &self,
                req: CycleRequest,
                _host: &dyn CycleHost,
            ) -> crate::Result<CycleResult> {
                self.turns.fetch_add(1, Ordering::SeqCst);
                self.tool_log
                    .lock()
                    .expect("tool log poisoned")
                    .push(format!("notify_slack({})", req.cycle_id));
                Ok(CycleResult {
                    channel_responses: vec![OutboundMessage {
                        message_id: None,
                        task_id: None,
                        outputs: Vec::new(),
                        channel: "operator".into(),
                        agent: Some("ceo".into()),
                        text: "a full turn ran".into(),
                        steps: Vec::new(),
                        reply_to: None,
                        mentions: Vec::new(),
                    }],
                    new_traces: Vec::new(),
                    ledger_deltas: Vec::new(),
                    token_usage: TokenUsage {
                        input: 4_000,
                        output: 500,
                        cached_input: 0,
                        cost_usd: 0.12,
                    },
                })
            }
        }

        /// A brain that parks an explicit `request_approval` call on every
        /// `OperatorMessage` and counts every denial it is later told about.
        #[derive(Default)]
        struct ExplicitRequestBrain {
            denials: AtomicUsize,
        }

        impl ExplicitRequestBrain {
            fn denials(&self) -> usize {
                self.denials.load(Ordering::SeqCst)
            }
        }

        #[async_trait::async_trait]
        impl Brain for ExplicitRequestBrain {
            async fn run_cycle(
                &self,
                req: CycleRequest,
                host: &dyn CycleHost,
            ) -> crate::Result<CycleResult> {
                for event in &req.events {
                    match event {
                        CompanyEvent::OperatorMessage { .. } => {
                            host.park_effect(crate::ports::types::Effect {
                                kind: crate::ports::types::REQUEST_APPROVAL_EFFECT_KIND.into(),
                                group: crate::ports::types::EffectGroup::Sign,
                                amount_usd: Some(42.0),
                                established_thread: false,
                                first_time_counterparty: false,
                                payload: serde_json::json!({
                                    "title": "Submit the filing",
                                    "question": "May I submit it?"
                                }),
                                agent: Some("ceo".into()),
                                run_id: None,
                            })
                            .await?;
                        }
                        CompanyEvent::ApprovalResolved {
                            verdict: crate::ports::types::Verdict::Deny,
                            ..
                        } => {
                            self.denials.fetch_add(1, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                }
                Ok(CycleResult {
                    channel_responses: Vec::new(),
                    new_traces: Vec::new(),
                    ledger_deltas: Vec::new(),
                    token_usage: TokenUsage::default(),
                })
            }
        }

        fn manifest() -> crate::company::CompanyManifest {
            toml::from_str(
                "[company]\nname = \"Acme\"\n\
                 [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
                 [policy]\nmode = \"full\"\n",
            )
            .expect("manifest")
        }

        fn operator() -> Actor {
            Actor {
                kind: ActorKind::Operator,
                id: "owner".into(),
            }
        }

        fn ask() -> CompanyEvent {
            CompanyEvent::OperatorMessage {
                text: "ship the release".into(),
                by: Some(operator()),
                chat: None,
                parent: None,
                deliverable: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
            }
        }

        /// A settled verdict, the receipt `spawn_follow_up` turns into a
        /// continuation turn.
        fn settled(approval: &str) -> super::super::ResolveReceipt {
            use crate::ports::types::{ApprovalId, Verdict};
            super::super::ResolveReceipt::Settled(Box::new(CompanyEvent::ApprovalResolved {
                approval_id: ApprovalId::new(approval),
                verdict: Verdict::Approve,
                by: operator(),
            }))
        }

        async fn working_company() -> (Arc<CompanyRuntime>, Arc<WorkingBrain>, tempfile::TempDir) {
            let home = tempfile::Builder::new()
                .prefix("opencompany-emergency-")
                .tempdir()
                .expect("tempdir");
            let brain = Arc::new(WorkingBrain::default());
            let rt = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                    .with_brain(brain.clone())
                    .build()
                    .await
                    .expect("runtime"),
            );
            (rt, brain, home)
        }

        /// How many inference samples the meter holds — the bill.
        async fn billed(rt: &CompanyRuntime) -> usize {
            rt.usage()
                .query(rt.id(), 0)
                .await
                .expect("usage query")
                .len()
        }

        /// The stop must survive the runtime being *assembled*, not only the
        /// runtime being constructed.
        ///
        /// `CompanyRuntime::new` gates the workflow gate queue, and then the
        /// builder replaces that field wholesale with the queue it prepared —
        /// a fresh one on a boot, the outgoing runtime's on a rebuild. Neither
        /// has a company to ask, so neither carries a gate, and the queue that
        /// actually reaches production carried none: a batch whose last sibling
        /// expired during a stop was released and destroyed, and the approved
        /// work in it could not be recovered.
        ///
        /// Built through the real builder rather than by hand, because
        /// assembling it by hand is what hid this.
        #[tokio::test]
        async fn a_builder_assembled_runtime_still_refuses_to_release_a_batch_while_stopped() {
            use crate::ports::types::{Effect, EffectGroup, Verdict};
            use crate::runtime::workflow_resume::{PAYLOAD_NODE_ID, WORKFLOW_APPROVE_KIND};

            let (rt, _brain, _home) = working_company().await;

            let gate = Effect {
                kind: WORKFLOW_APPROVE_KIND.to_string(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::json!({ PAYLOAD_NODE_ID: "node-a" }),
                agent: None,
                run_id: Some("wr-1".to_string()),
            };
            let id = crate::ports::types::ApprovalId::new("appr-a");
            rt.workflow_gates().arm("turn-1", &id, &gate);
            rt.workflow_gates().decide("turn-1", &id, Verdict::Approve);

            rt.workflow_gates()
                .release("turn-1")
                .expect("running, so the batch releases")
                .expect("a batch was armed");

            rt.workflow_gates().arm("turn-2", &id, &gate);
            rt.workflow_gates().decide("turn-2", &id, Verdict::Approve);
            rt.emergency_pause(operator(), None).await.expect("stop");

            rt.workflow_gates()
                .release("turn-2")
                .expect_err("a stopped company must not release a decided batch");
            assert!(
                rt.workflow_gates().is_armed("turn-2"),
                "the refused batch keeps every verdict it banked, for a redrive after the stop"
            );
            assert_eq!(
                rt.workflow_gates().ready_for_release(),
                vec!["turn-2".to_string()],
                "and it is discoverable, which is what makes the approved work recoverable"
            );
        }

        /// **The defect.** With the stop engaged, a new turn must not run, the
        /// tool it would have called must not execute, and no inference may be
        /// billed.
        ///
        /// The first cycle is deliberately run *before* the stop, so a fixture
        /// that silently never works cannot pass this by doing nothing.
        #[tokio::test]
        async fn a_stopped_company_runs_no_turn_calls_no_tool_and_bills_nothing() {
            let (rt, brain, _home) = working_company().await;

            rt.run_cycle(vec![ask()]).await.expect("a running company");
            assert_eq!(brain.turns(), 1, "the fixture must really run a turn");
            assert_eq!(brain.tool_calls().len(), 1);
            assert_eq!(billed(&rt).await, 1, "the fixture must really bill");

            assert!(
                rt.emergency_pause(operator(), Some("stop everything".into()))
                    .await
                    .expect("pause"),
                "this call engaged the stop"
            );

            let refused = rt.run_cycle(vec![ask()]).await;
            assert!(
                matches!(refused, Err(crate::OpenCompanyError::EmergencyStop(_))),
                "a stopped company must refuse a new cycle, got {refused:?}"
            );
            assert_eq!(
                brain.turns(),
                1,
                "no turn may run while the emergency stop is engaged"
            );
            assert_eq!(
                brain.tool_calls().len(),
                1,
                "no tool may execute while the emergency stop is engaged"
            );
            assert_eq!(
                billed(&rt).await,
                1,
                "no inference may be billed while the emergency stop is engaged"
            );
        }

        /// The journaled entry point is the one the chat route uses, so it owes
        /// the same refusal — otherwise the switch is bypassed by whichever
        /// ingress happens to append first.
        #[tokio::test]
        async fn a_stopped_company_refuses_a_journaled_cycle_too() {
            let (rt, brain, _home) = working_company().await;
            rt.emergency_pause(operator(), None).await.expect("pause");

            let seq = rt.events().append(rt.id(), ask()).await.expect("append");
            let refused = rt.run_journaled_cycle(vec![(seq, ask())], None).await;
            assert!(
                matches!(refused, Err(crate::OpenCompanyError::EmergencyStop(_))),
                "the journaled entry point must refuse too, got {refused:?}"
            );
            assert_eq!(brain.turns(), 0);
            assert_eq!(billed(&rt).await, 0);
        }

        /// The continuation funnel: every follow-up turn — an operator's
        /// verdict, a TTL expiry, a released blocker, a workflow replay —
        /// reaches its dispatch through `spawn_follow_up`. A stop that guarded
        /// only the ingress would leave that whole family running, and the TTL
        /// sweep reaches it without passing an ingress at all.
        #[tokio::test]
        async fn a_stopped_company_runs_no_continuation_turn() {
            let (rt, brain, _home) = working_company().await;
            rt.emergency_pause(operator(), None).await.expect("pause");

            let refused = rt
                .spawn_follow_up(settled("appr-continuation"))
                .await
                .expect("the follow-up task joins");
            assert!(
                matches!(refused, Err(crate::OpenCompanyError::EmergencyStop(_))),
                "a follow-up turn must be refused while stopped, got {refused:?}"
            );
            assert_eq!(
                brain.turns(),
                0,
                "a continuation must not run a turn while the stop is engaged"
            );
            assert_eq!(billed(&rt).await, 0);
        }

        /// Releasing restores **all** of it. A company that cannot resume is a
        /// worse bug than one that cannot stop.
        #[tokio::test]
        async fn releasing_the_stop_restores_turns_tools_and_billing() {
            let (rt, brain, _home) = working_company().await;
            rt.emergency_pause(operator(), None).await.expect("pause");
            assert!(rt.run_cycle(vec![ask()]).await.is_err());

            assert!(
                rt.emergency_resume(operator(), Some("all clear".into()))
                    .await
                    .expect("resume"),
                "this call released the stop"
            );

            rt.run_cycle(vec![ask()]).await.expect("a released company");
            assert_eq!(brain.turns(), 1, "the turn runs again after the release");
            assert_eq!(brain.tool_calls().len(), 1, "tools execute again");
            assert_eq!(billed(&rt).await, 1, "inference is billed again");

            rt.spawn_follow_up(settled("appr-released"))
                .await
                .expect("the follow-up task joins")
                .expect("a released company");
            assert_eq!(
                brain.turns(),
                2,
                "continuations run again after the release"
            );
        }

        /// The stop survives a restart as **enforcement**, not only as a flag.
        ///
        /// `emergency_paused: true` on a rebooted company that still runs turns
        /// is the exact shape of the defect, so the reboot is asserted by
        /// dispatching a cycle into it.
        #[tokio::test]
        async fn the_stop_survives_a_restart_and_the_rebooted_company_still_refuses_work() {
            let home = tempfile::Builder::new()
                .prefix("opencompany-emergency-reboot-")
                .tempdir()
                .expect("tempdir");

            let first = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                .with_brain(Arc::new(WorkingBrain::default()))
                .build()
                .await
                .expect("runtime");
            first
                .emergency_pause(operator(), None)
                .await
                .expect("pause");
            drop(first);

            let brain = Arc::new(WorkingBrain::default());
            let rebooted = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                    .with_brain(brain.clone())
                    .build()
                    .await
                    .expect("runtime"),
            );
            assert!(rebooted.is_emergency_paused(), "the stop replayed");

            let refused = rebooted.run_cycle(vec![ask()]).await;
            assert!(
                matches!(refused, Err(crate::OpenCompanyError::EmergencyStop(_))),
                "a rebooted stopped company must still refuse work, got {refused:?}"
            );
            assert_eq!(brain.turns(), 0);
            assert_eq!(billed(&rebooted).await, 0);

            // And the release still works on the rebooted runtime.
            rebooted
                .emergency_resume(operator(), None)
                .await
                .expect("resume");
            rebooted.run_cycle(vec![ask()]).await.expect("released");
            assert_eq!(brain.turns(), 1);
        }

        /// The native-effect path is untouched: an engaged stop still denies a
        /// side-effecting effect and still refuses to park one, exactly as
        /// before, and releasing restores both.
        #[tokio::test]
        async fn native_effect_denial_is_unchanged_by_the_admission_gate() {
            use crate::ports::approvals::ApprovalGate;
            use crate::ports::types::{Effect, EffectGroup, PolicyDecision};

            let (rt, _brain, _home) = working_company().await;
            let effect = Effect {
                kind: "filing.submit".into(),
                group: EffectGroup::Sign,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::json!({}),
                agent: Some("ceo".into()),
                run_id: None,
            };

            rt.emergency_pause(operator(), None).await.expect("pause");
            assert_eq!(
                rt.approval_gate
                    .evaluate(rt.id(), &effect)
                    .await
                    .expect("evaluate"),
                PolicyDecision::Deny,
                "the gate still denies a side-effecting effect while stopped"
            );
            assert!(
                matches!(
                    rt.approval_gate.park(rt.id(), effect.clone()).await,
                    Err(crate::OpenCompanyError::EmergencyStop(_))
                ),
                "the gate still refuses to park one while stopped"
            );

            rt.emergency_resume(operator(), None).await.expect("resume");
            assert_eq!(
                rt.approval_gate
                    .evaluate(rt.id(), &effect)
                    .await
                    .expect("evaluate"),
                PolicyDecision::Allow,
                "releasing restores the company's own `full` policy"
            );
        }

        /// An explicit-request continuation whose dispatch a live stop refused
        /// is not a blocked-node stash, so `reconcile_stranded_blocked_nodes`
        /// never sees it. Releasing the stop must still redeliver it on this
        /// same live process rather than leaving it for the next restart.
        #[tokio::test]
        async fn releasing_the_stop_redelivers_a_continuation_the_stop_itself_refused() {
            let home = tempfile::Builder::new()
                .prefix("opencompany-emergency-continuation-")
                .tempdir()
                .expect("tempdir");
            let gate = Arc::new(
                crate::policy::ManifestApprovalGate::new(manifest().policy.clone())
                    .with_ttl_millis(0),
            );
            let brain = Arc::new(ExplicitRequestBrain::default());
            let rt = Arc::new(
                crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
                    .with_brain(brain.clone())
                    .with_approvals(gate)
                    .build()
                    .await
                    .expect("runtime"),
            );

            let report = rt
                .run_cycle(vec![ask()])
                .await
                .expect("a running company parks the request");
            assert_eq!(
                report.parked.len(),
                1,
                "the fixture must really park an explicit request"
            );
            let approval_id = report.parked[0].clone();

            rt.emergency_pause(operator(), None).await.expect("pause");

            // The gate's zero TTL means the approval is already past its
            // deadline: the sweep retires it, mints its continuation, and
            // tries to dispatch it — the dispatch `spawn_follow_up`'s own
            // check refuses while the company is stopped.
            rt.sweep_expired_approvals().await.expect("sweep");

            // Let the refused dispatch's spawned task actually run (and fail)
            // before asserting on it.
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }

            assert_eq!(
                brain.denials(),
                0,
                "the stop must have refused the continuation's dispatch"
            );
            assert!(
                rt.grants.peek_continuation(&approval_id).is_some(),
                "a continuation the stop refused to dispatch must stay durable, not be lost"
            );

            rt.emergency_resume(operator(), None).await.expect("resume");

            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while brain.denials() == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "releasing the stop must redeliver the continuation it refused, without a \
                     restart; continuation_live={}",
                    rt.grants.peek_continuation(&approval_id).is_some()
                )
            });
            assert_eq!(brain.denials(), 1);
        }
    }
