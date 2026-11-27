use super::*;
use crate::engine::lifecycle::LifecycleUpdate;
use crate::engine::snapshot::{load_snapshot, set_job_state};
use crate::models::Playlist;
use async_trait::async_trait;
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

pub(super) fn crash_point(point: &str) {
    if std::env::var("SOUNDSHIFT_UNIT_CRASH_POINT").ok().as_deref() == Some(point) {
        // Test-only code in this unit-test executable; never compiled into the app.
        std::process::exit(73);
    }
}

struct FakeProvider {
    calls: AtomicUsize,
    entered: Notify,
    release: Semaphore,
    gated: bool,
    gate_canary: bool,
    revision: Option<String>,
    session_error: Option<crate::providers::error::ProviderError>,
    result: Result<(), String>,
    remote_file: Option<PathBuf>,
    read_ids: Mutex<Vec<Vec<String>>>,
    outcome: Option<crate::providers::contracts::MutationOutcome>,
    check_before: Option<Arc<Mutex<Connection>>>,
}

impl FakeProvider {
    fn new(gated: bool, result: Result<(), String>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Semaphore::new(0),
            gated,
            gate_canary: false,
            revision: None,
            session_error: None,
            result,
            remote_file: None,
            read_ids: Mutex::new(Vec::new()),
            outcome: None,
            check_before: None,
        }
    }
    async fn send(&self) -> Result<(), String> {
        if let Some(db) = &self.check_before {
            let saved:bool = db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM transfer_operations WHERE before_json IS NOT NULL AND sent_at IS NOT NULL AND evidence_version=2)",[],|r|r.get(0)).unwrap();
            assert!(
                saved,
                "before-state must be durable before outbound mutation"
            );
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(path) = &self.remote_file {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "accepted_offline_mutation").unwrap();
            file.sync_all().unwrap();
            crash_point("send");
        }
        self.entered.notify_one();
        if self.gated {
            self.release.acquire().await.unwrap().forget();
        }
        self.result.clone()
    }
}

#[async_trait]
impl MusicProvider for FakeProvider {
    async fn playlist_revision(
        &self,
        _: &str,
    ) -> Result<Option<String>, crate::providers::error::ProviderError> {
        Ok(self.revision.clone())
    }
    async fn check_session(&self) -> Result<(), crate::providers::error::ProviderError> {
        if let Some(error) = &self.session_error {
            return Err(error.clone());
        }
        self.canary()
            .await
            .map_err(crate::providers::error::ProviderError::before_send)
    }
    async fn canary(&self) -> Result<(), String> {
        if self.gate_canary {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
        Ok(())
    }
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        Ok(vec![])
    }
    async fn get_playlist_tracks(
        &self,
        _: &str,
        _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        let mut reads = self.read_ids.lock().unwrap();
        let ids = if reads.len() > 1 {
            reads.remove(0)
        } else {
            reads.first().cloned().unwrap_or_default()
        };
        Ok(ids.into_iter().map(|id| serde_json::from_value(serde_json::json!({"id":id,"title":"fixture","artists":[],"duration_ms":0,"is_explicit":false,"is_playable":true})).unwrap()).collect())
    }
    async fn search_track(&self, _: &str) -> Result<Vec<SourceTrack>, String> {
        Ok(vec![])
    }
    async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
        self.send().await?;
        Ok("created_target".into())
    }
    async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        self.send().await
    }
    async fn add_occurrences(
        &self,
        _: &str,
        ids: &[String],
    ) -> Result<crate::providers::contracts::MutationOutcome, crate::providers::error::ProviderError>
    {
        use crate::providers::{contracts::*, error::*};
        self.send().await.map_err(|error| {
            if error.contains("429") {
                ProviderError::http(429, &reqwest::header::HeaderMap::new())
            } else {
                ProviderError::new(
                    ProviderErrorKind::Timeout,
                    SendCertainty::MayHaveApplied,
                    &error,
                )
            }
        })?;
        let outcome = self.outcome.clone().unwrap_or_else(|| MutationOutcome {
            occurrences: ids
                .iter()
                .enumerate()
                .map(|(index, id)| OccurrenceOutcome {
                    index,
                    track_id: id.clone(),
                    status: OccurrenceStatus::Accepted,
                    remote_id: None,
                })
                .collect(),
            revision: None,
        });
        let mut reads = self.read_ids.lock().unwrap();
        if reads.is_empty() {
            reads.push(Vec::new());
        }
        reads.last_mut().unwrap().extend(
            outcome
                .occurrences
                .iter()
                .filter(|o| o.status == OccurrenceStatus::Accepted)
                .map(|o| o.track_id.clone()),
        );
        Ok(outcome)
    }
    async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        panic!("No rollback should be dispatched in these tests")
    }
    async fn delete_playlist(&self, _: &str) -> Result<(), String> {
        panic!("No playlist deletion should be dispatched in these tests")
    }
}

