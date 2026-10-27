use super::*;
use crate::providers::contracts::{OccurrenceOutcome, PlaylistOccurrence};
fn occurrence(index: usize, id: &str, remote: &str) -> PlaylistOccurrence {
    PlaylistOccurrence {index,track:serde_json::from_value(serde_json::json!({"id":id,"title":"fixture","artists":[],"duration_ms":0,"is_explicit":false,"is_playable":true})).unwrap(),remote_id:Some(remote.into())}
}
struct OccurrenceProvider {
    live: std::sync::Mutex<Vec<PlaylistOccurrence>>,
    removed: std::sync::Mutex<Vec<String>>,
    remote_file: Option<std::path::PathBuf>,
    lose_response: std::sync::atomic::AtomicBool,
    deleted: std::sync::atomic::AtomicBool,
    deletion_verified: bool,
}
#[async_trait::async_trait]
impl crate::providers::traits::MusicProvider for OccurrenceProvider {
    async fn list_playlists(&self) -> Result<Vec<crate::models::Playlist>, String> {
        Ok(vec![])
    }
    async fn get_playlist_tracks(
        &self,
        _: &str,
        _: Option<tokio::sync::mpsc::Sender<Vec<crate::models::SourceTrack>>>,
    ) -> Result<Vec<crate::models::SourceTrack>, String> {
        Ok(self
            .live
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.track.clone())
            .collect())
    }
    async fn read_playlist(&self, _: &str) -> Result<PlaylistRead, ProviderError> {
        Ok(PlaylistRead {
            occurrences: self.live.lock().unwrap().clone(),
            revision: None,
            complete: true,
        })
    }
    async fn search_track(&self, _: &str) -> Result<Vec<crate::models::SourceTrack>, String> {
        Ok(vec![])
    }
    async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
        Err("unused".into())
    }
    async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        Err("unused".into())
    }
    async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        panic!("raw track-ID removal must never be used")
    }
    async fn delete_playlist(&self, _: &str) -> Result<(), String> {
        self.deleted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if self.deletion_verified {
            self.live.lock().unwrap().clear();
        }
        Ok(())
    }
    fn supports_verified_deletion(&self) -> bool {
        true
    }
    async fn verify_playlist_removed(&self, _: &str) -> Result<bool, ProviderError> {
        Ok(self.deleted.load(std::sync::atomic::Ordering::SeqCst) && self.deletion_verified)
    }
    fn supports_occurrence_removal(&self) -> bool {
        true
    }
    async fn remove_occurrences(
        &self,
        _: &str,
        occurrences: &[PlaylistOccurrence],
        _: Option<&str>,
    ) -> Result<(), ProviderError> {
        let ids: Vec<_> = occurrences
            .iter()
            .map(|o| o.remote_id.clone().unwrap())
            .collect();
        if let Some(path) = &self.remote_file {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "accepted_owned_removal").unwrap();
            file.sync_all().unwrap();
            if std::env::var("SOUNDSHIFT_RECOVERY_CRASH_POINT").as_deref() == Ok("rollback_send") {
                std::process::exit(73);
            }
        }
        self.removed.lock().unwrap().extend(ids.clone());
        self.live
            .lock()
            .unwrap()
            .retain(|o| !ids.contains(o.remote_id.as_ref().unwrap()));
        if self
            .lose_response
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ProviderError::new(
                crate::providers::error::ProviderErrorKind::Timeout,
                SendCertainty::MayHaveApplied,
                "response lost after applied removal",
            ));
        }
        Ok(())
    }
}
fn rollback_fixture() -> (std::sync::Mutex<Connection>, String, OccurrenceProvider) {
    rollback_fixture_mode(false)
}

