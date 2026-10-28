use super::*;
use crate::models::SourceTrack;
use crate::providers::contracts::{PlaylistOccurrence, PlaylistRead};

fn track(id: &str) -> SourceTrack {
    SourceTrack {
        id: id.into(),
        title: id.into(),
        artists: vec!["fixture".into()],
        is_playable: true,
        ..SourceTrack::unavailable(0)
    }
}
fn read(ids: &[&str], remote: bool) -> PlaylistRead {
    PlaylistRead {
        complete: true,
        revision: Some("revision".into()),
        occurrences: ids
            .iter()
            .enumerate()
            .map(|(index, id)| PlaylistOccurrence {
                index,
                track: track(id),
                remote_id: remote.then(|| format!("{id}-{index}")),
            })
            .collect(),
    }
}

#[test]
fn initial_merge_preserves_requested_occurrences_and_order() {
    let plan = planner::initial_merge(
        "pair",
        &read(&["a", "a", "b"], false),
        &read(&["a", "b", "c"], true),
    );
    assert_eq!(
        plan.desired
            .iter()
            .map(|o| o
                .track(Side::Spotify)
                .or_else(|| o.track(Side::Ytmusic))
                .unwrap()
                .id
                .as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b", "c"]
    );
    assert_eq!(plan.desired.len(), 4);
    assert_eq!(
        plan.desired.iter().filter(|o| o.ytmusic.is_some()).count(),
        3
    );
}

#[test]
fn incremental_merge_propagates_removal_without_resurrecting_it() {
    let a = read(&["a", "b"], false);
    let b = read(&["a", "b"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b.clone(),
    };
    let plan = planner::incremental_merge("pair", &baseline, &read(&["b", "c"], false), &b);
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.desired
            .iter()
            .map(|o| o.spotify.as_ref().unwrap().id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c"]
    );
}

#[test]
fn incremental_merge_copies_a_one_sided_reorder() {
    let a = read(&["a", "b", "c"], false);
    let b = read(&["a", "b", "c"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b.clone(),
    };
    let plan = planner::incremental_merge("pair", &baseline, &read(&["c", "a", "b"], false), &b);
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.desired
            .iter()
            .map(|o| o.spotify.as_ref().unwrap().id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "a", "b"]
    );
}

#[test]
fn incompatible_two_sided_ordering_requires_review() {
    let a = read(&["a", "b", "c"], false);
    let b = read(&["a", "b", "c"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b.clone(),
    };
    let mut changed = b.clone();
    changed.occurrences.swap(0, 1);
    for (index, o) in changed.occurrences.iter_mut().enumerate() {
        o.index = index;
    }
    let plan =
        planner::incremental_merge("pair", &baseline, &read(&["c", "a", "b"], false), &changed);
    assert!(plan.conflicts.iter().any(|c| c.code == "ordering_conflict"));
}

#[test]
fn removal_against_a_concurrent_reorder_is_a_conflict() {
    let a = read(&["a", "b", "c"], false);
    let b = read(&["a", "b", "c"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b.clone(),
    };
    let mut moved = b;
    moved.occurrences.rotate_left(1);
    for (index, o) in moved.occurrences.iter_mut().enumerate() {
        o.index = index;
    }
    let plan = planner::incremental_merge("pair", &baseline, &read(&["b", "c"], false), &moved);
    assert!(plan
        .conflicts
        .iter()
        .any(|c| c.code == "removal_order_conflict"));
}

#[test]
fn changed_spotify_duplicate_baseline_cannot_attribute_occurrences() {
    let a = read(&["a", "a", "b"], false);
    let b = read(&["a", "a", "b"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a.clone(),
        ytmusic: b.clone(),
    };
    let mut changed = a;
    changed.revision = Some("changed".into());
    let plan = planner::incremental_merge("pair", &baseline, &changed, &b);
    assert!(plan
        .conflicts
        .iter()
        .any(|c| c.code == "duplicate_attribution"));
}

#[test]
fn concurrent_insertions_keep_anchors_with_spotify_first_in_shared_gap() {
    let a = read(&["a", "b"], false);
    let b = read(&["a", "b"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b.clone(),
    };
    let mut changed = b;
    changed.occurrences.insert(
        1,
        PlaylistOccurrence {
            index: 1,
            track: track("y"),
            remote_id: Some("y-new".into()),
        },
    );
    for (index, o) in changed.occurrences.iter_mut().enumerate() {
        o.index = index;
    }
    let plan =
        planner::incremental_merge("pair", &baseline, &read(&["a", "x", "b"], false), &changed);
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.desired
            .iter()
            .map(|o| o
                .spotify
                .as_ref()
                .or(o.ytmusic.as_ref())
                .unwrap()
                .id
                .as_str())
            .collect::<Vec<_>>(),
        vec!["a", "x", "y", "b"]
    );
}

fn database() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    crate::storage::schema::run_migrations(&conn).unwrap();
    conn.execute("INSERT INTO service_sessions(service_id,auth_data,updated_at) VALUES('spotify','fixture',1),('ytmusic','fixture',1)",[]).unwrap();
    conn
}

#[test]
fn saved_pairs_reserve_both_endpoints_and_survive_reload() {
    let conn = database();
    let pair = store::create_pair(
        &conn,
        "spotify-list",
        "Spotify fixture",
        "youtube-list",
        "YouTube fixture",
    )
    .unwrap();
    assert!(!pair.enabled);
    assert_eq!(
        store::get_pair(&conn, &pair.pair_id)
            .unwrap()
            .spotify
            .playlist_id,
        "spotify-list"
    );
    assert!(store::create_pair(&conn, "spotify-list", "other", "youtube-other", "other").is_err());
    assert_eq!(store::list_pairs(&conn).unwrap().len(), 1);
}

#[test]
fn durable_preview_rejects_tampering_instead_of_authorizing_writes() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let plan = planner::initial_merge(&pair.pair_id, &read(&["a"], false), &read(&["a"], true));
    let saved = store::save_preview(&conn, &plan).unwrap();
    assert_eq!(
        store::load_preview(&conn, &saved.plan_id).unwrap().plan_id,
        saved.plan_id
    );
    conn.execute(
        "UPDATE sync_previews SET payload_json='{}' WHERE plan_id=?1",
        [saved.plan_id.as_str()],
    )
    .unwrap();
    assert!(store::load_preview(&conn, &saved.plan_id)
        .unwrap_err()
        .contains("checksum"));
}

use crate::providers::contracts::{
    MutationOutcome, OccurrenceOutcome, OccurrenceStatus, SyncCapabilities,
};
use crate::providers::error::ProviderError;
use crate::providers::traits::MusicProvider;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
struct MemoryProvider {
    live: Mutex<PlaylistRead>,
    sends: AtomicUsize,
    editable: AtomicBool,
    uncertain: AtomicBool,
    after_send: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    file: Option<std::path::PathBuf>,
    rejections: AtomicUsize,
    attempts: AtomicUsize,
    retry_ms: AtomicUsize,
}
impl MemoryProvider {
    fn new(ids: &[&str], remote: bool) -> Arc<Self> {
        Arc::new(Self {
            live: Mutex::new(read(ids, remote)),
            sends: AtomicUsize::new(0),
            editable: AtomicBool::new(true),
            uncertain: AtomicBool::new(false),
            after_send: Mutex::new(None),
            file: None,
            rejections: AtomicUsize::new(0),
            attempts: AtomicUsize::new(0),
            retry_ms: AtomicUsize::new(5),
        })
    }
    fn receipt(&self, track_id: &str, remote_id: Option<String>) -> MutationOutcome {
        MutationOutcome {
            occurrences: vec![OccurrenceOutcome {
                index: 0,
                track_id: track_id.into(),
                remote_id,
                status: OccurrenceStatus::Accepted,
            }],
            revision: self.live.lock().unwrap().revision.clone(),
        }
    }
    fn updated(&self, live: &mut PlaylistRead) {
        for (index, o) in live.occurrences.iter_mut().enumerate() {
            o.index = index;
        }
        live.revision = Some(format!(
            "revision-{}",
            self.sends.fetch_add(1, Ordering::SeqCst) + 1
        ));
        if let Some(path) = &self.file {
            use std::io::Write;
            let mut file = std::fs::File::create(path).unwrap();
            file.write_all(serde_json::to_string(live).unwrap().as_bytes())
                .unwrap();
            file.sync_all().unwrap();
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.with_extension("log"))
                .unwrap();
            writeln!(log, "mutation").unwrap();
            log.sync_all().unwrap();
        }
    }
    fn persisted(path: std::path::PathBuf, ids: &[&str], remote: bool) -> Arc<Self> {
        let mut provider = Self::new(ids, remote);
        if path.exists() {
            *provider.live.lock().unwrap() =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        } else {
            std::fs::write(
                &path,
                serde_json::to_vec(&*provider.live.lock().unwrap()).unwrap(),
            )
            .unwrap();
        }
        Arc::get_mut(&mut provider).unwrap().file = Some(path);
        provider
    }
}
#[async_trait::async_trait]
impl MusicProvider for MemoryProvider {
    async fn check_playlist_editable(&self, _: &str) -> Result<(), ProviderError> {
        if self.editable.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ProviderError::new(
                crate::providers::error::ProviderErrorKind::Permission,
                crate::providers::error::SendCertainty::NotSent,
                "Playlist is read-only",
            ))
        }
    }
    fn sync_capabilities(&self) -> SyncCapabilities {
        SyncCapabilities {
            duplicate_insertion: true,
            exact_removal: true,
            occurrence_movement: true,
        }
    }
    async fn list_playlists(&self) -> Result<Vec<crate::models::Playlist>, String> {
        Ok(vec![])
    }
    async fn get_playlist_tracks(
        &self,
        _: &str,
        _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        Ok(self
            .live
            .lock()
            .unwrap()
            .occurrences
            .iter()
            .map(|o| o.track.clone())
            .collect())
    }
    async fn read_playlist(&self, _: &str) -> Result<PlaylistRead, ProviderError> {
        Ok(self.live.lock().unwrap().clone())
    }
    async fn search_track(&self, _: &str) -> Result<Vec<SourceTrack>, String> {
        Ok(vec![])
    }
    async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
        Err("not used".into())
    }
    async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        Err("not used".into())
    }
    async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
        Err("not used".into())
    }
    async fn delete_playlist(&self, _: &str) -> Result<(), String> {
        Err("not used".into())
    }
    async fn add_occurrences(
        &self,
        _: &str,
        ids: &[String],
    ) -> Result<MutationOutcome, ProviderError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .rejections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            let mut error = ProviderError::new(
                crate::providers::error::ProviderErrorKind::RateLimited,
                crate::providers::error::SendCertainty::Rejected,
                "Rate limited before acceptance",
            );
            error.retry_after_ms = Some(self.retry_ms.load(Ordering::SeqCst) as u64);
            return Err(error);
        }
        let id = &ids[0];
        let remote = Some(new_id("remote"));
        {
            let mut live = self.live.lock().unwrap();
            let index = live.occurrences.len();
            live.occurrences.push(PlaylistOccurrence {
                index,
                track: track(id),
                remote_id: remote.clone(),
            });
            self.updated(&mut live);
        }
        if self.uncertain.load(Ordering::SeqCst) {
            return Err(ProviderError::new(
                crate::providers::error::ProviderErrorKind::Timeout,
                crate::providers::error::SendCertainty::MayHaveApplied,
                "Response lost after dispatch",
            ));
        }
        if let Some(callback) = self.after_send.lock().unwrap().as_ref() {
            callback();
        }
        Ok(self.receipt(id, remote))
    }
    async fn sync_remove_occurrence(
        &self,
        _: &str,
        o: &PlaylistOccurrence,
        _: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        {
            let mut live = self.live.lock().unwrap();
            live.occurrences.remove(o.index);
            self.updated(&mut live);
        }
        Ok(self.receipt(&o.track.id, o.remote_id.clone()))
    }
    async fn move_occurrence(
        &self,
        _: &str,
        o: &PlaylistOccurrence,
        before: Option<&PlaylistOccurrence>,
        _: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        {
            let mut live = self.live.lock().unwrap();
            let item = live.occurrences.remove(o.index);
            let target = before.map_or(live.occurrences.len(), |b| {
                if o.index < b.index {
                    b.index - 1
                } else {
                    b.index
                }
            });
            live.occurrences.insert(target, item);
            self.updated(&mut live);
        }
        Ok(self.receipt(&o.track.id, o.remote_id.clone()))
    }
}

