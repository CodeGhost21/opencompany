use super::*;
// The globals-unaware readers: these tests assert the company's own two
// sources, so they call the form that resolves no baseline.
use crate::company::{list_workflows_union, load_workflow_union};

/// The listed rows this company itself has, with the global baseline
/// filtered out. Every company lists the baseline graphs; these tests are
/// about what this one created, deleted, or declared.
///
/// This is an **id heuristic**, not provenance: `WorkflowSummary` carries
/// no `global` flag, so a row is classified as "the baseline's" purely by
/// id membership in `crate::globals::workflows()`. A company definition of
/// the *same* id supersedes the global one and would be wrongly excluded
/// here — none of the fixtures below give a company workflow a colliding
/// id, so the gap does not fire in this suite; see
/// `write_test::workflow_create_of_an_id_matching_a_global_wins_by_content`
/// for that case asserted directly, without this helper.
fn own_rows(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("array response")
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            !crate::globals::workflows().iter().any(|w| w.id == id)
        })
        .collect()
}

const DEMO: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    summary = "Kicks it off."
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
    summary = "Does the thing."
    agent = "assistant"
    [[node]]
    id = "done"
    kind = "output"
    name = "Report"
    [[edge]]
    from = "start"
    to = "worker"
    [[edge]]
    from = "worker"
    to = "done"
    label = "ok"
"#;

/// Writes `DEMO` to `<dir>/workflows/demo.toml` and returns `dir`.
fn seed_demo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("demo.toml"), DEMO).unwrap();
    dir
}