fn rollback_fixture_mode(
    is_new: bool,
) -> (std::sync::Mutex<Connection>, String, OccurrenceProvider) {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    let snapshot = crate::engine::snapshot::create_pre_mutation_snapshot(
        &conn,
        "owned-job",
        "spotify",
        "ytmusic",
        "fixture",
        "target",
        "target",
        is_new,
        1,
        1,
        if is_new {
            vec![]
        } else {
            vec!["duplicate".into()]
        },
        vec![],
    )
    .unwrap();
    let before = PlaylistRead {
        occurrences: if is_new {
            vec![]
        } else {
            vec![occurrence(0, "duplicate", "old")]
        },
        revision: None,
        complete: true,
    };
    let outcome = MutationOutcome {
        occurrences: vec![OccurrenceOutcome {
            index: 0,
            track_id: "duplicate".into(),
            status: OccurrenceStatus::Accepted,
            remote_id: Some("owned".into()),
        }],
        revision: None,
    };
    let op = crate::engine::snapshot::record_operation_intent(
        &conn,
        "owned-job",
        0,
        &["duplicate".into()],
    )
    .unwrap();
    crate::engine::snapshot::update_operation_outcome(
        &conn,
        &op,
        crate::engine::snapshot::OperationOutcome::Confirmed,
        1,
        Some("accepted"),
    )
    .unwrap();
    conn.execute("UPDATE transfer_operations SET evidence_version=2,before_json=?1,outcome_json=?2,sent_at=1,source_indices_json='[0]' WHERE operation_id=?3",rusqlite::params![serde_json::to_string(&before).unwrap(),serde_json::to_string(&outcome).unwrap(),op]).unwrap();
    (
        std::sync::Mutex::new(conn),
        snapshot,
        OccurrenceProvider {
            live: std::sync::Mutex::new(if is_new {
                vec![occurrence(0, "duplicate", "owned")]
            } else {
                vec![
                    occurrence(0, "duplicate", "old"),
                    occurrence(1, "duplicate", "owned"),
                ]
            }),
            removed: std::sync::Mutex::new(vec![]),
            remote_file: None,
            lose_response: std::sync::atomic::AtomicBool::new(false),
            deleted: std::sync::atomic::AtomicBool::new(false),
            deletion_verified: false,
        },
    )
}
#[tokio::test]
async fn rollback_removes_only_owned_duplicate_and_verifies_original_order() {
    let (db, snapshot, provider) = rollback_fixture();
    crate::engine::snapshot::rollback_snapshot(&db, &snapshot, false, &provider)
        .await
        .unwrap();
    assert_eq!(*provider.removed.lock().unwrap(), vec!["owned"]);
    assert_eq!(
        provider.live.lock().unwrap()[0].remote_id.as_deref(),
        Some("old")
    );
    let done: bool = db
        .lock()
        .unwrap()
        .query_row("SELECT is_rolled_back FROM transfer_snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(done);
}

#[test]
fn reconciliation_requires_owned_occurrence_receipts_and_unchanged_prefix() {
    let before = PlaylistRead {
        occurrences: vec![occurrence(0, "duplicate", "old")],
        revision: None,
        complete: true,
    };
    let receipt = MutationOutcome {
        occurrences: vec![OccurrenceOutcome {
            index: 0,
            track_id: "duplicate".into(),
            status: OccurrenceStatus::Uncertain,
            remote_id: Some("owned".into()),
        }],
        revision: None,
    };
    let after = PlaylistRead {
        occurrences: vec![
            occurrence(0, "duplicate", "old"),
            occurrence(1, "duplicate", "owned"),
        ],
        revision: None,
        complete: true,
    };
    assert!(reconcile_receipts(&before, &receipt, &after)
        .unwrap()
        .all_accepted());
    assert!(reconcile_receipts(&before, &receipt, &before).is_none());
    let changed = PlaylistRead {
        occurrences: vec![
            occurrence(0, "duplicate", "someone-else"),
            occurrence(1, "duplicate", "owned"),
        ],
        ..after
    };
    assert!(reconcile_receipts(&before, &receipt, &changed).is_none());
}

#[test]
fn recovery_subprocess_boundaries_preserve_uncertainty_without_replay() {
    let temporary = std::env::temp_dir();
    for point in [
        "rollback_intent",
        "rollback_send",
        "rollback_response",
        "rollback_checkpoint",
        "reconcile_intent",
        "reconcile_response",
        "reconcile_checkpoint",
    ] {
        let root = temporary.join(format!(
            "soundshift_recovery_crash_{}_{}",
            point,
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine::recovery::tests::recovery_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("SOUNDSHIFT_RECOVERY_CRASH_POINT", point)
            .env("SOUNDSHIFT_RECOVERY_CRASH_ROOT", &root)
            .output()
            .unwrap();
        assert_eq!(
            status.status.code(),
            Some(73),
            "{} failed to interrupt at its boundary: {}",
            point,
            String::from_utf8_lossy(&status.stderr)
        );
        let log = root.join("remote.log");
        let before = std::fs::read_to_string(&log).unwrap_or_default();
        let expected = usize::from(matches!(
            point,
            "rollback_send" | "rollback_response" | "rollback_checkpoint"
        ));
        assert_eq!(before.lines().count(), expected);
        for _ in 0..2 {
            let conn = crate::storage::init_db(&root).unwrap();
            let state: String = conn
                .query_row(
                    "SELECT job_state FROM transfer_jobs WHERE job_id='owned-job'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                state, "recovery_required",
                "unfinished {} must be discoverable",
                point
            );
            let restored: bool = conn
                .query_row("SELECT is_rolled_back FROM transfer_snapshots", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(restored, point == "rollback_checkpoint");
            assert_eq!(std::fs::read_to_string(&log).unwrap_or_default(), before);
        }
        assert!(root.starts_with(&temporary));
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[tokio::test]
#[ignore = "subprocess fixture invoked by recovery_subprocess_boundaries_preserve_uncertainty_without_replay"]
async fn recovery_crash_child() {
    let Ok(root) = std::env::var("SOUNDSHIFT_RECOVERY_CRASH_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let (memory, snapshot, mut provider) = rollback_fixture();
    {
        let conn = memory.lock().unwrap();
        crate::storage::schema::run_migrations(&conn).unwrap();
        conn.execute(
            "VACUUM INTO ?1",
            [root.join("soundshift.db").to_string_lossy().as_ref()],
        )
        .unwrap();
    }
    let db = std::sync::Mutex::new(Connection::open(root.join("soundshift.db")).unwrap());
    provider.remote_file = Some(root.join("remote.log"));
    let point = std::env::var("SOUNDSHIFT_RECOVERY_CRASH_POINT").unwrap();
    if point.starts_with("reconcile") {
        reconcile_job(&db, &provider, "owned-job", |_| {})
            .await
            .unwrap();
    } else {
        rollback_recorded(&db, &snapshot, false, &provider, |_| {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn lost_rollback_response_is_verified_without_replaying_removal() {
    let (db, snapshot, provider) = rollback_fixture();
    provider
        .lose_response
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(rollback_recorded(&db, &snapshot, false, &provider, |_| {})
        .await
        .is_err());
    let result = rollback_recorded(&db, &snapshot, false, &provider, |_| {})
        .await
        .unwrap();
    assert_eq!(*provider.removed.lock().unwrap(), vec!["owned"]);
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.verification, "verified");
}

#[tokio::test]
async fn durable_rollback_checkpoint_recovers_its_final_lifecycle_acknowledgment() {
    let (db, snapshot, provider) = rollback_fixture();
    rollback_recorded(&db, &snapshot, false, &provider, |_| {})
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "UPDATE transfer_jobs SET job_state='recovery_required',recovery_required=1",
            [],
        )
        .unwrap();
    let result = rollback_recorded(&db, &snapshot, false, &provider, |_| {})
        .await
        .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.verification, "verified");
    assert_eq!(*provider.removed.lock().unwrap(), vec!["owned"]);
}

#[tokio::test]
async fn attributed_reconciliation_reaches_verified_completion_without_mutation() {
    let (db, _, provider) = rollback_fixture();
    let receipt = MutationOutcome {
        occurrences: vec![OccurrenceOutcome {
            index: 0,
            track_id: "duplicate".into(),
            status: OccurrenceStatus::Uncertain,
            remote_id: Some("owned".into()),
        }],
        revision: None,
    };
    db.lock().unwrap().execute("UPDATE transfer_operations SET status='ambiguous',certainty='ambiguous',outcome_json=?1",[serde_json::to_string(&receipt).unwrap()]).unwrap();
    let inspection = reconcile_job(&db, &provider, "owned-job", |_| {})
        .await
        .unwrap();
    assert_eq!(inspection.counts.confirmed, 1);
    assert_eq!(inspection.counts.uncertain, 0);
    assert_eq!(inspection.state, "completed_verified");
    assert_eq!(inspection.verification, "verified");
    assert!(provider.removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn whole_playlist_rollback_requires_independent_removal_verification() {
    for verified in [false, true] {
        let (db, snapshot, mut provider) = rollback_fixture_mode(true);
        provider.deletion_verified = verified;
        let result = rollback_recorded(&db, &snapshot, true, &provider, |_| {}).await;
        assert!(provider.deleted.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(result.is_ok(), verified);
        let done: bool = db
            .lock()
            .unwrap()
            .query_row("SELECT is_rolled_back FROM transfer_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(done, verified);
        let status = inspect_job(&db.lock().unwrap(), "owned-job").unwrap();
        assert_eq!(
            status.state,
            if verified {
                "rolled_back"
            } else {
                "recovery_required"
            }
        );
    }
}
