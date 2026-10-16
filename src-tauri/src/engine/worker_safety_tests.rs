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
    result: Result<(), String>,
    remote_file: Option<PathBuf>,
}

impl FakeProvider {
    fn new(gated: bool, result: Result<(), String>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Semaphore::new(0),
            gated,
            result,
            remote_file: None,
        }
    }
    async fn send(&self) -> Result<(), String> {
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
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        Ok(vec![])
    }
    async fn get_playlist_tracks(
        &self,
        _: &str,
        _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        Ok(vec![])
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