#[tokio::test]
async fn sync_run_commits_baseline_only_after_both_ordered_states_verify() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&["a"], false);
    let b = MemoryProvider::new(&["b"], true);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    for o in &mut plan.desired {
        let t = o.spotify.as_ref().or(o.ytmusic.as_ref()).unwrap().clone();
        o.spotify = Some(t.clone());
        o.ytmusic = Some(t);
    }
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let job = new_id("sync");
    let control = crate::engine::lifecycle::TransferControl::new(job, db.clone(), |_| {});
    let summary = runner::execute(db.clone(), &plan.plan_id, [a.clone(), b.clone()], control)
        .await
        .unwrap();
    assert!(summary.verified);
    assert_eq!(summary.state, "completed_verified");
    assert_eq!(a.live.lock().unwrap().ids(), vec!["a", "b"]);
    assert_eq!(b.live.lock().unwrap().ids(), vec!["a", "b"]);
    assert_eq!(
        store::get_pair(&db.lock().unwrap(), &pair.pair_id)
            .unwrap()
            .baseline
            .unwrap()
            .version,
        1
    );
}

#[tokio::test]
async fn noneditable_playlist_blocks_both_directions_before_dispatch() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&["a"], false);
    let b = MemoryProvider::new(&["a"], true);
    b.editable.store(false, Ordering::SeqCst);
    let plan = store::save_preview(
        &conn,
        &planner::initial_merge(
            &pair.pair_id,
            &a.live.lock().unwrap(),
            &b.live.lock().unwrap(),
        ),
    )
    .unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    assert!(
        runner::execute(db, &plan.plan_id, [a.clone(), b.clone()], control)
            .await
            .unwrap_err()
            .contains("read-only")
    );
    assert_eq!(
        a.sends.load(Ordering::SeqCst) + b.sends.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn uncertain_sync_is_not_replayed_even_when_membership_matches() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    a.uncertain.store(true, Ordering::SeqCst);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let job = new_id("sync");
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    let summary = runner::execute(db.clone(), &plan.plan_id, [a.clone(), b.clone()], control)
        .await
        .unwrap();
    assert_eq!(summary.state, "recovery_required");
    assert!(!summary.verified);
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    assert!(
        runner::resume(db.clone(), &job, [a.clone(), b.clone()], control)
            .await
            .is_err()
    );
    assert_eq!(a.sends.load(Ordering::SeqCst), 1);
    assert!(store::get_pair(&db.lock().unwrap(), &pair.pair_id)
        .unwrap()
        .baseline
        .is_none());
}