fn fixture() -> (Arc<Mutex<Connection>>, TransferControl) {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    create_pre_mutation_snapshot(
        &conn,
        "worker_job",
        "spotify",
        "ytmusic",
        "source",
        "target",
        "target",
        false,
        1,
        1,
        vec![],
        vec![],
    )
    .unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = TransferControl::new("worker_job".into(), db.clone(), |_| {});
    control.start().unwrap();
    (db, control)
}

fn spawn_add(
    db: Arc<Mutex<Connection>>,
    control: TransferControl,
    provider: Arc<FakeProvider>,
) -> tokio::task::JoinHandle<Result<bool, String>> {
    tokio::spawn(async move {
        journaled_add(
            &db,
            provider.as_ref(),
            "worker_job",
            "target",
            0,
            &["track".into()],
            &AtomicU32::new(2),
            &control,
        )
        .await
    })
}

async fn entered(provider: &FakeProvider) {
    tokio::time::timeout(Duration::from_secs(2), provider.entered.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn pause_before_dispatch_prevents_send_and_cancel_wakes_worker() {
    let (db, control) = fixture();
    control.pause().unwrap();
    let provider = Arc::new(FakeProvider::new(false, Ok(())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    tokio::task::yield_now().await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    control.cancel().unwrap();
    assert!(!task.await.unwrap().unwrap());
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn write_checkpoint_precedes_pause_acknowledgment() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(true, Ok(())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    entered(&provider).await;
    assert_eq!(control.pause().unwrap().state, JobState::Pausing);
    provider.release.add_permits(1);
    assert!(task.await.unwrap().unwrap());
    assert_eq!(control.snapshot().unwrap().state, JobState::Paused);
    assert_eq!(
        load_snapshot(&db.lock().unwrap(), "snap_worker_job")
            .unwrap()
            .added_track_ids,
        vec!["track"]
    );
}

#[tokio::test]
async fn cancellation_during_write_preserves_confirmed_work_before_ack() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(true, Ok(())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    entered(&provider).await;
    assert_eq!(control.cancel().unwrap().state, JobState::Cancelling);
    provider.release.add_permits(1);
    assert!(task.await.unwrap().unwrap());
    assert_eq!(control.snapshot().unwrap().state, JobState::Cancelling);
    assert_eq!(
        load_snapshot(&db.lock().unwrap(), "snap_worker_job")
            .unwrap()
            .added_track_ids
            .len(),
        1
    );
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn checkpoint_failure_overrides_pending_cancellation() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(true, Ok(())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    entered(&provider).await;
    control.cancel().unwrap();
    db.lock().unwrap().execute_batch("CREATE TRIGGER fail_checkpoint BEFORE UPDATE ON transfer_operations BEGIN SELECT RAISE(ABORT, 'checkpoint unavailable'); END;").unwrap();
    provider.release.add_permits(1);
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        control.snapshot().unwrap().state,
        JobState::RecoveryRequired
    );
    assert!(control.finish().await.is_err());
    let status: String = db
        .lock()
        .unwrap()
        .query_row("SELECT status FROM transfer_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "pending");
}

#[tokio::test]
async fn timeout_is_not_replayed_or_reported_cancelled() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(true, Err("request timeout".into())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    entered(&provider).await;
    control.cancel().unwrap();
    provider.release.add_permits(1);
    assert!(task.await.unwrap().is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        control.snapshot().unwrap().state,
        JobState::RecoveryRequired
    );
    let status: String = db
        .lock()
        .unwrap()
        .query_row("SELECT status FROM transfer_operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "ambiguous");
}

#[tokio::test]
async fn aborting_worker_during_send_requires_recovery() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(true, Ok(())));
    let task = spawn_add(db, control.clone(), provider.clone());
    entered(&provider).await;
    task.abort();
    assert!(task.await.is_err());
    assert_eq!(
        control.snapshot().unwrap().state,
        JobState::RecoveryRequired
    );
}

#[tokio::test]
async fn retry_backoff_can_be_cancelled_without_another_send() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(false, Err("429 rate limited".into())));
    let task = spawn_add(db, control.clone(), provider.clone());
    entered(&provider).await;
    control.cancel().unwrap();
    assert!(!tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn creation_is_reserved_and_identity_checkpointed_before_cancel_ack() {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = TransferControl::new("create_job".into(), db.clone(), |_| {});
    let provider = Arc::new(FakeProvider::new(true, Ok(())));
    let worker_db = db.clone();
    let worker_control = control.clone();
    let worker_provider = provider.clone();
    let config = BatchTransferConfig {
        job_id: "create_job".into(),
        source_service: "spotify".into(),
        target_service: "ytmusic".into(),
        source_playlist_name: "source".into(),
        target_playlist_id: None,
        target_playlist_name: None,
        is_new_playlist: true,
        tracks: vec![],
        concurrency: 1,
        skip_duplicates: false,
    };
    let task = tokio::spawn(async move {
        prepare_transfer(
            &worker_db,
            worker_provider.as_ref(),
            &config,
            &worker_control,
        )
        .await
    });
    entered(&provider).await;
    assert_eq!(control.pause().unwrap().state, JobState::Pausing);
    control.cancel().unwrap();
    provider.release.add_permits(1);
    let (_, id, _) = task.await.unwrap().unwrap();
    assert_eq!(id, "created_target");
    let persisted: String = db
        .lock()
        .unwrap()
        .query_row("SELECT target_playlist_id FROM transfer_jobs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(persisted, id);
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn intent_storage_failure_sends_nothing() {
    let (db, control) = fixture();
    db.lock()
        .unwrap()
        .execute_batch("DROP TABLE transfer_operations")
        .unwrap();
    let provider = Arc::new(FakeProvider::new(false, Ok(())));
    assert!(spawn_add(db, control, provider.clone())
        .await
        .unwrap()
        .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn actual_log_and_persisted_error_boundaries_redact_secrets() {
    let (db, _) = fixture();
    let sensitive = "Cookie: sp_dc=cookie_secret\nAuthorization: Bearer bearer_secret";
    dispatch_transfer_log(
        TransferLogPayload {
            job_id: "worker_job".into(),
            timestamp: 1,
            worker_id: 1,
            message: sensitive.into(),
            level: "error".into(),
        },
        |log| {
            assert!(!log.message.contains("cookie_secret"));
            assert!(!log.message.contains("bearer_secret"));
        },
    );
    let conn = db.lock().unwrap();
    let id = record_operation_intent(&conn, "worker_job", 0, &["track".into()]).unwrap();
    update_operation_outcome(&conn, &id, OperationOutcome::Failed, 1, Some(sensitive)).unwrap();
    set_job_state(&conn, "worker_job", "recovery_required", Some(sensitive)).unwrap();
    for sql in [
        "SELECT provider_response FROM transfer_operations",
        "SELECT last_error FROM transfer_jobs",
    ] {
        let value: String = conn.query_row(sql, [], |r| r.get(0)).unwrap();
        assert!(!value.contains("cookie_secret"));
        assert!(!value.contains("bearer_secret"));
    }
}

#[tokio::test]
#[ignore = "fixture driver launched only by subprocess_crash_boundaries_do_not_replay"]
async fn crash_child_driver() {
    let directory = PathBuf::from(
        std::env::var("SOUNDSHIFT_UNIT_CRASH_DIRECTORY")
            .expect("isolated fixture directory required"),
    );
    assert!(directory.starts_with(std::env::temp_dir()));
    assert!(directory.join("offline-fixture").is_file());
    let conn = crate::storage::init_db(&directory).unwrap();
    create_pre_mutation_snapshot(
        &conn,
        "worker_job",
        "spotify",
        "ytmusic",
        "source",
        "target",
        "target",
        false,
        1,
        1,
        vec![],
        vec![],
    )
    .unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = TransferControl::new("worker_job".into(), db.clone(), |_: LifecycleUpdate| {});
    control.start().unwrap();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.remote_file = Some(directory.join("offline-remote.log"));
    journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["track".into()],
        &AtomicU32::new(1),
        &control,
    )
    .await
    .unwrap();
    panic!("The subprocess should exit at its selected crash point");
}

struct CrashDirectory(PathBuf);
impl Drop for CrashDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn subprocess_crash_boundaries_do_not_replay() {
    for point in ["intent", "send", "response", "checkpoint"] {
        let dir = CrashDirectory(
            std::env::temp_dir().join(format!("soundshift_crash_{}", rand::random::<u64>())),
        );
        std::fs::create_dir(&dir.0).unwrap();
        std::fs::write(dir.0.join("offline-fixture"), b"No live provider access").unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine::worker::safety_tests::crash_child_driver",
                "--ignored",
                "--nocapture",
            ])
            .env("SOUNDSHIFT_UNIT_CRASH_DIRECTORY", &dir.0)
            .env("SOUNDSHIFT_UNIT_CRASH_POINT", point)
            .output()
            .unwrap();
        assert_eq!(
            child.status.code(),
            Some(73),
            "fixture {point}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let remote_path = dir.0.join("offline-remote.log");
        let before = std::fs::read(&remote_path).unwrap_or_default();
        assert_eq!(!before.is_empty(), point != "intent");
        for _ in 0..2 {
            let conn = crate::storage::init_db(&dir.0).unwrap();
            let state: String = conn
                .query_row("SELECT job_state FROM transfer_jobs", [], |r| r.get(0))
                .unwrap();
            assert_eq!(state, "recovery_required");
            let snapshot = load_snapshot(&conn, "snap_worker_job").unwrap();
            assert_eq!(
                snapshot.added_track_ids.len(),
                usize::from(point == "checkpoint")
            );
            assert_eq!(
                std::fs::read(&remote_path).unwrap_or_default(),
                before,
                "restart cannot replay remote writes"
            );
        }
        assert!(Path::new(&dir.0).join("soundshift.db").is_file());
    }
}

#[tokio::test]
async fn cancellation_interrupts_inter_batch_pacing() {
    let (_, control) = fixture();
    let cancellation = control.clone();
    let pacing = tokio::spawn(async move { pace_batch(&control).await });
    tokio::task::yield_now().await;
    cancellation.cancel().unwrap();
    tokio::time::timeout(Duration::from_millis(100), pacing)
        .await
        .expect("pacing must wake on cancellation")
        .unwrap();
}

#[tokio::test]
async fn unsupported_youtube_duplicate_occurrences_block_before_creation() {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = TransferControl::new("duplicates".into(), db.clone(), |_| {});
    let provider = FakeProvider::new(false, Ok(()));
    let track: SourceTrack = serde_json::from_value(serde_json::json!({
        "id":"same", "title":"same", "artists":[], "duration_ms":0,
        "is_explicit":false,"is_playable":true
    }))
    .unwrap();
    let config = BatchTransferConfig {
        job_id: "duplicates".into(),
        source_service: "spotify".into(),
        target_service: "ytmusic".into(),
        source_playlist_name: "fixture".into(),
        target_playlist_id: None,
        target_playlist_name: None,
        is_new_playlist: true,
        tracks: vec![track.clone(), track],
        concurrency: 1,
        skip_duplicates: false,
    };
    let result = prepare_transfer(&db, &provider, &config, &control).await;
    assert!(
        result.is_err(),
        "duplicate preservation is unsupported and must not silently skip"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unstable_read_never_becomes_a_complete_baseline() {
    let provider = FakeProvider::new(false, Ok(()));
    *provider.read_ids.lock().unwrap() = vec![vec!["a".into()], vec!["b".into()]];
    assert!(provider.read_playlist("target").await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn partial_write_checkpoints_accepted_and_uncertain_occurrences_separately() {
    use crate::providers::contracts::*;
    let (db, control) = fixture();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.outcome = Some(MutationOutcome {
        occurrences: vec![
            OccurrenceOutcome {
                index: 0,
                track_id: "a".into(),
                status: OccurrenceStatus::Accepted,
                remote_id: Some("occ-a".into()),
            },
            OccurrenceOutcome {
                index: 1,
                track_id: "b".into(),
                status: OccurrenceStatus::Uncertain,
                remote_id: None,
            },
        ],
        revision: None,
    });
    let result = journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["a".into(), "b".into()],
        &AtomicU32::new(2),
        &control,
    )
    .await;
    assert!(result.is_err(), "partial write must require recovery");
    let conn = db.lock().unwrap();
    let evidence: String = conn
        .query_row("SELECT outcome_json FROM transfer_operations", [], |r| {
            r.get(0)
        })
        .unwrap();
    let outcome: MutationOutcome = serde_json::from_str(&evidence).unwrap();
    assert_eq!(outcome.occurrences[0].status, OccurrenceStatus::Accepted);
    assert_eq!(outcome.occurrences[1].status, OccurrenceStatus::Uncertain);
    assert_eq!(
        load_snapshot(&conn, "snap_worker_job")
            .unwrap()
            .added_track_ids,
        vec!["a"]
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn playlist_mutations_serialize_for_every_provider() {
    assert_eq!(resolve_concurrency("spotify", 8), 1);
    assert_eq!(resolve_concurrency("ytmusic", 8), 1);
}

#[tokio::test]
async fn before_state_is_durable_before_dispatch() {
    let (db, control) = fixture();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.check_before = Some(db.clone());
    assert!(journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["track".into()],
        &AtomicU32::new(0),
        &control
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn recovery_inspection_reports_disjoint_partial_counts_and_blocks_replay() {
    use crate::providers::contracts::*;
    let (db, control) = fixture();
    db.lock()
        .unwrap()
        .execute("UPDATE transfer_jobs SET total_tracks=3", [])
        .unwrap();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.outcome = Some(MutationOutcome {
        occurrences: vec![
            OccurrenceOutcome {
                index: 0,
                track_id: "a".into(),
                status: OccurrenceStatus::Accepted,
                remote_id: Some("owned-a".into()),
            },
            OccurrenceOutcome {
                index: 1,
                track_id: "b".into(),
                status: OccurrenceStatus::Uncertain,
                remote_id: None,
            },
        ],
        revision: None,
    });
    assert!(journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["a".into(), "b".into()],
        &AtomicU32::new(0),
        &control
    )
    .await
    .is_err());
    let inspection =
        crate::engine::recovery::inspect_job(&db.lock().unwrap(), "worker_job").unwrap();
    assert_eq!(inspection.counts.confirmed, 1);
    assert_eq!(inspection.counts.uncertain, 1);
    assert_eq!(inspection.counts.unsent, 1);
    assert_eq!(inspection.counts.selected, 3);
    assert!(!inspection.can_resume);
}

#[test]
fn planning_retains_original_occurrence_indices_and_durable_unsent_work() {
    let (db, _) = fixture();
    let track:SourceTrack=serde_json::from_value(serde_json::json!({"id":"a","title":"fixture","artists":[],"duration_ms":0,"is_explicit":false,"is_playable":true})).unwrap();
    let config = BatchTransferConfig {
        job_id: "worker_job".into(),
        source_service: "spotify".into(),
        target_service: "spotify".into(),
        source_playlist_name: "fixture".into(),
        target_playlist_id: Some("target".into()),
        target_playlist_name: None,
        is_new_playlist: false,
        tracks: vec![track.clone(), track.clone()],
        concurrency: 8,
        skip_duplicates: true,
    };
    let batches = plan_batches(&db.lock().unwrap(), &config, &[]).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].source_indices, vec![0]);
    let conn = db.lock().unwrap();
    let indices: String = conn
        .query_row("SELECT skipped_indices_json FROM transfer_jobs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(indices, "[1]");
    let planned: bool = conn
        .query_row(
            "SELECT sent_at IS NULL AND evidence_version=2 FROM transfer_operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(planned);
}

#[tokio::test]
async fn reconciliation_never_uses_membership_or_absence_as_write_proof() {
    let (db, _) = fixture();
    record_operation_intent(&db.lock().unwrap(), "worker_job", 0, &["a".into()]).unwrap();
    let provider = FakeProvider::new(false, Ok(()));
    *provider.read_ids.lock().unwrap() = vec![vec!["a".into()]];
    let inspection = crate::engine::recovery::reconcile_job(&db, &provider, "worker_job", |_| {})
        .await
        .unwrap();
    assert_eq!(inspection.counts.confirmed, 0);
    assert_eq!(inspection.counts.uncertain, 1);
    assert_eq!(inspection.state, "manual_review");
    *provider.read_ids.lock().unwrap() = vec![vec![]];
    let inspection = crate::engine::recovery::reconcile_job(&db, &provider, "worker_job", |_| {})
        .await
        .unwrap();
    assert_eq!(inspection.counts.uncertain, 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resume_uses_durable_operation_ids_without_replaying_confirmed_work() {
    use crate::providers::contracts::*;
    let (db, _) = fixture();
    let track = |id: &str| {
        serde_json::from_value::<SourceTrack>(serde_json::json!({"id":id,"title":"fixture","artists":[],"duration_ms":0,"is_explicit":false,"is_playable":true})).unwrap()
    };
    let config = BatchTransferConfig {
        job_id: "worker_job".into(),
        source_service: "spotify".into(),
        target_service: "spotify".into(),
        source_playlist_name: "fixture".into(),
        target_playlist_id: Some("target".into()),
        target_playlist_name: None,
        is_new_playlist: false,
        tracks: vec![track("a"), track("b")],
        concurrency: 1,
        skip_duplicates: false,
    };
    {
        let conn = db.lock().unwrap();
        conn.execute("UPDATE transfer_jobs SET total_tracks=2,manifest_json=?1,job_state='recovery_required'",[serde_json::to_string(&config).unwrap()]).unwrap();
        for (index, id) in ["a", "b"].iter().enumerate() {
            let op =
                record_operation_intent(&conn, "worker_job", index, &[id.to_string()]).unwrap();
            conn.execute("UPDATE transfer_operations SET evidence_version=2,source_indices_json=?1 WHERE operation_id=?2",rusqlite::params![format!("[{index}]"),op]).unwrap();
            if index == 0 {
                update_operation_outcome(
                    &conn,
                    &op,
                    OperationOutcome::Confirmed,
                    1,
                    Some("accepted"),
                )
                .unwrap();
                let outcome = MutationOutcome {
                    occurrences: vec![OccurrenceOutcome {
                        index: 0,
                        track_id: "a".into(),
                        status: OccurrenceStatus::Accepted,
                        remote_id: Some("owned-a".into()),
                    }],
                    revision: None,
                };
                conn.execute(
                    "UPDATE transfer_operations SET outcome_json=?1 WHERE operation_id=?2",
                    rusqlite::params![serde_json::to_string(&outcome).unwrap(), op],
                )
                .unwrap();
            }
        }
    }
    let provider = FakeProvider::new(false, Ok(()));
    *provider.read_ids.lock().unwrap() = vec![vec!["a".into()]];
    let control = TransferControl::new("worker_job".into(), db.clone(), |_| {});
    let inspection = crate::engine::recovery::resume_job(&db, &provider, "worker_job", &control)
        .await
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(inspection.counts.confirmed, 2);
    assert_eq!(inspection.state, "completed_verified");
    assert!(
        crate::engine::recovery::resume_job(&db, &provider, "worker_job", &control)
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn verification_rejects_reordered_occurrences_even_with_equal_membership() {
    let provider = FakeProvider::new(false, Ok(()));
    *provider.read_ids.lock().unwrap() = vec![vec!["b".into(), "a".into()]];
    let audit = audit_playlist(&provider, "target", &["a".into(), "b".into()])
        .await
        .unwrap();
    assert!(!audit.is_verified);
}

#[tokio::test]
async fn legacy_snapshot_without_attribution_never_dispatches_rollback() {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    let snapshot = create_pre_mutation_snapshot(
        &conn,
        "legacy",
        "spotify",
        "ytmusic",
        "fixture",
        "target",
        "target",
        false,
        1,
        1,
        vec![],
        vec!["a".into()],
    )
    .unwrap();
    let db = Mutex::new(conn);
    let provider = FakeProvider::new(false, Ok(()));
    *provider.read_ids.lock().unwrap() = vec![vec!["a".into()]];
    assert!(
        crate::engine::snapshot::rollback_snapshot(&db, &snapshot, false, &provider)
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn exhausted_retry_budget_survives_job_restart() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE transfer_jobs SET manifest_json='{}',job_state='recovery_required'",
        [],
    )
    .unwrap();
    let op = record_operation_intent(&conn, "worker_job", 0, &["a".into()]).unwrap();
    let outcome = crate::providers::contracts::MutationOutcome {
        occurrences: vec![crate::providers::contracts::OccurrenceOutcome {
            index: 0,
            track_id: "a".into(),
            status: crate::providers::contracts::OccurrenceStatus::Rejected,
            remote_id: None,
        }],
        revision: None,
    };
    let error =
        crate::providers::error::ProviderError::http(429, &reqwest::header::HeaderMap::new());
    conn.execute("UPDATE transfer_operations SET evidence_version=2,source_indices_json='[0]',status='failed',attempt_count=22,sent_at=1,outcome_json=?1,provider_response=?2 WHERE operation_id=?3",rusqlite::params![serde_json::to_string(&outcome).unwrap(),serde_json::to_string(&error).unwrap(),op]).unwrap();
    assert!(
        !crate::engine::recovery::inspect_job(&conn, "worker_job")
            .unwrap()
            .can_resume
    );
}
#[test]
fn retry_guidance_defers_resume_across_restart() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE transfer_jobs SET manifest_json='{}',job_state='recovery_required'",
        [],
    )
    .unwrap();
    let op = record_operation_intent(&conn, "worker_job", 0, &["a".into()]).unwrap();
    let outcome = MutationOutcome {
        occurrences: vec![OccurrenceOutcome {
            index: 0,
            track_id: "a".into(),
            status: OccurrenceStatus::Rejected,
            remote_id: None,
        }],
        revision: None,
    };
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "60".parse().unwrap());
    let error = crate::providers::error::ProviderError::http(429, &headers);
    conn.execute("UPDATE transfer_operations SET evidence_version=2,source_indices_json='[0]',status='failed',attempt_count=1,sent_at=1,outcome_json=?1,provider_response=?2 WHERE operation_id=?3",rusqlite::params![serde_json::to_string(&outcome).unwrap(),serde_json::to_string(&error).unwrap(),op]).unwrap();
    assert!(
        !crate::engine::recovery::inspect_job(&conn, "worker_job")
            .unwrap()
            .can_resume
    );
}
#[tokio::test]
async fn cancellation_during_preflight_prevents_outbound_mutation() {
    let (db, control) = fixture();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.gate_canary = true;
    let provider = Arc::new(provider);
    let task = spawn_add(db, control.clone(), provider.clone());
    entered(&provider).await;
    control.cancel().unwrap();
    provider.release.add_permits(1);
    assert!(!task.await.unwrap().unwrap());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pause_during_preflight_acknowledges_without_starting_a_write() {
    let (db, control) = fixture();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.gate_canary = true;
    let provider = Arc::new(provider);
    let task = spawn_add(db, control.clone(), provider.clone());
    entered(&provider).await;
    control.pause().unwrap();
    provider.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(1), async {
        while control.snapshot().unwrap().state != JobState::Paused {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    control.cancel().unwrap();
    assert!(!task.await.unwrap().unwrap());
}

#[tokio::test]
async fn cancellation_preserves_proven_rejection_evidence_for_explicit_resume() {
    let (db, control) = fixture();
    let provider = Arc::new(FakeProvider::new(false, Err("429 rate limited".into())));
    let task = spawn_add(db.clone(), control.clone(), provider.clone());
    entered(&provider).await;
    control.cancel().unwrap();
    assert!(!task.await.unwrap().unwrap());
    let inspection =
        crate::engine::recovery::inspect_job(&db.lock().unwrap(), "worker_job").unwrap();
    assert!(
        inspection.operations[0].can_retry(),
        "cancellation must not erase the rejected response proof"
    );
}

#[tokio::test]
async fn destination_revision_drift_blocks_even_unchanged_track_membership() {
    let (db, control) = fixture();
    db.lock()
        .unwrap()
        .execute(
            "UPDATE transfer_jobs SET target_fingerprint=?1",
            [r#"{"revision":"old"}"#],
        )
        .unwrap();
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.revision = Some("new".into());
    assert!(journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["track".into()],
        &AtomicU32::new(0),
        &control
    )
    .await
    .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn final_verification_rejects_a_different_provider_revision() {
    let mut provider = FakeProvider::new(false, Ok(()));
    provider.revision = Some("unexpected".into());
    *provider.read_ids.lock().unwrap() = vec![vec!["a".into()]];
    let audit = crate::engine::snapshot::audit_playlist_at_revision(
        &provider,
        "target",
        &["a".into()],
        Some("receipt-revision"),
    )
    .await
    .unwrap();
    assert!(!audit.is_verified);
}

#[tokio::test]
async fn rate_limited_preflight_is_unsent_instead_of_uncertain() {
    let (db, control) = fixture();
    let mut provider = FakeProvider::new(false, Ok(()));
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "60".parse().unwrap());
    provider.session_error = Some(crate::providers::error::ProviderError::read_http(
        429, &headers,
    ));
    assert!(journaled_add(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["a".into()],
        &AtomicU32::new(0),
        &control
    )
    .await
    .is_err());
    let inspection =
        crate::engine::recovery::inspect_job(&db.lock().unwrap(), "worker_job").unwrap();
    assert_eq!(inspection.counts.unsent, 1);
    assert_eq!(inspection.counts.uncertain, 0);
    assert!(!inspection.can_resume);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn inconsistent_occurrence_receipts_cannot_change_a_recovery_payload() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    let id = record_operation_intent(&conn, "worker_job", 0, &["original".into()]).unwrap();
    update_operation_outcome(&conn, &id, OperationOutcome::Confirmed, 1, Some("accepted")).unwrap();
    let outcome = MutationOutcome {
        occurrences: vec![OccurrenceOutcome {
            index: 0,
            track_id: "different".into(),
            status: OccurrenceStatus::Accepted,
            remote_id: Some("wrong-owner".into()),
        }],
        revision: None,
    };
    conn.execute(
        "UPDATE transfer_operations SET outcome_json=?1 WHERE operation_id=?2",
        rusqlite::params![serde_json::to_string(&outcome).unwrap(), id],
    )
    .unwrap();
    assert!(load_snapshot(&conn, "snap_worker_job").is_err());
    assert!(crate::engine::recovery::inspect_job(&conn, "worker_job").is_err());
}

#[test]
fn altered_journal_payload_cannot_be_replayed_or_counted_as_confirmed() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    let id = record_operation_intent(&conn, "worker_job", 0, &["original".into()]).unwrap();
    update_operation_outcome(&conn, &id, OperationOutcome::Confirmed, 1, Some("accepted")).unwrap();
    conn.execute(
        "UPDATE transfer_operations SET payload_json=?1 WHERE operation_id=?2",
        rusqlite::params![r#"["different"]"#, id],
    )
    .unwrap();
    assert!(load_snapshot(&conn, "snap_worker_job").is_err());
    assert!(crate::engine::recovery::inspect_job(&conn, "worker_job").is_err());
}

#[test]
fn coarse_checkpoint_timestamps_never_shorten_retry_guidance() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    let op = record_operation_intent(&conn, "worker_job", 0, &["a".into()]).unwrap();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "60".parse().unwrap());
    let error = crate::providers::error::ProviderError::read_http(429, &headers);
    conn.execute("UPDATE transfer_operations SET evidence_version=2,status='failed',updated_at=1000,provider_response=?1 WHERE operation_id=?2",rusqlite::params![serde_json::to_string(&error).unwrap(),op]).unwrap();
    let inspection = crate::engine::recovery::inspect_job(&conn, "worker_job").unwrap();
    assert!(
        inspection.operations[0].retry_not_before_ms.unwrap() >= 1_060_999,
        "the response may have arrived at the end of the recorded second"
    );
}

#[test]
fn legacy_confirmation_cannot_authorize_a_new_automatic_resume() {
    let (db, _) = fixture();
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE transfer_jobs SET total_tracks=2,manifest_json='{}',job_state='recovery_required'",
        [],
    )
    .unwrap();
    let old = record_operation_intent(&conn, "worker_job", 0, &["old".into()]).unwrap();
    update_operation_outcome(
        &conn,
        &old,
        OperationOutcome::Confirmed,
        1,
        Some("legacy acceptance"),
    )
    .unwrap();
    let pending = record_operation_intent(&conn, "worker_job", 1, &["new".into()]).unwrap();
    conn.execute("UPDATE transfer_operations SET evidence_version=2,source_indices_json='[1]' WHERE operation_id=?1",[pending]).unwrap();
    assert!(
        !crate::engine::recovery::inspect_job(&conn, "worker_job")
            .unwrap()
            .can_resume
    );
}

#[tokio::test]
async fn chunk_revision_fast_check_skips_full_reads_and_blocks_drift() {
    use crate::providers::contracts::*;
    struct RevProvider {
        full_reads: AtomicUsize,
        rev_checks: AtomicUsize,
        live_rev: Mutex<String>,
        tracks: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl MusicProvider for RevProvider {
        async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
            Ok(vec![])
        }
        async fn get_playlist_tracks(
            &self,
            _: &str,
            _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
        ) -> Result<Vec<SourceTrack>, String> {
            self.full_reads.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .tracks
                .lock()
                .unwrap()
                .iter()
                .map(|id| SourceTrack {
                    id: id.clone(),
                    ..SourceTrack::unavailable(0)
                })
                .collect())
        }
        async fn search_track(&self, _: &str) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
            Ok("pl".into())
        }
        async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn delete_playlist(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn playlist_revision(
            &self,
            _: &str,
        ) -> Result<Option<String>, crate::providers::error::ProviderError> {
            self.rev_checks.fetch_add(1, Ordering::SeqCst);
            Ok(Some(self.live_rev.lock().unwrap().clone()))
        }
        async fn add_occurrences(
            &self,
            _: &str,
            ids: &[String],
        ) -> Result<MutationOutcome, crate::providers::error::ProviderError> {
            self.tracks.lock().unwrap().extend(ids.iter().cloned());
            let mut rev = self.live_rev.lock().unwrap();
            *rev = format!("{}_next", *rev);
            Ok(MutationOutcome {
                occurrences: ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| OccurrenceOutcome {
                        index,
                        track_id: id.clone(),
                        status: OccurrenceStatus::Accepted,
                        remote_id: None,
                    })
                    .collect(),
                revision: Some(rev.clone()),
            })
        }
    }

    let (db, control) = fixture();
    let provider = RevProvider {
        full_reads: AtomicUsize::new(0),
        rev_checks: AtomicUsize::new(0),
        live_rev: Mutex::new("rev0".into()),
        tracks: Mutex::new(vec![]),
    };
    assert!(journaled_add_with_options(
        &db,
        &provider,
        "worker_job",
        "target",
        0,
        &["t1".into()],
        &AtomicU32::new(0),
        &control,
        true
    )
    .await
    .unwrap());
    let reads_after_first = provider.full_reads.load(Ordering::SeqCst);
    assert!(reads_after_first >= 2);

    assert!(journaled_add_with_options(
        &db,
        &provider,
        "worker_job",
        "target",
        1,
        &["t2".into()],
        &AtomicU32::new(0),
        &control,
        true
    )
    .await
    .unwrap());
    assert_eq!(
        provider.full_reads.load(Ordering::SeqCst),
        reads_after_first,
        "subsequent chunk should verify lightweight revision without repeating full reads"
    );
    assert!(provider.rev_checks.load(Ordering::SeqCst) > 0);

    *provider.live_rev.lock().unwrap() = "external_drift".into();
    let drift_err = journaled_add_with_options(
        &db,
        &provider,
        "worker_job",
        "target",
        2,
        &["t3".into()],
        &AtomicU32::new(0),
        &control,
        true,
    )
    .await
    .unwrap_err();
    assert!(drift_err.contains("revision changed"));
}

#[tokio::test]
async fn create_playlist_rejection_surfaces_sanitized_reason_and_records_failed_outcome() {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = TransferControl::new("create_fail_job".into(), db.clone(), |_| {});
    let provider = FakeProvider::new(
        false,
        Err("Playlist creation failed with status: 400 Bad Request".into()),
    );
    let track: SourceTrack = serde_json::from_value(serde_json::json!({
        "id": "yt-1",
        "title": "Song",
        "artists": ["Artist"],
        "duration_ms": 180000,
        "is_explicit": false,
        "is_playable": true
    }))
    .unwrap();
    let config = BatchTransferConfig {
        job_id: "create_fail_job".into(),
        source_service: "spotify".into(),
        target_service: "ytmusic".into(),
        source_playlist_name: "Ummm....".into(),
        target_playlist_id: None,
        target_playlist_name: Some("Ummm....".into()),
        is_new_playlist: true,
        tracks: vec![track],
        concurrency: 1,
        skip_duplicates: true,
    };

    let err = prepare_transfer(&db, &provider, &config, &control)
        .await
        .unwrap_err();
    assert!(
        err.contains("400"),
        "expected sanitized 400 reason in error, got: {err}"
    );

    let conn = db.lock().unwrap();
    let (status, certainty, provider_response): (String, String, Option<String>) = conn
        .query_row(
            "SELECT status, certainty, provider_response FROM transfer_operations WHERE job_id = ?1 AND operation_type = 'create_playlist'",
            ["create_fail_job"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(certainty, "failed");
    assert!(provider_response.unwrap_or_default().contains("400"));
}
