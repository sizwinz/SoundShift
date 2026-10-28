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
}
impl MemoryProvider {
    fn new(ids: &[&str], remote: bool) -> Arc<Self> {
        Arc::new(Self {
            live: Mutex::new(read(ids, remote)),
            sends: AtomicUsize::new(0),
            editable: AtomicBool::new(true),
            uncertain: AtomicBool::new(false),
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