#[tokio::test]
async fn recorded_unsent_sync_resumes_the_same_operation_identity() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let data = runner::prepare(&pair, plan).unwrap();
    let job = new_id("sync");
    runner::start_run(&conn, &pair, &data, &job).unwrap();
    let edit = runner::next_edit(&data).unwrap();
    let payload = serde_json::to_string(&edit).unwrap();
    conn.execute("INSERT INTO transfer_operations(operation_id,job_id,operation_type,operation_index,payload_hash,payload_json,status,certainty,attempt_count,evidence_version,created_at,updated_at) VALUES('existing-intent',?1,'sync_add',0,?2,?3,'pending','unsent',0,3,0,0)",rusqlite::params![job,store::checksum(&payload),payload]).unwrap();
    crate::engine::snapshot::set_job_state(&conn, &job, "recovery_required", None).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    assert!(
        runner::resume(db.clone(), &job, [a.clone(), b], control)
            .await
            .unwrap()
            .verified
    );
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM transfer_operations WHERE job_id=?1",
                [job],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(a.sends.load(Ordering::SeqCst), 1);
}

#[test]
fn hourly_schedule_coalesces_missed_intervals_into_one_claim() {
    let conn = database();
    let mut pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    pair.baseline = Some(SyncBaseline {
        version: 1,
        occurrences: vec![],
        spotify: read(&[], false),
        ytmusic: read(&[], true),
    });
    pair.state = "synced".into();
    pair.enabled = true;
    pair.next_due_at = Some(1);
    store::save_pair(&conn, &pair).unwrap();
    let claimed = scheduler::claim_due(&conn, 36_000).unwrap().unwrap();
    assert_eq!(claimed.pair_id, pair.pair_id);
    assert_eq!(claimed.next_due_at, Some(39_600));
    assert!(scheduler::claim_due(&conn, 36_000).unwrap().is_none());
}

