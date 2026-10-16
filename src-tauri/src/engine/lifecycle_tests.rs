use super::*;
use crate::engine::snapshot::create_pre_mutation_snapshot;
use std::time::Duration;

fn fixture() -> (
    TransferControl,
    Arc<Mutex<Connection>>,
    Arc<Mutex<Vec<LifecycleUpdate>>>,
) {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::create_tables(&conn).unwrap();
    create_pre_mutation_snapshot(
        &conn,
        "control_job",
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
    let updates = Arc::new(Mutex::new(Vec::new()));
    let capture = updates.clone();
    let control = TransferControl::new("control_job".into(), db.clone(), move |u| {
        capture.lock().unwrap().push(u)
    });
    control.start().unwrap();
    (control, db, updates)
}

#[tokio::test]
async fn pause_acknowledges_only_after_active_checkpoint() {
    let (control, db, updates) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    assert_eq!(control.pause().unwrap().state, JobState::Pausing);
    let state: String = db
        .lock()
        .unwrap()
        .query_row("SELECT job_state FROM transfer_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "pausing");
    assert!(!updates
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.state == JobState::Paused));
    permit.checkpointed().unwrap();
    assert_eq!(control.snapshot().unwrap().state, JobState::Paused);
    assert_eq!(
        updates.lock().unwrap().last().unwrap().state,
        JobState::Paused
    );
}

#[tokio::test]
async fn cancel_acknowledges_only_after_orchestrator_drains() {
    let (control, _, _) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    assert_eq!(control.cancel().unwrap().state, JobState::Cancelling);
    assert!(control.finish().await.is_err());
    assert!(control.reserve_mutation().await.unwrap().is_none());
    permit.checkpointed().unwrap();
    assert_eq!(control.snapshot().unwrap().state, JobState::Cancelling);
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn duplicate_controls_and_terminal_races_are_idempotent() {
    let (control, _, updates) = fixture();
    let paused = control.pause().unwrap();
    assert_eq!(control.pause().unwrap().event_seq, paused.event_seq);
    control.resume().unwrap();
    let done = control.finish().await.unwrap();
    for update in [
        control.pause().unwrap(),
        control.cancel().unwrap(),
        control.resume().unwrap(),
    ] {
        assert_eq!(update.state, JobState::CompletedVerified);
        assert_eq!(update.event_seq, done.event_seq);
    }
    let emitted = updates.lock().unwrap();
    assert!(emitted
        .windows(2)
        .all(|pair| pair[0].event_seq < pair[1].event_seq));
}

#[tokio::test]
async fn resume_while_pausing_prevents_a_late_pause_ack() {
    let (control, _, _) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    control.pause().unwrap();
    control.resume().unwrap();
    permit.checkpointed().unwrap();
    assert_eq!(control.snapshot().unwrap().state, JobState::Running);
}

#[tokio::test]
async fn cancellation_wakes_paused_reservations_and_cannot_be_reversed() {
    let (control, _, _) = fixture();
    control.pause().unwrap();
    let waiter = control.clone();
    let task = tokio::spawn(async move { waiter.reserve_mutation().await.unwrap().is_none() });
    tokio::task::yield_now().await;
    control.cancel().unwrap();
    assert!(control.resume().is_err());
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap());
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn failure_beats_cancel_and_persisted_error_is_redacted() {
    let (control, db, _) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    control.cancel().unwrap();
    control
        .fail("checkpoint error sp_dc=hidden_secret")
        .unwrap();
    permit.checkpointed().unwrap();
    assert!(control.finish().await.is_err());
    let (state, error): (String, String) = db
        .lock()
        .unwrap()
        .query_row("SELECT job_state, last_error FROM transfer_jobs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(state, "recovery_required");
    assert!(!error.contains("hidden_secret"));
}

#[tokio::test]
async fn abandoned_reservation_cannot_be_acknowledged_as_cancelled() {
    let (control, _, _) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    control.cancel().unwrap();
    drop(permit);
    assert_eq!(
        control.snapshot().unwrap().state,
        JobState::RecoveryRequired
    );
    assert!(control.finish().await.is_err());
}

#[tokio::test]
async fn lifecycle_persistence_failure_blocks_all_new_sends() {
    let (control, db, updates) = fixture();
    let permit = control.reserve_mutation().await.unwrap().unwrap();
    control.pause().unwrap();
    db.lock().unwrap().execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON transfer_events BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    assert!(permit.checkpointed().is_err());
    assert_eq!(
        control.snapshot().unwrap().state,
        JobState::RecoveryRequired
    );
    assert!(control.reserve_mutation().await.unwrap().is_none());
    assert!(!updates
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.state == JobState::Paused));
    db.lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_event")
        .unwrap();
    crate::engine::snapshot::reconcile_incomplete_jobs(&db.lock().unwrap()).unwrap();
    let state: String = db
        .lock()
        .unwrap()
        .query_row("SELECT job_state FROM transfer_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "recovery_required");
}

#[tokio::test]
async fn backoff_is_interrupted_by_pause_and_cancel() {
    let (control, _, _) = fixture();
    let waiting = control.clone();
    let delay = tokio::spawn(async move {
        waiting.interruptible_delay(Duration::from_secs(30)).await;
    });
    tokio::task::yield_now().await;
    control.pause().unwrap();
    tokio::time::timeout(Duration::from_secs(1), delay)
        .await
        .unwrap()
        .unwrap();
    control.cancel().unwrap();
    assert_eq!(control.finish().await.unwrap().state, JobState::Cancelled);
}

#[tokio::test]
async fn completion_waits_for_paused_job_without_lost_wakeups() {
    let (control, _, _) = fixture();
    control.pause().unwrap();
    let finishing = control.clone();
    let task = tokio::spawn(async move { finishing.finish().await });
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    control.resume().unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .state,
        JobState::CompletedVerified
    );
}