/// Issue #383: the run route driven against a runner that can be held
/// mid-run, so detach, cancellation, and surviving a dropped client are all
/// observable through the real router.
mod running {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::company::CompanyManifest;
    use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord, EventSeq};
    use crate::ports::{CompanyStore, WorkflowRun, WorkflowRunContext, WorkflowRunner};
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    /// A runner that parks until released, and settles as cancelled if the
    /// run's stop signal fires first.
    ///
    /// It is the real `WorkflowRunner` port, so everything above it — the
    /// route, the supervisor registration, the spawned task, the journal
    /// write — is production code. Only the graph walk is stubbed, which is
    /// what lets these tests be about the *entry point* rather than about
    /// the engine (the engine's own cancel behaviour is pinned in
    /// `workflows::runner`).
    struct StalledRunner {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        /// Set only if the run was allowed to finish on its own terms —
        /// which is how a test tells "the run completed" from "the run was
        /// dropped with the connection".
        completed: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl WorkflowRunner for StalledRunner {
        async fn run(
            &self,
            _company: &CompanyId,
            _workflow: &crate::company::WorkflowFile,
            _input: serde_json::Value,
            ctx: &WorkflowRunContext,
        ) -> crate::Result<WorkflowRun> {
            // `notify_one` on both, not `notify_waiters`: a permit is
            // stored, so neither side has to be already parked. The detached
            // run answers before its task has even been polled, so a test
            // that waits on `entered` afterwards would otherwise race the
            // notification and hang.
            self.entered.notify_one();
            let released = self.release.notified();
            tokio::select! {
                () = released => {}
                () = ctx.cancel.cancelled() => {
                    return Ok(WorkflowRun {
                        output: serde_json::Value::Null,
                        pending_approvals: Vec::new(),
                        deliveries: Vec::new(),
                        cancelled: true,
                        nodes: Vec::new(),
                        notices: Vec::new(),
                        board: Vec::new(),
                        blocked_nodes: Vec::new(),
                        approvals: Vec::new(),
                    });
                }
            }
            self.completed.store(true, Ordering::SeqCst);
            Ok(WorkflowRun {
                output: serde_json::json!({ "run": {}, "nodes": {} }),
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

    struct Stalled {
        app: axum::Router,
        runtime: Arc<crate::company::runtime::CompanyRuntime>,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        completed: Arc<AtomicBool>,
    }

    const GRAPH: &str = r#"
id = "demo"
name = "Demo"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Report"
[[edge]]
from = "start"
to = "done"
label = "ok"
"#;

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("oc-workflows-running-")
            .tempdir()
            .expect("tempdir")
    }

    /// A hosted company with one overlay workflow and a runner that stalls.
    async fn stalled_company(home: &std::path::Path) -> Stalled {
        let manifest: CompanyManifest =
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
        let id = CompanyId::new("acme");
        FsCompanyStore::new(home.to_path_buf())
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                    id: "demo".to_string(),
                    toml: GRAPH.to_string(),
                }],
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_tool_grants: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                lifecycle: "running".to_string(),
                template_provenance: None,
                setup: None,
                name_confirmed: false,
                activation_completed_at: None,
                created_at_millis: None,
            })
            .await
            .unwrap();

        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(AtomicBool::new(false));
        let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        runtime.set_workflow_runner(Arc::new(StalledRunner {
            entered: entered.clone(),
            release: release.clone(),
            completed: completed.clone(),
        }));
        let runtime = Arc::new(runtime);

        let state = AppState::new(AppConfig::default());
        state.registry().insert(id.clone(), runtime.clone());
        crate::server::test_support::seed_fixed_admin(&state, "acme").await;

        Stalled {
            app: router(state),
            runtime,
            entered,
            release,
            completed,
        }
    }

    fn run_request(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/company/workflows/demo/run")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn cancel_request(run_id: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/company/workflows/runs/{run_id}/cancel"))
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap()
    }

    fn get_workflow_request() -> Request<Body> {
        Request::builder()
            .uri("/api/v1/company/workflows/demo")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap()
    }

    fn delete_workflow_request(version: &str) -> Request<Body> {
        Request::builder()
            .method("DELETE")
            .uri(format!(
                "/api/v1/company/workflows/demo?expectedVersion={version}"
            ))
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap()
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// Every event the company journaled, oldest first.
    async fn journal(
        runtime: &Arc<crate::company::runtime::CompanyRuntime>,
    ) -> Vec<CompanyEvent> {
        runtime
            .events()
            .read_from(runtime.id(), EventSeq::new(0), usize::MAX)
            .await
            .expect("read")
            .into_iter()
            .map(|s| s.event)
            .collect()
    }

    /// Waits (bounded) for a `WorkflowRunFinished` to appear.
    async fn await_finished(
        runtime: &Arc<crate::company::runtime::CompanyRuntime>,
    ) -> Option<CompanyEvent> {
        for _ in 0..200 {
            if let Some(event) = journal(runtime)
                .await
                .into_iter()
                .find(|e| matches!(e, CompanyEvent::WorkflowRunFinished { .. }))
            {
                return Some(event);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        None
    }

    /// **The keystone.** A client that walks away mid-run must not take the
    /// run with it.
    ///
    /// The route used to await the run *inside* the request future, and
    /// hyper drops that future when the peer closes — so `record_run_finished`
    /// never ran and the run produced no history entry at all. Post-#385
    /// that is strictly worse: the run has already journaled a
    /// `WorkflowRunStarted`, so the fold reports `running: true` forever and
    /// `sweep_interrupted_runs` is boot-only. "Workflow Run produces no
    /// run-history entry" is that bug, reported from staging.
    ///
    /// `Router::oneshot` reproduces the cancellation by the same mechanism
    /// hyper uses rather than by analogy: the handler future is owned by the
    /// future being polled, so dropping the latter drops the former.
    #[tokio::test]
    async fn a_dropped_connection_does_not_cancel_a_synchronous_run() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let mut running = Box::pin(c.app.clone().oneshot(run_request(
            serde_json::json!({ "input": { "request": "go" } }),
        )));
        tokio::select! {
            _ = &mut running => panic!("the run answered before the runner was under way"),
            () = c.entered.notified() => {}
        }
        // The client hangs up, exactly as a proxy does when it gives up.
        drop(running);

        // The run must still be there to finish.
        c.release.notify_one();
        let finished = await_finished(&c.runtime).await.expect(
            "the run died with the dropped connection: nothing was journaled, so the \
                     history shows it running forever",
        );
        assert!(
            c.completed.load(Ordering::SeqCst),
            "the runner never got to finish its work"
        );
        let CompanyEvent::WorkflowRunFinished {
            error, cancelled, ..
        } = finished
        else {
            unreachable!()
        };
        assert!(error.is_none(), "a client hanging up is not a run failure");
        assert!(!cancelled, "nobody cancelled this run");
    }

    /// The same proof over a **real socket**, so the keystone rests on
    /// hyper's actual behaviour rather than on `oneshot` modelling it well.
    ///
    /// **The pause after the close is load-bearing.** Hyper does not learn
    /// the peer is gone when the client calls `shutdown` — it learns when
    /// its connection task next polls the socket and reads EOF. Release the
    /// runner before that happens and the run finishes on its own merits, so
    /// the test passes against the broken code and proves nothing.
    #[tokio::test]
    async fn a_real_socket_close_does_not_cancel_a_synchronous_run() {
        use tokio::io::AsyncWriteExt;

        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = c.app.clone();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let body = serde_json::json!({ "input": {} }).to_string();
        let request = format!(
            "POST /api/v1/company/workflows/demo/run HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Cookie: {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            crate::server::test_support::fixed_cookie("acme"),
            body.len(),
        );
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();

        c.entered.notified().await;
        socket.shutdown().await.unwrap();
        drop(socket);
        // See the doc comment: without this, hyper has not yet noticed.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        c.release.notify_one();
        assert!(
            await_finished(&c.runtime).await.is_some(),
            "a real peer close cancelled the run: nothing was journaled"
        );
        assert!(c.completed.load(Ordering::SeqCst));
        server.abort();
    }

    /// `detach: true` answers `202` with the run id while the run is
    /// demonstrably still going — the half that removes the wait.
    #[tokio::test]
    async fn a_detached_run_answers_202_before_the_run_finishes() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "detach": true })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = json_body(response).await;
        assert_eq!(body["detached"], true, "{body}");
        assert!(
            body["runId"].as_str().is_some_and(|s| !s.is_empty()),
            "the id is the whole point of the response: {body}"
        );
        assert!(
            body.get("output").is_none(),
            "a detached response must not look settled: {body}"
        );
        assert!(
            !c.completed.load(Ordering::SeqCst),
            "the response arrived before the run finished, which is the point"
        );

        // And it settles on its own, with nobody waiting.
        c.release.notify_one();
        assert!(await_finished(&c.runtime).await.is_some());
    }

    /// The wire-compat guarantee in the other direction: a caller that sends
    /// no `detach` gets exactly the response it always got — a `200`
    /// carrying the settled run — so an older console is untouched.
    #[tokio::test]
    async fn a_body_without_detach_still_gets_the_synchronous_response() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;
        // Pre-release, so the runner never parks and this is a plain
        // start-to-finish call — the shape an older console makes.
        c.release.notify_one();

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "input": {} })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert!(
            body.get("output").is_some(),
            "the settled shape carries `output`: {body}"
        );
        assert!(
            body.get("detached").is_none(),
            "the synchronous response must not carry the detach discriminator: {body}"
        );
        assert!(body["runId"].as_str().is_some(), "{body}");
    }

    /// Codex review finding on PR #2140 (`3952230576`): the emergency stop
    /// is a separate switch from `lifecycle` (a stopped company still
    /// reports `running`), so `ensure_running` alone missed it here. This
    /// POST was the one manual admission door
    /// `CompanyRuntime::ensure_not_emergency_stopped`'s own doc did not
    /// enumerate, because a workflow run never reaches `run_cycle`,
    /// `spawn_follow_up`, or the boot reconciler.
    #[tokio::test]
    async fn an_emergency_stopped_company_refuses_a_manual_run() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;
        c.runtime
            .emergency_pause(
                crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Operator,
                    id: "owner".into(),
                },
                None,
            )
            .await
            .expect("pause");

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "input": {} })))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "a stopped company must refuse a manual run exactly as it refuses chat"
        );
        assert!(
            !c.completed.load(Ordering::SeqCst),
            "the refusal must return before the runner ever ran, let alone finished"
        );
    }

    /// Cancel a live run: `200`, and it settles as cancelled rather than as
    /// an error.
    #[tokio::test]
    async fn cancelling_a_live_run_stops_it_and_records_it_as_cancelled() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "detach": true })))
            .await
            .unwrap();
        let run_id = json_body(response).await["runId"]
            .as_str()
            .unwrap()
            .to_string();
        c.entered.notified().await;

        let response = c
            .app
            .clone()
            .oneshot(cancel_request(&run_id))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["cancelling"], true);

        let CompanyEvent::WorkflowRunFinished {
            cancelled,
            error,
            run_id: journaled_id,
            ..
        } = await_finished(&c.runtime)
            .await
            .expect("a cancelled run still journals a finish")
        else {
            unreachable!()
        };
        assert!(cancelled, "the outcome must say it was stopped");
        assert!(
            error.is_none(),
            "a deliberate stop is not a failure: {error:?}"
        );
        assert_eq!(
            journaled_id.as_deref(),
            Some(run_id.as_str()),
            "the finish carries the id the run route handed back — no second identifier"
        );
        assert!(
            !c.completed.load(Ordering::SeqCst),
            "the run must not have completed its work"
        );
    }

    /// **B-121: deleting a workflow stops the run of it still in flight.**
    ///
    /// Delete used to take the schedule and the revisions and leave the run
    /// executing — and, worse, leave it *uncontrollable*: the only Stop
    /// button in the product is on the workflow detail page the delete
    /// removes, so the run went on calling models and spending with nothing
    /// anywhere able to reach it, still reporting `running: true` under a
    /// workflow that no longer existed.
    ///
    /// The assertion that carries the weight is `completed`: the stalled
    /// runner finishes its work only when released, so a run that reaches
    /// its own completion here is one the delete failed to stop.
    #[tokio::test]
    async fn deleting_a_workflow_stops_the_run_of_it_still_in_flight() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "detach": true })))
            .await
            .unwrap();
        let run_id = json_body(response).await["runId"]
            .as_str()
            .unwrap()
            .to_string();
        c.entered.notified().await;
        assert_eq!(
            c.runtime.run_supervisor().live().len(),
            1,
            "the run has to be genuinely live, or this proves nothing"
        );

        let response = c.app.clone().oneshot(get_workflow_request()).await.unwrap();
        let version = json_body(response).await["version"]
            .as_str()
            .expect("the graph carries its version token")
            .to_string();
        let response = c
            .app
            .clone()
            .oneshot(delete_workflow_request(&version))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        // CodeRabbit review (PR #2053): the count the console's toast now
        // reads has to be the sweep's own answer, not a client-side guess —
        // pin it here so a regression back to "no body" or a miscounted
        // sweep shows up as a body assertion rather than only as a wrong
        // toast nobody is testing.
        assert_eq!(json_body(response).await["stoppedRuns"], 1);

        let CompanyEvent::WorkflowRunFinished {
            cancelled,
            error,
            run_id: journaled_id,
            ..
        } = await_finished(&c.runtime)
            .await
            .expect("the deleted workflow's run settles rather than running on")
        else {
            unreachable!()
        };
        assert!(
            cancelled,
            "the run of a deleted workflow settles stopped, on the Stop button's own path"
        );
        assert!(
            error.is_none(),
            "a stop that follows from a delete is not a failure: {error:?}"
        );
        assert_eq!(
            journaled_id.as_deref(),
            Some(run_id.as_str()),
            "the same run the run route handed back — no second identifier"
        );
        assert!(
            !c.completed.load(Ordering::SeqCst),
            "the run must not have gone on to finish the work of a workflow that no \
             longer exists"
        );
    }

    /// The mirror: a delete with **no** run in flight cancels nothing.
    /// Without it the sweep above could quietly grow into "delete stops
    /// something" for a company that had nothing to stop.
    #[tokio::test]
    async fn deleting_an_idle_workflow_stops_nothing() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        assert!(c.runtime.run_supervisor().live().is_empty());
        let response = c.app.clone().oneshot(get_workflow_request()).await.unwrap();
        let version = json_body(response).await["version"]
            .as_str()
            .expect("version")
            .to_string();
        let response = c
            .app
            .clone()
            .oneshot(delete_workflow_request(&version))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        // CodeRabbit review (PR #2053): the response body is what the
        // console's toast now reads, so pin the zero here too — the same
        // reason the in-flight case above pins its 1.
        assert_eq!(json_body(response).await["stoppedRuns"], 0);
        assert_eq!(
            c.runtime.stop_runs_of_workflow("demo"),
            0,
            "nothing was in flight, so nothing was stopped"
        );
    }

    /// **A synchronous run can be cancelled mid-request, and its response
    /// has to say so.**
    ///
    /// Easy to miss, because "detached" and "cancellable" sound like the
    /// same feature: the run id is registered the moment the task is
    /// spawned, and the console learns it from the `workflow_run_started`
    /// frame — so the cancel route is reachable well before the synchronous
    /// response is written. The runner then resolves to a cancelled run
    /// whose `output` is `null` with no approvals and no deliveries, which
    /// is byte-identical to a run that legitimately produced nothing. This
    /// caller was the last reader in the PR still left guessing.
    #[tokio::test]
    async fn a_synchronous_run_cancelled_mid_request_says_so_in_its_response() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let mut running = Box::pin(
            c.app
                .clone()
                .oneshot(run_request(serde_json::json!({ "input": {} }))),
        );
        // Wait until the runner is actually parked, then find the run the
        // way the console does — by its id — and stop it.
        tokio::select! {
            _ = &mut running => panic!("the run answered before the runner was under way"),
            () = c.entered.notified() => {}
        }
        // Off the supervisor rather than the journal: this stub is the
        // `WorkflowRunner` port, so it never reaches the harness runner that
        // writes `WorkflowRunStarted`. The supervisor is the registration
        // the cancel route itself consults, which makes it the more direct
        // assertion anyway — the id is addressable while the request is open.
        let live = c.runtime.run_supervisor().live();
        assert_eq!(live.len(), 1, "the open synchronous run is registered");
        let (run_id, workflow_id) = live.into_iter().next().unwrap();
        assert_eq!(workflow_id, "demo");
        let response = c
            .app
            .clone()
            .oneshot(cancel_request(&run_id))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a synchronous run is cancellable while its request is open"
        );

        // The still-open request now answers, and must not read as a clean
        // empty success.
        let response = running.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["cancelled"], true, "{body}");
        assert_eq!(body["runId"], run_id.as_str(), "{body}");
        assert!(
            !c.completed.load(Ordering::SeqCst),
            "the run must not have completed its work"
        );
    }

    /// …and the flag is omitted entirely on a run nobody stopped, so an
    /// existing caller's body is byte-unchanged.
    #[tokio::test]
    async fn an_uncancelled_synchronous_response_omits_the_flag() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;
        c.release.notify_one();

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "input": {} })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert!(
            body.get("cancelled").is_none(),
            "a run nobody stopped carries no flag at all: {body}"
        );
    }

    /// The history fold reports it, so the console can render a stopped run
    /// as stopped rather than as a clean success.
    #[tokio::test]
    async fn a_cancelled_run_reads_back_as_cancelled_and_not_running() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "detach": true })))
            .await
            .unwrap();
        let run_id = json_body(response).await["runId"]
            .as_str()
            .unwrap()
            .to_string();
        c.entered.notified().await;
        c.app
            .clone()
            .oneshot(cancel_request(&run_id))
            .await
            .unwrap();
        await_finished(&c.runtime).await.expect("settles");

        let response = c
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/company/workflows/runs")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let rows = json_body(response).await;
        let row = &rows["runs"].as_array().expect("array")[0];
        assert_eq!(row["runId"], run_id.as_str(), "{rows}");
        assert_eq!(row["cancelled"], true, "{rows}");
        assert!(
            row.get("running").is_none(),
            "a settled run is not running: {rows}"
        );
        assert!(
            row.get("error").is_none(),
            "a stopped run carries no error: {rows}"
        );
    }

    /// Unknown and already-settled are the same `404`: there is nothing to
    /// stop. Keeping a tombstone to tell them apart would mean choosing an
    /// expiry for it, and the run history already says what became of a
    /// settled run.
    #[tokio::test]
    async fn cancelling_an_unknown_or_settled_run_is_not_found() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(cancel_request("never-existed"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Now run one to completion and try again.
        let response = c
            .app
            .clone()
            .oneshot(run_request(serde_json::json!({ "detach": true })))
            .await
            .unwrap();
        let run_id = json_body(response).await["runId"]
            .as_str()
            .unwrap()
            .to_string();
        c.entered.notified().await;
        c.release.notify_one();
        await_finished(&c.runtime).await.expect("settles");

        let response = c
            .app
            .clone()
            .oneshot(cancel_request(&run_id))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a settled run is no longer cancellable"
        );
    }

    /// The cancel route is behind the same `ScopedCompany` guard as every
    /// other route in this module — an unauthenticated caller cannot stop a
    /// company's work.
    #[tokio::test]
    async fn cancelling_without_a_session_is_rejected() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/workflows/runs/whatever/cancel")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::OK,
            "an unauthenticated cancel must not be accepted"
        );
        assert!(
            response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::FORBIDDEN,
            "expected an auth rejection, got {}",
            response.status()
        );
    }

    /// The cancel path is a static prefix under `/workflows`, and `runs` is
    /// a syntactically valid workflow id — so this pins that it is not
    /// shadowed by the dynamic `/workflows/{wid}` routes, the same guarantee
    /// `run_history_is_not_shadowed_by_the_graph_read` makes for the GET.
    #[tokio::test]
    async fn the_cancel_route_is_not_shadowed_by_the_dynamic_workflow_routes() {
        let home_dir = home();
        let c = stalled_company(home_dir.path()).await;

        let response = c
            .app
            .clone()
            .oneshot(cancel_request("anything"))
            .await
            .unwrap();
        // 404 from the *cancel handler* (nothing to stop), not a 405 or a
        // route miss — reaching the handler at all is the assertion.
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = json_body(response).await;
        assert!(
            body.to_string().contains("workflow run"),
            "the 404 should come from the cancel handler: {body}"
        );
    }

    // ── Issue #542: dry run through the real route ──────────────────────

    /// A runner that completes immediately, returning one node row — enough
    /// to prove the route maps `WorkflowRun.nodes` onto the response and
    /// echoes the request's `dry_run` as the discriminator.
    struct EchoRunner;

    #[async_trait::async_trait]
    impl WorkflowRunner for EchoRunner {
        async fn run(
            &self,
            _company: &CompanyId,
            _workflow: &crate::company::WorkflowFile,
            _input: serde_json::Value,
            _ctx: &WorkflowRunContext,
        ) -> crate::Result<WorkflowRun> {
            Ok(WorkflowRun {
                output: serde_json::json!({ "run": {}, "nodes": {} }),
                pending_approvals: Vec::new(),
                deliveries: Vec::new(),
                cancelled: false,
                nodes: vec![crate::ports::WorkflowRunNodeRow {
                    node_id: "done".to_string(),
                    status: crate::ports::types::WorkflowNodeStatus::Ok,
                    elapsed_ms: 3,
                    diagnostics: Vec::new(),
                }],
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            })
        }
    }

    /// A runner that settles cleanly and hands back one delivery row —
    /// every node `ok`, no error, nothing cancelled, and a report that did
    /// not go out. The exact shape issue #981 caught reading green.
    struct DroppedReportRunner;

    /// A runner whose only delivery row is a **dry run**'s (issue #542): the
    /// report was routed as far as its destination and deliberately not
    /// dispatched. The row shape a real `deliver_outputs_dry` writes.
    struct DryRunRunner;

    #[async_trait::async_trait]
    impl WorkflowRunner for DryRunRunner {
        async fn run(
            &self,
            _company: &CompanyId,
            _workflow: &crate::company::WorkflowFile,
            _input: serde_json::Value,
            _ctx: &WorkflowRunContext,
        ) -> crate::Result<WorkflowRun> {
            Ok(WorkflowRun {
                output: serde_json::json!({ "run": {}, "nodes": {} }),
                pending_approvals: Vec::new(),
                deliveries: vec![crate::ports::DeliveryReport {
                    node: "done".to_string(),
                    kind: "channel".to_string(),
                    target: Some("engineering".to_string()),
                    status: crate::ports::DeliveryStatus::Skipped,
                    detail: "this was a test run — nothing was sent".to_string(),
                    reason: crate::ports::DeliveryReason::DryRun,
                }],
                cancelled: false,
                nodes: vec![crate::ports::WorkflowRunNodeRow {
                    node_id: "done".to_string(),
                    status: crate::ports::types::WorkflowNodeStatus::Ok,
                    elapsed_ms: 3,
                    diagnostics: Vec::new(),
                }],
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            })
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRunner for DroppedReportRunner {
        async fn run(
            &self,
            _company: &CompanyId,
            _workflow: &crate::company::WorkflowFile,
            _input: serde_json::Value,
            _ctx: &WorkflowRunContext,
        ) -> crate::Result<WorkflowRun> {
            Ok(WorkflowRun {
                output: serde_json::json!({ "run": {}, "nodes": {} }),
                pending_approvals: Vec::new(),
                deliveries: vec![crate::ports::DeliveryReport {
                    node: "done".to_string(),
                    kind: "channel".to_string(),
                    target: Some("operator".to_string()),
                    status: crate::ports::DeliveryStatus::Failed,
                    detail: "`operator` is not an automation delivery channel".to_string(),
                    reason: crate::ports::DeliveryReason::ChannelNotWired,
                }],
                cancelled: false,
                nodes: vec![crate::ports::WorkflowRunNodeRow {
                    node_id: "done".to_string(),
                    status: crate::ports::types::WorkflowNodeStatus::Ok,
                    elapsed_ms: 3,
                    diagnostics: Vec::new(),
                }],
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            })
        }
    }

    /// A hosted company whose runner echoes immediately.
    async fn echo_company(home: &std::path::Path) -> axum::Router {
        company_with_runner(home, Arc::new(EchoRunner)).await
    }

    /// A hosted company with one overlay graph and the given runner behind
    /// the port, so the route, the supervisor and the journal write are all
    /// production code and only the graph walk is stubbed.
    async fn company_with_runner(
        home: &std::path::Path,
        runner: Arc<dyn WorkflowRunner>,
    ) -> axum::Router {
        let manifest: CompanyManifest =
            toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
        let id = CompanyId::new("acme");
        FsCompanyStore::new(home.to_path_buf())
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                    id: "demo".to_string(),
                    toml: GRAPH.to_string(),
                }],
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                overlay_tool_grants: None,
                overlay_desk_tools: Default::default(),
                disabled_workflows: Vec::new(),
                lifecycle: "running".to_string(),
                template_provenance: None,
                setup: None,
                name_confirmed: false,
                activation_completed_at: None,
                created_at_millis: None,
            })
            .await
            .unwrap();
        let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        runtime.set_workflow_runner(runner);
        let state = AppState::new(AppConfig::default());
        state.registry().insert(id.clone(), Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, "acme").await;
        router(state)
    }

    /// T8 — `{"dry_run":true}` answers 200 carrying `dryRun:true` and the
    /// per-node `nodes`; a plain body carries neither `dryRun` (a real run's
    /// shape an old host would produce) — the presence discriminator the
    /// console reads instead of trusting what it asked for.
    #[tokio::test]
    async fn dry_run_request_echoes_the_marker_and_nodes_a_plain_body_omits_it() {
        let home_dir = home();
        let app = echo_company(home_dir.path()).await;

        let dry = app
            .clone()
            .oneshot(run_request(serde_json::json!({ "dry_run": true })))
            .await
            .unwrap();
        assert_eq!(dry.status(), StatusCode::OK);
        let body = json_body(dry).await;
        assert_eq!(body["dryRun"], serde_json::json!(true), "{body}");
        assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
        assert_eq!(body["nodes"][0]["status"], "ok", "{body}");

        let plain = app
            .oneshot(run_request(serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(plain.status(), StatusCode::OK);
        let body = json_body(plain).await;
        assert!(
            body.get("dryRun").is_none(),
            "a real run must carry no dryRun key: {body}"
        );
        // The node trail rides every settled run, dry or not.
        assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
    }

    // ── Issue #981 (part 2): the run's own verdict ───────────────────────

    /// **The defect, at the HTTP boundary.** A run whose report was refused
    /// answers `200` with every node `ok` and no error — and before this
    /// there was nothing on the body that said otherwise, so a client
    /// folding `nodes[].status` (the QA harness among them) scored it green.
    #[tokio::test]
    async fn a_run_whose_report_was_dropped_does_not_answer_as_a_clean_run() {
        let home_dir = home();
        let app = company_with_runner(home_dir.path(), Arc::new(DroppedReportRunner)).await;

        let response = app
            .oneshot(run_request(serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;

        assert_eq!(body["verdict"], "undelivered", "{body}");

        // …and the three facts the verdict must NOT have disturbed. A
        // delivery failure is not a broken graph: the node ran, so its
        // status stays `ok`, no `error` appears, and the run is not
        // cancelled. Flipping any of them would send the copilot's
        // fix-from-run at a graph that was fine.
        assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
        assert_eq!(body["nodes"][0]["status"], "ok", "{body}");
        assert!(body.get("error").is_none(), "{body}");
        assert!(body.get("cancelled").is_none(), "{body}");
        // The row is still where the *reason* lives; the verdict is the
        // reading.
        assert_eq!(
            body["deliveries"][0]["reason"], "channel-not-wired",
            "{body}"
        );
    }

    /// Issue #981, the second half: a **test run** is not a run that lost
    /// its report.
    ///
    /// `deliver_outputs_dry` writes one `skipped`/`dry-run` row per routed
    /// `output` node, so before this every single test run of a graph with
    /// a destination answered `undelivered` — the console badged the safest
    /// thing an operator can do as a failure, every time. The rows stay on
    /// the body: they are what say *where* the report would have gone.
    ///
    /// Asked for as a **real dry run** (`dry_run: true`) and the `dryRun`
    /// discriminator asserted alongside the verdict, so this cannot pass on
    /// a run that was not one — a stub runner returning a `dry-run` row is
    /// only half the claim.
    #[tokio::test]
    async fn a_test_run_is_not_a_run_that_lost_its_report() {
        let home_dir = home();
        let app = company_with_runner(home_dir.path(), Arc::new(DryRunRunner)).await;

        let response = app
            .oneshot(run_request(serde_json::json!({ "dry_run": true })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;

        assert_eq!(body["verdict"], "ok", "{body}");
        assert_eq!(body["dryRun"], serde_json::json!(true), "{body}");
        assert_eq!(body["deliveries"][0]["reason"], "dry-run", "{body}");
        assert_eq!(body["deliveries"][0]["status"], "skipped", "{body}");
    }

    /// The other direction, which is the one that must not regress: a run
    /// that delivered everything still reads `ok`.
    #[tokio::test]
    async fn a_run_that_delivered_fine_still_answers_ok() {
        let home_dir = home();
        let app = echo_company(home_dir.path()).await;

        let response = app
            .oneshot(run_request(serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["verdict"], "ok", "{body}");
    }
}