#[tokio::test]
async fn fresh_sync_preview_keeps_missing_matches_for_review_without_writes() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let db = Arc::new(Mutex::new(conn));
    let a = MemoryProvider::new(&["missing"], false);
    let b = MemoryProvider::new(&[], true);
    let plan = service::preview(&db, &pair.pair_id, &[a.clone(), b.clone()], None)
        .await
        .unwrap();
    assert!(plan.conflicts.iter().any(|c| c.code == "match_required"));
    assert_eq!(
        a.sends.load(Ordering::SeqCst) + b.sends.load(Ordering::SeqCst),
        0
    );
    assert!(store::load_preview(&db.lock().unwrap(), &plan.plan_id).is_ok());
}

#[tokio::test]
async fn manual_match_resolution_is_bound_to_the_original_fresh_preview() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let db = Arc::new(Mutex::new(conn));
    let a = MemoryProvider::new(&["missing"], false);
    let b = MemoryProvider::new(&[], true);
    let providers: runner::Providers = [a.clone(), b.clone()];
    let plan = service::preview(&db, &pair.pair_id, &providers, None)
        .await
        .unwrap();
    let id = plan.desired[0].logical_id.clone();
    let resolved = service::preview(
        &db,
        &pair.pair_id,
        &providers,
        Some(SyncResolution {
            plan_id: plan.plan_id,
            order: vec![id.clone()],
            choices: vec![MatchChoice {
                logical_id: id,
                side: Side::Ytmusic,
                track: track("chosen"),
            }],
            strategy: None,
        }),
    )
    .await
    .unwrap();
    assert!(resolved.conflicts.is_empty());
    assert_eq!(resolved.desired[0].ytmusic.as_ref().unwrap().id, "chosen");
    assert_eq!(
        a.sends.load(Ordering::SeqCst) + b.sends.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn conflict_review_can_keep_youtube_edits_with_fresh_validation() {
    let conn = database();
    let mut pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&["a", "b", "c"], false);
    let b = MemoryProvider::new(&["a", "b", "c"], true);
    let initial = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    pair.baseline = Some(SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a.live.lock().unwrap().clone(),
        ytmusic: b.live.lock().unwrap().clone(),
    });
    pair.state = "synced".into();
    store::save_pair(&conn, &pair).unwrap();
    {
        let mut live = a.live.lock().unwrap();
        live.occurrences.rotate_right(1);
        for (index, o) in live.occurrences.iter_mut().enumerate() {
            o.index = index;
        }
    }
    {
        let mut live = b.live.lock().unwrap();
        live.occurrences.swap(0, 1);
        for (index, o) in live.occurrences.iter_mut().enumerate() {
            o.index = index;
        }
    }
    let db = Arc::new(Mutex::new(conn));
    let providers: runner::Providers = [a, b];
    let plan = service::preview(&db, &pair.pair_id, &providers, None)
        .await
        .unwrap();
    assert!(!plan.conflicts.is_empty());
    let resolved = service::preview(
        &db,
        &pair.pair_id,
        &providers,
        Some(SyncResolution {
            plan_id: plan.plan_id,
            order: vec![],
            choices: vec![],
            strategy: Some(Side::Ytmusic),
        }),
    )
    .await
    .unwrap();
    assert!(resolved.conflicts.is_empty());
    assert_eq!(
        resolved
            .desired
            .iter()
            .map(|o| o.ytmusic.as_ref().unwrap().id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a", "c"]
    );
}

