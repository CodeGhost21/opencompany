    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::store::conformance;

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Whether a missing server must FAIL rather than skip. Issue #555.
    ///
    /// The `OPENCOMPANY_TEST_MONGODB_URI` skip above is right for a laptop with
    /// no MongoDB — it keeps a default `cargo test` offline — and wrong for the
    /// CI lane whose entire purpose is running this suite. There, an unset URI
    /// is a misconfigured job, and the skip would report it as a pass: the
    /// whole suite silently absent behind a green tick, which is the exact
    /// defect this lane was added to fix, reintroduced one layer down.
    ///
    /// So CI sets this second variable and nothing else does. Set = the caller
    /// has promised a reachable server, so not finding one is an error.
    ///
    /// `0` and the empty string read as unset, so the variable can be threaded
    /// through a workflow matrix or a shell wrapper that always defines it.
    fn required() -> bool {
        std::env::var("OPENCOMPANY_TEST_MONGODB_REQUIRED")
            .is_ok_and(|value| !value.is_empty() && value != "0")
    }

    /// Issue #697. The partial filter must be keyed on the **field name passed
    /// in**, never on the literal `"present"` — the parameter's own name.
    ///
    /// Raised in review of #733 as a HIGH finding: that `doc!` stringifies
    /// identifier keys, so `doc! {present: ...}` would build
    /// `{"present": {"$exists": true}}`, a filter no document matches. The
    /// index would then be built over an empty set and reject no insert,
    /// silently removing the one-file-per-path guarantee on this backend.
    ///
    /// It does not: the `bson!` key arms end at
    /// `insert::<_, Bson>(($($key)+), $value)`, passing the key tokens as an
    /// expression rather than through `stringify!`. But that is an argument
    /// about a macro's expansion, and the cost of being wrong is a guard that
    /// looks present and enforces nothing — so this asserts the built artifact
    /// instead of the reasoning.
    ///
    /// Needs no server: it inspects the `IndexModel` this code constructs, so
    /// it runs on the `mongodb` feature alone and cannot pass vacuously the way
    /// the URI-gated tests can.
    #[test]
    fn the_partial_filter_is_keyed_on_the_field_not_the_parameter_name() {
        let model = unique_partial(doc! {"company_id": 1, "file_path_key": 1}, "file_path_key");
        let filter = model
            .options
            .as_ref()
            .and_then(|options| options.partial_filter_expression.as_ref())
            .expect("the index is partial");

        assert!(
            filter.contains_key("file_path_key"),
            "the filter must name the field it guards: {filter:?}"
        );
        assert!(
            !filter.contains_key("present"),
            "a filter keyed on the parameter's own name would match no document, so the \
             index would be built over an empty set and reject nothing: {filter:?}"
        );
        assert_eq!(
            filter.get_document("file_path_key").expect("the condition"),
            &doc! {"$exists": true},
            "and the condition is existence of that field: {filter:?}"
        );
        assert!(
            model
                .options
                .as_ref()
                .and_then(|options| options.unique)
                .unwrap_or(false),
            "a partial filter without uniqueness would guard nothing at all"
        );
    }

    /// Issue #759's index, asserted the same way and for the same reason.
    ///
    /// The folder guard is a second `unique_partial`, and a partial filter that
    /// named the wrong field would be the identical silent failure: an index
    /// built over an empty set, rejecting nothing, while every sequential test
    /// still passed. Asserting the constructed `IndexModel` catches that with no
    /// server, so it cannot pass vacuously.
    ///
    /// It also pins the field **name**: the folder key must be its own field,
    /// not `file_path_key`. Sharing one field would make a folder and a file
    /// contend for a single name — a new tree rule this change explicitly does
    /// not introduce.
    #[test]
    fn the_folder_claim_index_is_partial_unique_on_its_own_field() {
        let model = unique_partial(
            doc! {"company_id": 1, "folder_path_key": 1},
            "folder_path_key",
        );
        let filter = model
            .options
            .as_ref()
            .and_then(|options| options.partial_filter_expression.as_ref())
            .expect("the index is partial");

        assert!(
            filter.contains_key("folder_path_key"),
            "the filter must name the field it guards: {filter:?}"
        );
        assert!(
            !filter.contains_key("file_path_key"),
            "the folder guard must not key on the file field, or a folder and a note would \
             contend for one name: {filter:?}"
        );
        assert_eq!(
            filter
                .get_document("folder_path_key")
                .expect("the condition"),
            &doc! {"$exists": true},
        );
        assert!(
            model
                .options
                .as_ref()
                .and_then(|options| options.unique)
                .unwrap_or(false),
            "a partial filter without uniqueness would guard nothing at all"
        );
        // The two keys share an encoding, which is what lets one `path_key`
        // serve both — pinned so a future edit cannot make them silently differ.
        assert_eq!(
            folder_path_key(Some("p"), "task-42"),
            file_path_key(Some("p"), "task-42")
        );
    }

    /// Issue #759, the subtle half: a folder that is **moved** drops its claim.
    ///
    /// `rename_move` has to `$unset` `folder_path_key`, and a missing unset is
    /// invisible until somebody needs the vacated path again. The moved
    /// document would keep guarding the path it left, so the next publish that
    /// wanted `agents/cmo/task-42/` would be refused by an index entry
    /// describing a folder that is no longer there — the permanent outage this
    /// primitive exists to prevent, reintroduced by the fix itself.
    ///
    /// Asserted by reclaiming the old path and checking a *new* folder was
    /// minted there, rather than by reading the document: the claim is only
    /// worth what the next claimer observes.
    #[tokio::test]
    async fn a_moved_folder_releases_its_claim_on_the_path_it_left() {
        use crate::ports::workspace::WorkspaceStore;
        let Some(s) = store().await else { return };
        let company = CompanyId::new("mover");
        let origin = crate::ports::workspace::WorkspaceOrigin::Seed;

        let parent = s
            .adopt_or_create_folder(&company, None, "Agents", origin.clone())
            .await
            .expect("the root")
            .into_node()
            .id;
        let moved = s
            .adopt_or_create_folder(&company, Some(&parent), "task-42", origin.clone())
            .await
            .expect("the folder")
            .into_node()
            .id;

        // The operator renames it out of the way.
        s.rename_move(&company, &moved, Some("task-42-archived"), None)
            .await
            .expect("rename to the workspace root");

        // The vacated path must be claimable again, by a genuinely new folder.
        let reclaimed = s
            .adopt_or_create_folder(&company, Some(&parent), "task-42", origin)
            .await
            .expect("the path the moved folder left must be free");
        assert!(
            reclaimed.was_created(),
            "a stale claim would have made this adopt a folder that is not there"
        );
        assert_ne!(reclaimed.node().id, moved);

        drop_db(&s).await;
    }

    /// **Issue #392 through the port**: the host-durable append asks the server
    /// for `j:true`, and the process-durable one does not.
    ///
    /// `assert_journal_store` cannot catch this — a backend that ignored the
    /// `Durability` argument stores and orders every record identically and
    /// passes the whole suite, silently dropping the guarantee that keeps an
    /// already-fired effect from firing again after a primary crash. So the
    /// constructed handles are asserted directly, the same way
    /// `the_partial_filter_is_keyed_on_the_field_not_the_parameter_name` asserts
    /// a built `IndexModel` rather than the reasoning behind it.
    ///
    /// Needs no server: `Client::with_options` resolves lazily and
    /// `collection_with_options` builds a handle locally, so this runs on the
    /// `mongodb` feature alone and cannot pass vacuously the way the URI-gated
    /// tests can. (It is a `tokio::test` only because the driver's constructor
    /// spawns a cleanup task, not because anything here awaits the network.)
    #[tokio::test]
    async fn only_the_host_durable_journal_write_asks_for_j_true() {
        // A client handle, not a connection: `with_options` resolves lazily and
        // never touches the network, so this stays a pure shape assertion.
        let client = Client::with_options(
            mongodb::options::ClientOptions::builder()
                .hosts(vec![mongodb::options::ServerAddress::Tcp {
                    host: "localhost".into(),
                    port: Some(27017),
                }])
                .build(),
        )
        .expect("build a client handle");
        let store = MongoStore {
            db: client.database("oc_test_shape"),
            senders: Arc::new(StdMutex::new(HashMap::new())),
        };

        let host = store.journaled(JOURNAL);
        assert_eq!(
            host.write_concern().and_then(|concern| concern.journal),
            Some(true),
            "a host-durable record must be committed to the server's journal \
             before the insert is acknowledged"
        );

        let process = store.collection(JOURNAL);
        assert!(
            process
                .write_concern()
                .and_then(|concern| concern.journal)
                .is_none(),
            "the process-durable level must NOT pay a disk flush: these are the \
             journal's highest-volume records, and losing one makes the runtime \
             re-ask rather than re-fire"
        );
    }

    /// The URI with any `user:password@` replaced by `***@`, for the panic
    /// message below.
    ///
    /// The unreachable-server panic names the URI so the failure says *which*
    /// server it could not reach — a bare "connection refused" in a CI log is
    /// most of a debugging session. But a connection string carries its
    /// credentials inline, and a panic lands in the CI log, the terminal
    /// scrollback and any artifact that captures either. CI points at an
    /// unauthenticated localhost, so nothing leaks there; a developer pointing
    /// this suite at a real cluster is the case that would, and that is exactly
    /// when the message is most useful. Redacting keeps the host and port,
    /// which is the part worth printing.
    fn redact_credentials(uri: &str) -> String {
        let Some((scheme, rest)) = uri.split_once("://") else {
            return uri.to_string();
        };
        // Userinfo, when present, precedes the first `/` of the path — so only
        // an `@` before that boundary delimits it. A password may itself
        // contain `@`, so split at the LAST one within the authority.
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(authority_end);
        match authority.rfind('@') {
            Some(at) => format!("{scheme}://***{}{tail}", &authority[at..]),
            None => uri.to_string(),
        }
    }

    #[test]
    fn redaction_keeps_the_host_and_drops_the_credentials() {
        // The CI shape: nothing to redact, nothing changed.
        assert_eq!(
            redact_credentials("mongodb://localhost:27017"),
            "mongodb://localhost:27017"
        );
        // The shape that would leak.
        assert_eq!(
            redact_credentials("mongodb://user:hunter2@cluster.example:27017"),
            "mongodb://***@cluster.example:27017"
        );
        // A password containing `@` — splitting at the FIRST one would leave
        // the tail of the password in the message.
        assert_eq!(
            redact_credentials("mongodb://user:p@ss@cluster.example:27017"),
            "mongodb://***@cluster.example:27017"
        );
        // An `@` in the path or query must not be mistaken for userinfo.
        assert_eq!(
            redact_credentials("mongodb://localhost:27017/db?replicaSet=a@b"),
            "mongodb://localhost:27017/db?replicaSet=a@b"
        );
        // A credentialed URI that also carries a path keeps the path.
        assert_eq!(
            redact_credentials("mongodb+srv://u:p@host/admin?retryWrites=true"),
            "mongodb+srv://***@host/admin?retryWrites=true"
        );
        // Not a URI at all: returned untouched rather than mangled.
        assert_eq!(redact_credentials("localhost:27017"), "localhost:27017");
    }

    async fn store() -> Option<Arc<MongoStore>> {
        let uri = match std::env::var("OPENCOMPANY_TEST_MONGODB_URI") {
            Ok(uri) => uri,
            Err(_) => {
                assert!(
                    !required(),
                    "OPENCOMPANY_TEST_MONGODB_REQUIRED is set but \
                     OPENCOMPANY_TEST_MONGODB_URI is not. This lane exists to run the \
                     MongoDB conformance suite against a real server, so a skip here is \
                     a misconfigured job rather than a pass — point the URI at the \
                     service container."
                );
                eprintln!("skipping: OPENCOMPANY_TEST_MONGODB_URI is not set");
                return None;
            }
        };
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        let db = format!(
            "oc_test_{}_{}_{}",
            std::process::id(),
            nonce,
            DB_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        // `connect` creates indexes, so it round-trips to the server rather
        // than resolving lazily: an unreachable host fails HERE, after the
        // driver's server-selection timeout, instead of much later inside
        // whichever assertion happened to touch the database first.
        let store = MongoStore::connect(&uri, &db).await.unwrap_or_else(|err| {
            panic!(
                "could not reach the MongoDB server at {}: {err}",
                redact_credentials(&uri)
            )
        });
        Some(Arc::new(store))
    }

    /// One failing index must not stop the other forty-three from being
    /// created, **nor stop the store from opening at all**.
    ///
    /// Two properties in one test, because they have the same setup.
    ///
    /// `ensure_indexes` runs concurrently, so a short-circuit would drop
    /// in-flight driver operations mid-await — and an operation cancelled after
    /// its request is sent but before its reply is read leaves a connection the
    /// pool cannot safely reuse. That hazard does not exist in a sequential
    /// loop; it arrived with the concurrency, so it is pinned here.
    ///
    /// And the failure is now reported by logging rather than by refusing to
    /// construct the store (#1716). Returning `Err` here took the whole process
    /// down; under the microVM runtime the workload is PID 1, so that panicked
    /// the guest kernel and the tenant was gone. Indexes are a performance
    /// property, not a precondition for the process existing.
    #[tokio::test]
    async fn a_failing_index_stops_neither_the_other_indexes_nor_the_boot() {
        let Some(store) = store().await else { return };

        // `store()` has already run `ensure_indexes` once, so the assertion has
        // to be about something this run RE-creates. Drop a known index first;
        // if the pipeline short-circuits before reaching it, it stays missing.
        store
            .collection("notifications")
            .drop_index("company_id_1_id_1")
            .await
            .expect("drop the index whose return proves the run continued");

        // Induce the failure the way MongoDB actually produces one: an existing
        // index on the same keys with conflicting options. Replacing the unique
        // `owners` index with a non-unique one makes `ensure_indexes` collide.
        store
            .collection("owners")
            .drop_index("company_id_1")
            .await
            .expect("drop owners index");
        store
            .collection("owners")
            .create_index(IndexModel::builder().keys(doc! {"company_id": 1}).build())
            .await
            .expect("seed the conflicting index");

        // The store must still open. A tenant that cannot boot because one
        // index conflicts is strictly worse than one serving without it.
        store
            .ensure_indexes()
            .await
            .expect("a conflicting index must not stop the store from opening");

        // The point: the run continued past the failure and recreated the index
        // dropped above. A short-circuit would leave it absent.
        let mut names = Vec::new();
        let mut cursor = store
            .collection("notifications")
            .list_indexes()
            .await
            .expect("list notifications indexes");
        while cursor.advance().await.expect("advance") {
            if let Some(name) = cursor
                .deserialize_current()
                .expect("model")
                .options
                .and_then(|o| o.name)
            {
                names.push(name);
            }
        }
        assert!(
            names.iter().any(|n| n == "company_id_1_id_1"),
            "a failure on `owners` must not stop other indexes being created; got {names:?}"
        );

        drop_db(&store).await;
    }

    /// Issue #1573: the backfill copies `agentId` out of `run_json` for rows
    /// written before the mirror column existed, and does so in bounded batches
    /// that re-probe between passes rather than holding the whole collection.
    ///
    /// Seeded directly into the `runs` collection with no `agent_id` field —
    /// the exact shape a row predating the upgrade has — because it is not
    /// reachable through the port: `create_run`/`put_run` always write the
    /// mirror. The store is built as a bare struct, not through `connect`, so
    /// no background backfill task shares the database with this one's
    /// assertions.
    #[tokio::test]
    async fn backfill_fills_legacy_run_rows_in_bounded_batches() {
        let uri = match std::env::var("OPENCOMPANY_TEST_MONGODB_URI") {
            Ok(uri) => uri,
            Err(_) => {
                assert!(
                    !required(),
                    "OPENCOMPANY_TEST_MONGODB_REQUIRED is set but \
                     OPENCOMPANY_TEST_MONGODB_URI is not"
                );
                eprintln!("skipping: OPENCOMPANY_TEST_MONGODB_URI is not set");
                return;
            }
        };
        let client = Client::with_uri_str(&uri).await.unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        let db_name = format!(
            "oc_test_{}_{}_{}",
            std::process::id(),
            nonce,
            DB_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let store = MongoStore {
            db: client.database(&db_name),
            senders: Arc::new(StdMutex::new(HashMap::new())),
        };

        let company = CompanyId::new("legacy-co");
        let runs = store.collection("runs");
        // More than one batch, so the re-probe loop is exercised — the rows past
        // the first `BACKFILL_BATCH_SIZE` must be picked up by a later pass.
        for i in 0..BACKFILL_BATCH_SIZE + 3 {
            let agent = if i % 2 == 0 { "engineer" } else { "designer" };
            let record = crate::ports::runs::RunRecord {
                id: format!("legacy-{i}"),
                company: company.clone(),
                task_id: None,
                agent_id: agent.to_string(),
                chat_id: None,
                thread_root: None,
                workflow_run_id: None,
                node_id: None,
                attempt: 1,
                status: crate::ports::runs::RunStatus::Succeeded,
                trigger_event_seq: None,
                created_at_millis: 1_700_000_000_000,
                started_at_millis: None,
                finished_at_millis: None,
                error: None,
                usage: Default::default(),
                step_count: 1,
            };
            runs.insert_one(doc! {
                "company_id": company.as_ref(),
                "run_id": &record.id,
                "run_json": serde_json::to_string(&record).unwrap(),
            })
            .await
            .unwrap();
        }
        // A row that already carries the mirror (written through the port after
        // the upgrade) must be neither touched nor counted.
        let fresh = crate::ports::runs::RunRecord {
            id: "fresh".to_string(),
            company: company.clone(),
            task_id: None,
            agent_id: "engineer".to_string(),
            chat_id: None,
            thread_root: None,
            workflow_run_id: None,
            node_id: None,
            attempt: 1,
            status: crate::ports::runs::RunStatus::Pending,
            trigger_event_seq: None,
            created_at_millis: 1_700_000_000_000,
            started_at_millis: None,
            finished_at_millis: None,
            error: None,
            usage: Default::default(),
            step_count: 0,
        };
        runs.insert_one(doc! {
            "company_id": company.as_ref(),
            "run_id": &fresh.id,
            "agent_id": "engineer",
            "status": "pending",
            "attempt": 1i64,
            "created_ms": 1_700_000_000_000i64,
            "run_json": serde_json::to_string(&fresh).unwrap(),
        })
        .await
        .unwrap();

        let filled = store.backfill_run_agent_ids().await.unwrap();
        assert_eq!(
            filled,
            BACKFILL_BATCH_SIZE + 3,
            "every legacy row is filled; the fresh row is not counted"
        );

        // The mirror landed on disk, not just in the return value — one row from
        // each batch's worth of desks.
        let migrated = runs
            .find_one(doc! {"run_id": "legacy-0"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(get_str(&migrated, "agent_id").unwrap(), "engineer");
        let later = runs
            .find_one(doc! {"run_id": "legacy-1"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(get_str(&later, "agent_id").unwrap(), "designer");

        // A second pass has nothing left to do — the `$exists: false` probe is
        // exhausted.
        assert_eq!(store.backfill_run_agent_ids().await.unwrap(), 0);

        drop_db(&store).await;
    }

    async fn drop_db(store: &MongoStore) {
        let _ = store.db.drop().await;
    }

    /// Ages every staged blob past [`ORPHAN_BLOB_MIN_AGE_MS`] (issue #664).
    ///
    /// The sweep only reclaims blobs old enough to be abandoned, so a test that
    /// uploads bytes and reboots in the same millisecond is staging an
    /// *in-flight* upload, not an orphaned one. Rewriting `uploadDate` is how a
    /// test says "and then an hour passed" without sleeping for one.
    async fn age_blobs_past_the_sweep_threshold(store: &MongoStore) {
        let old = mongodb::bson::DateTime::from_millis(
            now_millis() as i64 - ORPHAN_BLOB_MIN_AGE_MS - 60_000,
        );
        store
            .db
            .collection::<Document>(&format!("{BLOB_BUCKET}.files"))
            .update_many(doc! {}, doc! { "$set": { "uploadDate": old } })
            .await
            .expect("backdate staged blobs");
    }

    /// The boot sweep reclaims a payload whose node document never landed.
    ///
    /// This is the crash the write ordering deliberately allows: blob first,
    /// document second, so an interrupted `create_binary` leaves bytes nothing
    /// references. Seeded here directly — uploading to the bucket without ever
    /// inserting the node — because that is precisely the state a crash between
    /// the two writes produces, and it is not reachable through the port.
    ///
    /// The node-backed blob beside it is the half that must be left alone: a
    /// sweep that reclaimed live payloads would be far worse than the leak it
    /// fixes.
    #[tokio::test]
    async fn the_boot_sweep_reclaims_orphaned_blobs_and_spares_live_ones() {
        let Some(s) = store().await else { return };
        let company = CompanyId::new("sweep-co");

        // A live binary node, written through the port.
        let node = crate::ports::workspace::WorkspaceNode {
            id: "keep".to_string(),
            name: "keep.png".to_string(),
            kind: crate::ports::workspace::NodeKind::File,
            parent_id: None,
            updated_at_millis: now_millis(),
            created_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            updated_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            mime: Some("image/png".to_string()),
            size: None,
            sha256: None,
            adopted: false,
        };
        crate::ports::workspace::WorkspaceStore::create_binary(&*s, &company, &node, b"live-bytes")
            .await
            .unwrap();

        // …and a dangling one: the bytes of an interrupted create.
        s.put_blob(&company, "vanished", "ghost.png", b"orphan-bytes")
            .await
            .unwrap();
        // A blob with no metadata at all — a shape this store never writes, and
        // therefore unmatchable to any node, so it is an orphan by definition.
        {
            use futures::io::AsyncWriteExt;
            let mut up = s
                .blobs()
                .open_upload_stream("nometa.bin")
                .await
                .expect("upload");
            up.write_all(b"no-metadata").await.unwrap();
            up.close().await.unwrap();
        }

        let before = s.blobs().find(doc! {}).await.unwrap();
        assert_eq!(
            before.try_collect::<Vec<_>>().await.unwrap().len(),
            3,
            "two orphans and one live payload are staged"
        );

        // The orphans have to be *old* to be orphans (issue #664). Staged
        // seconds ago they are indistinguishable from a peer's in-flight
        // upload, and the sweep now spares them for that reason — see
        // `a_recent_orphan_is_left_alone_because_it_may_be_an_in_flight_upload`.
        age_blobs_past_the_sweep_threshold(&s).await;

        // Constructing a store over the same database runs the sweep.
        let rebooted = MongoStore::from_database(s.db.clone()).await.unwrap();

        let files = rebooted
            .blobs()
            .find(doc! {})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(files.len(), 1, "both orphans are reclaimed");
        assert_eq!(files[0].filename.as_deref(), Some("keep.png"));

        // The live node still serves its bytes — the sweep did not touch it.
        let (_, stream) =
            crate::ports::workspace::WorkspaceStore::read_bytes(&rebooted, &company, "keep")
                .await
                .unwrap()
                .expect("the live payload survives the sweep");
        let mut got = Vec::new();
        {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.unwrap());
            }
        }
        assert_eq!(got, b"live-bytes".to_vec());

        drop_db(&s).await;
    }

    /// Issue #664: the boot sweep must not delete a concurrent writer's upload.
    ///
    /// The state staged here is not a crash — it is the perfectly ordinary
    /// instant *inside* a healthy `create_binary`, after `put_blob` has
    /// returned and before the node insert lands. A second process booting then
    /// (a rolling deploy, a restarted replica, another tenant's container on a
    /// shared database) sees a blob with no node and, before this fix, reclaimed
    /// it. The writer's insert would then land on top, leaving a node whose
    /// download 404s forever and whose `size` still counts against the quota —
    /// unreclaimable, because the sweep only deletes blobs without nodes and
    /// never nodes without blobs.
    ///
    /// Deliberately *not* backdated: recency is the whole signal.
    #[tokio::test]
    async fn a_recent_orphan_is_left_alone_because_it_may_be_an_in_flight_upload() {
        let Some(s) = store().await else { return };
        let company = CompanyId::new("inflight-co");

        // Exactly what a concurrent `create_binary` has written so far.
        s.put_blob(&company, "arriving", "arriving.png", b"in-flight-bytes")
            .await
            .unwrap();

        // A second process boots against the same database and sweeps.
        let rebooted = MongoStore::from_database(s.db.clone()).await.unwrap();

        let files = rebooted
            .blobs()
            .find(doc! {})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            files.len(),
            1,
            "a blob younger than the threshold is an upload in progress, not an orphan"
        );
        assert_eq!(files[0].filename.as_deref(), Some("arriving.png"));

        // And the writer's insert, landing after the sweep, yields a node whose
        // bytes are actually there — the whole point.
        let node = crate::ports::workspace::WorkspaceNode {
            id: "arriving".to_string(),
            name: "arriving.png".to_string(),
            kind: crate::ports::workspace::NodeKind::File,
            parent_id: None,
            updated_at_millis: now_millis(),
            created_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            updated_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            mime: Some("image/png".to_string()),
            size: None,
            sha256: None,
            adopted: false,
        };
        rebooted
            .collection("workspace_nodes")
            .insert_one(doc! {
                "company_id": company.as_ref(),
                "node_id": &node.id,
                "node_json": serde_json::to_string(&node).unwrap(),
                "content": "",
                "updated_ms": node.updated_at_millis as i64,
            })
            .await
            .unwrap();

        let (_, stream) =
            crate::ports::workspace::WorkspaceStore::read_bytes(&rebooted, &company, "arriving")
                .await
                .unwrap()
                .expect("the upload that was in flight during the sweep still has its bytes");
        let mut got = Vec::new();
        {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.unwrap());
            }
        }
        assert_eq!(got, b"in-flight-bytes".to_vec());

        drop_db(&s).await;
    }

    /// A `create_binary` name conflict must not strand the payload it just
    /// uploaded.
    ///
    /// This backend writes blob-first (issue #894), so a sibling-name collision
    /// is detected only when the node-document insert fails — by which time the
    /// bytes are already in GridFS. Before the conflict path reclaimed them, the
    /// error returned with that blob still present: no node document referenced
    /// it, and the boot sweep (which runs only at store construction, and only
    /// for blobs older than an hour) was the sole reclaim path. The
    /// chat-attachment flow reaches this branch every time a repeated filename
    /// is disambiguated and retried, so a long-lived tenant would accumulate
    /// invisible GridFS copies. The conflict path must own the payload it
    /// uploaded before the caller learns of the conflict.
    #[tokio::test]
    async fn a_name_conflict_reclaims_the_blob_it_just_uploaded() {
        let Some(s) = store().await else { return };
        let company = CompanyId::new("conflict-co");

        let first = crate::ports::workspace::WorkspaceNode {
            id: "winner".to_string(),
            name: "image.png".to_string(),
            kind: crate::ports::workspace::NodeKind::File,
            parent_id: None,
            updated_at_millis: now_millis(),
            created_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            updated_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            mime: Some("image/png".to_string()),
            size: None,
            sha256: None,
            adopted: false,
        };
        crate::ports::workspace::WorkspaceStore::create_binary(&*s, &company, &first, b"first")
            .await
            .unwrap();

        let loser = crate::ports::workspace::WorkspaceNode {
            id: "loser".to_string(),
            ..first.clone()
        };
        let err = crate::ports::workspace::WorkspaceStore::create_binary(
            &*s, &company, &loser, b"second",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, crate::error::OpenCompanyError::Conflict(_)),
            "a taken sibling name is a Conflict, not a storage fault: {err:?}"
        );

        // The loser's payload must not survive the conflict as an orphan.
        let files = s
            .blobs()
            .find(MongoStore::blob_filter(&company, "loser"))
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            files.len(),
            0,
            "the conflicted upload leaves no orphan blob for the sweep to find later"
        );

        // The winner still serves its bytes.
        let (_, stream) =
            crate::ports::workspace::WorkspaceStore::read_bytes(&*s, &company, "winner")
                .await
                .unwrap()
                .expect("the winner's payload is untouched by the refusal");
        let mut got = Vec::new();
        {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.unwrap());
            }
        }
        assert_eq!(got, b"first".to_vec());

        drop_db(&s).await;
    }

    /// A same-id `create_binary` race must not reclaim the winning payload.
    ///
    /// Two callers racing the same fresh node id both pass the `contains_key`
    /// pre-check (neither document is visible when the reads land), both upload
    /// a GridFS payload under that id, and the node-document insert then loses
    /// on the unique `(company_id, node_id)` index for exactly one of them.
    /// The loser's conflict cleanup used to sweep *every* blob for the id —
    /// including the winner's, the bytes its live node now points at — leaving
    /// the download irrecoverable on hosted MongoDB deployments. The cleanup
    /// must name and delete only the upload this losing call made.
    ///
    /// Spawned rather than staged: the whole point is the interleaving, and the
    /// runtime guarantees it here — each racer awaits a database read before
    /// either inserts, so both `contains_key` checks necessarily see the empty
    /// tree regardless of which document insert finally wins.
    #[tokio::test]
    async fn a_same_id_race_preserves_the_winning_payload() {
        let Some(s) = store().await else { return };
        let company = CompanyId::new("dupe-race-co");

        let node = crate::ports::workspace::WorkspaceNode {
            id: "dupe".to_string(),
            name: "dupe.png".to_string(),
            kind: crate::ports::workspace::NodeKind::File,
            parent_id: None,
            updated_at_millis: now_millis(),
            created_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            updated_by: crate::ports::workspace::WorkspaceOrigin::Operator,
            mime: Some("image/png".to_string()),
            size: None,
            sha256: None,
            adopted: false,
        };

        let racer_a = {
            let s = s.clone();
            let company = company.clone();
            let node = node.clone();
            tokio::spawn(async move {
                crate::ports::workspace::WorkspaceStore::create_binary(
                    &*s,
                    &company,
                    &node,
                    b"payload-a",
                )
                .await
            })
        };
        let racer_b = {
            let s = s.clone();
            let company = company.clone();
            let node = node.clone();
            tokio::spawn(async move {
                crate::ports::workspace::WorkspaceStore::create_binary(
                    &*s,
                    &company,
                    &node,
                    b"payload-b",
                )
                .await
            })
        };

        let (outcome_a, outcome_b) = (racer_a.await.unwrap(), racer_b.await.unwrap());
        assert_eq!(
            outcome_a.is_ok() as u8 + outcome_b.is_ok() as u8,
            1,
            "exactly one same-id caller wins the insert; the other must be a Conflict"
        );

        // The survivor's node still serves its bytes: the loser's cleanup
        // deleted only its own upload, never the winner's.
        let (_, stream) =
            crate::ports::workspace::WorkspaceStore::read_bytes(&*s, &company, "dupe")
                .await
                .unwrap()
                .expect("the winning payload survives the same-id race");
        let mut got = Vec::new();
        {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.unwrap());
            }
        }
        assert!(
            got == b"payload-a" || got == b"payload-b",
            "the surviving payload is exactly one racer's, not a mix: {got:?}"
        );

        // And exactly one blob remains for the id — the losing upload is gone,
        // the winner's is untouched.
        let files = s
            .blobs()
            .find(MongoStore::blob_filter(&company, "dupe"))
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(
            files.len(),
            1,
            "the losing upload is reclaimed without touching the winner's"
        );

        drop_db(&s).await;
    }

    /// Issue #1077: the orphan report composes `list()` and `owners()`
    /// correctly against a real server.
    ///
    /// The pure set difference is unit-tested in `app::orphans`. What only a
    /// live backend can prove is that the two reads are *comparable* — that
    /// `owners()` keys on the same id string `list()` returns. They do
    /// (`company_id` in both collections), but nothing in the type system says
    /// so: both sides are `CompanyId`, and if one had been namespaced and the
    /// other bare, the report would have called every company on the platform
    /// an orphan while still type-checking and still passing every unit test.
    ///
    /// Namespaced ids specifically, because that is the only mode in which the
    /// `owners` collection is load-bearing at all.
    #[tokio::test]
    async fn orphaned_companies_are_found_through_the_real_ports() {
        let Some(s) = store().await else { return };

        let manifest: CompanyManifest = toml::from_str("[company]\nname = \"Acme\"\n").unwrap();
        let owned = crate::app::namespace_company_id("tenant-a", CompanyId::new("owned"));
        let orphan = crate::app::namespace_company_id("tenant-a", CompanyId::new("orphan"));

        for id in [&owned, &orphan] {
            let record = CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".into(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_agent_edits: Vec::new(),
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
            s.save(&record).await.expect("save company");
        }
        // Only one of the two gets an owner row. The other is exactly the state
        // a failed `set_owner` used to leave behind before #1050 was fixed.
        s.set_owner(&owned, "tenant-a").await.expect("record owner");
        // ...plus a row naming a company that was never saved, which is the
        // benign direction #1073 deliberately prefers on a rolled-back provision.
        let ghost = crate::app::namespace_company_id("tenant-a", CompanyId::new("ghost"));
        s.set_owner(&ghost, "tenant-a").await.expect("record ghost");

        let companies = CompanyStore::list(s.as_ref()).await.expect("list");
        let owners = s.owners().await.expect("owners");
        let report = crate::app::find_orphans(&companies, &owners);

        let unowned: Vec<&str> = report.unowned.iter().map(|c| c.id.as_ref()).collect();
        assert!(
            unowned.contains(&orphan.as_ref()),
            "the company with no owner row must be reported: {report:?}"
        );
        assert!(
            !unowned.contains(&owned.as_ref()),
            "the company WITH an owner row must not be: {report:?}"
        );
        let dangling: Vec<&str> = report.dangling.iter().map(|r| r.id.as_ref()).collect();
        assert!(
            dangling.contains(&ghost.as_ref()),
            "the owner row naming no company must be reported: {report:?}"
        );
        assert!(
            !dangling.contains(&owned.as_ref()),
            "a row whose company exists must not be: {report:?}"
        );

        drop_db(&s).await;
    }

    /// Shared-single-DB namespacing: two tenants registering the same template
    /// name land distinct namespaced ids in one database, so the `companies`
    /// unique index never conflicts, and the `owners` rows carry the right
    /// tenant for each. Mirrors what the workload does when
    /// `OPENCOMPANY_TENANT_ID` is set (see `AppConfig::namespaced_company_id`).
    #[tokio::test]
    async fn shared_db_namespaced_companies_do_not_conflict() {
        let Some(s) = store().await else { return };

        let manifest: CompanyManifest = toml::from_str("[company]\nname = \"Acme\"\n").unwrap();
        let id_a = crate::app::namespace_company_id(
            "tenant-a",
            crate::runtime::company_id_from_name(&manifest.company.name),
        );
        let id_b = crate::app::namespace_company_id(
            "tenant-b",
            crate::runtime::company_id_from_name(&manifest.company.name),
        );
        assert_eq!(id_a.as_ref(), "tenant-a--acme");
        assert_eq!(id_b.as_ref(), "tenant-b--acme");

        for (id, tenant) in [(&id_a, "tenant-a"), (&id_b, "tenant-b")] {
            let record = CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".into(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_agent_edits: Vec::new(),
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
            // Same template name under two tenants: distinct namespaced ids, no
            // `companies` unique-index conflict.
            s.save(&record).await.expect("save namespaced company");
            s.set_owner(id, tenant).await.expect("record owner");
        }

        let mut owners = s.owners().await.unwrap();
        owners.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
        assert_eq!(
            owners,
            vec![
                (id_a.clone(), "tenant-a".to_string()),
                (id_b.clone(), "tenant-b".to_string()),
            ]
        );

        // Both companies remain addressable and carry the shared template name.
        assert_eq!(
            s.load(&id_a).await.unwrap().unwrap().manifest.company.name,
            "Acme"
        );
        assert_eq!(
            s.load(&id_b).await.unwrap().unwrap().manifest.company.name,
            "Acme"
        );

        drop_db(&s).await;
    }