#[test]
fn sync_lifecycle_event_persists_the_same_pair_run_and_sequence_payload() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let plan = planner::initial_merge(&pair.pair_id, &read(&[], false), &read(&[], true));
    let data = runner::prepare(&pair, plan).unwrap();
    runner::start_run(&conn, &pair, &data, "sync-fixture").unwrap();
    let seq =
        crate::engine::snapshot::persist_job_state(&conn, "sync-fixture", "running", None).unwrap();
    let json:String=conn.query_row("SELECT event_payload FROM transfer_events WHERE job_id='sync-fixture' AND event_seq=?1",[seq],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({"pair_id":pair.pair_id,"job_id":"sync-fixture","state":"running","event_seq":seq})
    );
}

#[test]
fn unlink_releases_endpoints_without_deleting_playlist_or_journal_history() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    store::unlink(&conn, &pair.pair_id).unwrap();
    assert!(store::list_pairs(&conn).unwrap().is_empty());
    assert_eq!(
        store::get_pair(&conn, &pair.pair_id).unwrap().state,
        "unlinked"
    );
    assert!(store::create_pair(&conn, "a", "A", "b", "B").is_ok());
}

#[tokio::test]
async fn cancellation_drains_accepted_sync_write_and_disables_the_pair() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a", "b"], true);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    for o in &mut plan.desired {
        o.spotify = o.ytmusic.clone();
    }
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    let cancel = control.clone();
    *a.after_send.lock().unwrap() = Some(Arc::new(move || {
        cancel.cancel().unwrap();
    }));
    let summary = runner::execute(db.clone(), &plan.plan_id, [a.clone(), b], control)
        .await
        .unwrap();
    assert_eq!(summary.state, "cancelled");
    assert_eq!(a.sends.load(Ordering::SeqCst), 1);
    let pair = store::get_pair(&db.lock().unwrap(), &pair.pair_id).unwrap();
    assert_eq!(pair.state, "cancelled");
    assert!(!pair.enabled);
    assert!(pair.baseline.is_none());
}

#[tokio::test]
async fn verified_removals_retain_mapping_and_tombstone_evidence() {
    let conn = database();
    let mut pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let old = planner::initial_merge(&pair.pair_id, &read(&["a"], false), &read(&["a"], true));
    pair.baseline = Some(SyncBaseline {
        version: 1,
        occurrences: old.desired,
        spotify: old.spotify_before,
        ytmusic: old.ytmusic_before,
    });
    store::save_pair(&conn, &pair).unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&[], true);
    let plan = planner::incremental_merge(
        &pair.pair_id,
        pair.baseline.as_ref().unwrap(),
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    assert!(
        runner::execute(db.clone(), &plan.plan_id, [a, b], control)
            .await
            .unwrap()
            .verified
    );
    let pair = store::get_pair(&db.lock().unwrap(), &pair.pair_id).unwrap();
    assert_eq!(pair.tombstones.len(), 1);
    assert_eq!(pair.approved_mappings.len(), 1);
}

#[tokio::test]
async fn reviewed_mapping_to_existing_item_captures_its_remote_identity() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&["a"], false);
    let b = MemoryProvider::new(&["b"], true);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired.truncate(1);
    plan.desired[0].ytmusic = Some(track("b"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    assert!(
        runner::execute(db.clone(), &plan.plan_id, [a, b], control)
            .await
            .unwrap()
            .verified
    );
    let pair = store::get_pair(&db.lock().unwrap(), &pair.pair_id).unwrap();
    assert_eq!(
        pair.baseline.unwrap().occurrences[0]
            .ytmusic_remote_id
            .as_deref(),
        Some("b-0")
    );
}

#[test]
fn inspection_counts_every_unsent_edit_instead_of_a_boolean() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &read(&[], false),
        &read(&["a", "b", "c"], true),
    );
    for o in &mut plan.desired {
        o.spotify = o.ytmusic.clone();
    }
    let data = runner::prepare(&pair, plan).unwrap();
    runner::start_run(&conn, &pair, &data, "sync-counts").unwrap();
    assert_eq!(
        runner::inspect(&conn, "sync-counts").unwrap().counts.unsent,
        3
    );
}

#[tokio::test]
#[ignore = "subprocess fixture executed by sync_subprocess_interruptions_never_replay_mutations"]
async fn sync_crash_child() {
    let root = std::path::PathBuf::from(std::env::var("SOUNDSHIFT_SYNC_ROOT").unwrap());
    let conn = crate::storage::init_db(&root.join("app")).unwrap();
    scheduler::recover_startup(&conn, 0).unwrap();
    let a = MemoryProvider::persisted(root.join("spotify.remote"), &[], false);
    let b = MemoryProvider::persisted(root.join("youtube.remote"), &["a"], true);
    if std::env::var("SOUNDSHIFT_SYNC_PHASE").as_deref() == Ok("first") {
        conn.execute("INSERT INTO service_sessions(service_id,auth_data,updated_at) VALUES('spotify','fixture',1),('ytmusic','fixture',1)",[]).unwrap();
        let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
        let mut plan = planner::initial_merge(
            &pair.pair_id,
            &a.live.lock().unwrap(),
            &b.live.lock().unwrap(),
        );
        plan.desired[0].spotify = Some(track("a"));
        let plan = store::save_preview(&conn, &plan).unwrap();
        let db = Arc::new(Mutex::new(conn));
        let control =
            crate::engine::lifecycle::TransferControl::new("sync-crash".into(), db.clone(), |_| {});
        runner::execute(db, &plan.plan_id, [a, b], control)
            .await
            .unwrap();
    } else {
        let db = Arc::new(Mutex::new(conn));
        let control =
            crate::engine::lifecycle::TransferControl::new("sync-crash".into(), db.clone(), |_| {});
        let result = runner::resume(db.clone(), "sync-crash", [a, b], control).await;
        let unknown = matches!(
            std::env::var("SOUNDSHIFT_SYNC_BOUNDARY").unwrap().as_str(),
            "send" | "response"
        );
        if unknown {
            assert!(result.is_err());
            assert!(
                !runner::inspect(&db.lock().unwrap(), "sync-crash")
                    .unwrap()
                    .verified
            );
        } else {
            assert!(result.unwrap().verified);
        }
    }
}

#[test]
fn sync_subprocess_interruptions_never_replay_mutations() {
    for boundary in [
        "intent",
        "send",
        "response",
        "checkpoint",
        "baseline",
        "baseline_committed",
    ] {
        let root = std::env::temp_dir().join(new_id("soundshift-sync-crash"));
        std::fs::create_dir_all(&root).unwrap();
        let invoke = |phase: &str| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "sync::tests::sync_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("SOUNDSHIFT_SYNC_ROOT", &root)
                .env("SOUNDSHIFT_SYNC_PHASE", phase)
                .env("SOUNDSHIFT_SYNC_BOUNDARY", boundary);
            if phase == "first" {
                command.env("SOUNDSHIFT_SYNC_CRASH", boundary);
            } else {
                command.env_remove("SOUNDSHIFT_SYNC_CRASH");
            }
            command.output().unwrap()
        };
        let first = invoke("first");
        assert_eq!(
            first.status.code(),
            Some(91),
            "{boundary}: {}",
            String::from_utf8_lossy(&first.stdout)
        );
        for _ in 0..2 {
            let restart = invoke("restart");
            assert!(
                restart.status.success(),
                "{boundary}: {} {}",
                String::from_utf8_lossy(&restart.stdout),
                String::from_utf8_lossy(&restart.stderr)
            );
        }
        let count = std::fs::read_to_string(root.join("spotify.log"))
            .unwrap_or_default()
            .lines()
            .count();
        assert_eq!(
            count,
            usize::from(boundary != "send"),
            "{boundary}: unexpected mutation replay"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("youtube.log"))
                .unwrap_or_default()
                .lines()
                .count(),
            0
        );
        assert_eq!(
            root.canonicalize().unwrap().parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[tokio::test]
async fn proven_rejection_retries_the_same_sync_operation_after_guidance() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    a.rejections.store(1, Ordering::SeqCst);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let job = new_id("sync");
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    let started = std::time::Instant::now();
    let summary = runner::execute(db.clone(), &plan.plan_id, [a.clone(), b], control)
        .await
        .unwrap();
    assert!(summary.verified);
    assert!(started.elapsed() >= std::time::Duration::from_millis(5));
    assert_eq!(a.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(a.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM transfer_operations WHERE job_id=?1",
                [job],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn unlink_invalidates_previously_approved_previews() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let plan = store::save_preview(
        &conn,
        &planner::initial_merge(&pair.pair_id, &read(&[], false), &read(&[], true)),
    )
    .unwrap();
    store::unlink(&conn, &pair.pair_id).unwrap();
    assert!(store::load_preview(&conn, &plan.plan_id).is_err());
}

#[test]
fn malformed_duplicate_remote_identity_cannot_verify_an_ordered_state() {
    let mut invalid = read(&["a", "b"], true);
    invalid.occurrences[1].remote_id = invalid.occurrences[0].remote_id.clone();
    assert!(!runner::same_state(&invalid, &invalid));
}

#[test]
fn same_new_recording_added_on_both_sides_requires_occurrence_review() {
    let a = read(&["a"], false);
    let b = read(&["a"], true);
    let initial = planner::initial_merge("pair", &a, &b);
    let baseline = SyncBaseline {
        version: 1,
        occurrences: initial.desired,
        spotify: a,
        ytmusic: b,
    };
    let plan = planner::incremental_merge(
        "pair",
        &baseline,
        &read(&["a", "x"], false),
        &read(&["a", "x"], true),
    );
    assert!(plan
        .conflicts
        .iter()
        .any(|c| c.code == "concurrent_addition"));
}

#[tokio::test]
async fn shutdown_wins_the_shared_mutation_gate_before_new_work() {
    let gate = Arc::new(tokio::sync::Mutex::new(()));
    let closing = AtomicBool::new(false);
    let held = runtime::acquire_gate(gate.clone(), &closing).unwrap();
    assert!(runtime::acquire_gate(gate.clone(), &closing).is_err());
    drop(held);
    closing.store(true, Ordering::SeqCst);
    assert!(runtime::acquire_gate(gate, &closing).is_err());
}

#[tokio::test]
async fn reviewed_database_restore_does_not_authorize_replaying_old_sync_jobs() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&[], true);
    let plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    let data = runner::prepare(&pair, plan).unwrap();
    runner::start_run(&conn, &pair, &data, "sync-restored").unwrap();
    conn.execute("INSERT INTO transfer_manifest_state(key,value,updated_at) VALUES('restore_fenced:sync-restored','restored evidence',0),('database_restore_reviewed','reviewed',0)",[]).unwrap();
    crate::engine::snapshot::set_job_state(&conn, "sync-restored", "recovery_required", None)
        .unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new("sync-restored".into(), db.clone(), |_| {});
    assert!(runner::resume(db, "sync-restored", [a, b], control)
        .await
        .is_err());
}

#[tokio::test]
async fn repeated_verified_sync_does_not_echo_or_replay_accepted_additions() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    assert!(
        runner::execute(db.clone(), &plan.plan_id, [a.clone(), b.clone()], control)
            .await
            .unwrap()
            .verified
    );
    let providers: runner::Providers = [a.clone(), b.clone()];
    let next = service::preview(&db, &pair.pair_id, &providers, None)
        .await
        .unwrap();
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    let result = runner::execute(db, &next.plan_id, providers, control)
        .await
        .unwrap();
    assert!(result.verified);
    assert_eq!(
        result.counts.added + result.counts.removed + result.counts.moved,
        0
    );
    assert_eq!(
        a.sends.load(Ordering::SeqCst) + b.sends.load(Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn long_retry_guidance_defers_sync_without_an_early_replay() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    a.rejections.store(1, Ordering::SeqCst);
    a.retry_ms.store(60_000, Ordering::SeqCst);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let job = new_id("sync");
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runner::execute(db.clone(), &plan.plan_id, [a.clone(), b.clone()], control),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.state, "recovery_required");
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    assert!(runner::resume(db, &job, [a.clone(), b], control)
        .await
        .unwrap_err()
        .contains("deadline"));
    assert_eq!(a.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_interrupts_sync_retry_wait_before_another_attempt() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&["a"], true);
    a.rejections.store(1, Ordering::SeqCst);
    a.retry_ms.store(30_000, Ordering::SeqCst);
    let mut plan = planner::initial_merge(
        &pair.pair_id,
        &a.live.lock().unwrap(),
        &b.live.lock().unwrap(),
    );
    plan.desired[0].spotify = Some(track("a"));
    let plan = store::save_preview(&conn, &plan).unwrap();
    let db = Arc::new(Mutex::new(conn));
    let job = new_id("sync");
    let control = crate::engine::lifecycle::TransferControl::new(job.clone(), db.clone(), |_| {});
    let cancel = control.clone();
    let execute_db = db.clone();
    let send = a.clone();
    let task = tokio::spawn(async move {
        runner::execute(execute_db, &plan.plan_id, [send, b], control).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2),async{loop{let rejected=db.lock().unwrap().query_row("SELECT COUNT(*) FROM transfer_operations WHERE job_id=?1 AND status='rejected'",[&job],|r|r.get::<_,i64>(0)).unwrap();if rejected==1{break;}tokio::time::sleep(std::time::Duration::from_millis(2)).await;}}).await.unwrap();
    cancel.cancel().unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.state, "cancelled");
    assert_eq!(a.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn incomplete_source_evidence_prevents_all_sync_mutations() {
    let conn = database();
    let pair = store::create_pair(&conn, "a", "A", "b", "B").unwrap();
    let a = MemoryProvider::new(&[], false);
    let b = MemoryProvider::new(&[], true);
    let mut source = a.live.lock().unwrap().clone();
    source.complete = false;
    let plan = store::save_preview(
        &conn,
        &planner::initial_merge(&pair.pair_id, &source, &b.live.lock().unwrap()),
    )
    .unwrap();
    let db = Arc::new(Mutex::new(conn));
    let control =
        crate::engine::lifecycle::TransferControl::new(new_id("sync"), db.clone(), |_| {});
    assert!(
        runner::execute(db, &plan.plan_id, [a.clone(), b.clone()], control)
            .await
            .is_err()
    );
    assert_eq!(
        a.sends.load(Ordering::SeqCst) + b.sends.load(Ordering::SeqCst),
        0
    );
}
