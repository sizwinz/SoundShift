//! One lock linearizes control requests and outbound mutation reservations.
//! A reservation stays active until the response AND its durable checkpoint settle.
use crate::auth::keyring_store::sanitize_for_log;
use crate::engine::snapshot::persist_job_state;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Starting,
    Running,
    Pausing,
    Paused,
    Cancelling,
    Cancelled,
    CompletedVerified,
    RecoveryRequired,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
            Self::CompletedVerified => "completed_verified",
            Self::RecoveryRequired => "recovery_required",
        }
    }
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::CompletedVerified | Self::RecoveryRequired
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleUpdate {
    pub job_id: String,
    pub state: JobState,
    pub event_seq: u64,
}

struct Inner {
    update: LifecycleUpdate,
    active: usize,
}

struct Shared {
    inner: Mutex<Inner>,
    db: Arc<Mutex<Connection>>,
    changed: watch::Sender<LifecycleUpdate>,
    emit: Box<dyn Fn(LifecycleUpdate) + Send + Sync>,
}

#[derive(Clone)]
pub struct TransferControl(Arc<Shared>);

impl TransferControl {
    pub fn new(
        job_id: String,
        db: Arc<Mutex<Connection>>,
        emit: impl Fn(LifecycleUpdate) + Send + Sync + 'static,
    ) -> Self {
        let update = LifecycleUpdate {
            job_id,
            state: JobState::Starting,
            event_seq: 0,
        };
        let (changed, _) = watch::channel(update.clone());
        Self(Arc::new(Shared {
            inner: Mutex::new(Inner { update, active: 0 }),
            db,
            changed,
            emit: Box::new(emit),
        }))
    }

    fn transition(
        &self,
        inner: &mut Inner,
        state: JobState,
        error: Option<&str>,
    ) -> Result<LifecycleUpdate, String> {
        if inner.update.state == state {
            return Ok(inner.update.clone());
        }
        // Update memory before persistence: if storage fails we must still block sends.
        inner.update.state = state;
        let persisted = self
            .0
            .db
            .lock()
            .map_err(|_| "Lifecycle storage lock unavailable".to_string())
            .and_then(|conn| persist_job_state(&conn, &inner.update.job_id, state.as_str(), error));
        match persisted {
            Ok(seq) => {
                inner.update.event_seq = seq;
                self.0.changed.send_replace(inner.update.clone());
                (self.0.emit)(inner.update.clone());
                Ok(inner.update.clone())
            }
            Err(error) => {
                inner.update.state = JobState::RecoveryRequired;
                self.0.changed.send_replace(inner.update.clone());
                Err(format!(
                    "Lifecycle checkpoint failed; recovery required: {}",
                    sanitize_for_log(&error)
                ))
            }
        }
    }

    pub fn snapshot(&self) -> Result<LifecycleUpdate, String> {
        self.0
            .inner
            .lock()
            .map(|inner| inner.update.clone())
            .map_err(|_| "Lifecycle lock unavailable".into())
    }

    /// Called after the immutable baseline and manifest exist, before any send.
    pub fn start(&self) -> Result<LifecycleUpdate, String> {
        let mut inner = self
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        if inner.update.state != JobState::Starting {
            return Err("Transfer already initialized".into());
        }
        self.transition(&mut inner, JobState::Running, None)
    }

    pub fn pause(&self) -> Result<LifecycleUpdate, String> {
        let mut inner = self
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        match inner.update.state {
            JobState::Starting => {
                return Err(
                    "Preparing the durable baseline; controls are available when ready".into(),
                )
            }
            JobState::Running => {
                self.transition(&mut inner, JobState::Pausing, None)?;
            }
            _ => return Ok(inner.update.clone()),
        }
        if inner.active == 0 {
            self.transition(&mut inner, JobState::Paused, None)
        } else {
            Ok(inner.update.clone())
        }
    }

    pub fn resume(&self) -> Result<LifecycleUpdate, String> {
        let mut inner = self
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        match inner.update.state {
            JobState::Pausing | JobState::Paused => {
                self.transition(&mut inner, JobState::Running, None)
            }
            JobState::Cancelling => Err("Cancellation cannot be reversed".into()),
            JobState::Starting => Err("Preparing the durable baseline".into()),
            _ => Ok(inner.update.clone()),
        }
    }

    pub fn cancel(&self) -> Result<LifecycleUpdate, String> {
        let mut inner = self
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        if inner.update.state.terminal() || inner.update.state == JobState::Cancelling {
            return Ok(inner.update.clone());
        }
        if inner.update.state == JobState::Starting {
            return Err("Preparing the durable baseline; controls are available when ready".into());
        }
        self.transition(&mut inner, JobState::Cancelling, None)
    }

    pub fn fail(&self, error: &str) -> Result<LifecycleUpdate, String> {
        let mut inner = self
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        // Successful terminal states cannot be rewritten by a late control response.
        if inner.update.state.terminal() {
            return Ok(inner.update.clone());
        }
        self.transition(&mut inner, JobState::RecoveryRequired, Some(error))
    }

    pub fn is_stopping(&self) -> bool {
        self.snapshot()
            .map(|u| {
                matches!(
                    u.state,
                    JobState::Cancelling | JobState::Cancelled | JobState::RecoveryRequired
                )
            })
            .unwrap_or(true)
    }

    pub async fn wait_until_ready(&self) -> Result<bool, String> {
        let mut changed = self.0.changed.subscribe();
        loop {
            match self.snapshot()?.state {
                JobState::Running => return Ok(true),
                JobState::Pausing | JobState::Paused => {}
                JobState::Starting => return Err("Transfer has no durable baseline".into()),
                _ => return Ok(false),
            }
            changed
                .changed()
                .await
                .map_err(|_| "Lifecycle channel closed")?;
        }
    }

    pub async fn reserve_mutation(&self) -> Result<Option<MutationPermit>, String> {
        let mut changed = self.0.changed.subscribe();
        loop {
            {
                let mut inner = self
                    .0
                    .inner
                    .lock()
                    .map_err(|_| "Lifecycle lock unavailable")?;
                match inner.update.state {
                    JobState::Running => {
                        inner.active += 1;
                        return Ok(Some(MutationPermit {
                            control: self.clone(),
                            settled: false,
                        }));
                    }
                    JobState::Pausing | JobState::Paused => {}
                    JobState::Starting => return Err("Transfer has no durable baseline".into()),
                    _ => return Ok(None),
                }
            }
            changed
                .changed()
                .await
                .map_err(|_| "Lifecycle channel closed")?;
        }
    }

    pub async fn interruptible_delay(&self, duration: std::time::Duration) {
        let mut changed = self.0.changed.subscribe();
        if self
            .snapshot()
            .map(|u| u.state != JobState::Running)
            .unwrap_or(true)
        {
            return;
        }
        tokio::select! { _ = tokio::time::sleep(duration) => {}, _ = changed.changed() => {} }
    }

    /// Only the orchestrator calls this after every worker has drained.
    pub async fn finish(&self) -> Result<LifecycleUpdate, String> {
        let mut changed = self.0.changed.subscribe();
        loop {
            {
                let mut inner = self
                    .0
                    .inner
                    .lock()
                    .map_err(|_| "Lifecycle lock unavailable")?;
                if inner.active != 0 {
                    return Err("Cannot finalize while mutations are active".into());
                }
                match inner.update.state {
                    JobState::Running => {
                        return self.transition(&mut inner, JobState::CompletedVerified, None)
                    }
                    JobState::Cancelling => {
                        return self.transition(&mut inner, JobState::Cancelled, None)
                    }
                    JobState::Paused | JobState::Pausing => {}
                    JobState::RecoveryRequired => {
                        return Err(
                            "Transfer requires recovery; completion was not acknowledged".into(),
                        )
                    }
                    _ => return Ok(inner.update.clone()),
                }
            }
            changed
                .changed()
                .await
                .map_err(|_| "Lifecycle channel closed")?;
        }
    }
}

pub struct MutationPermit {
    control: TransferControl,
    settled: bool,
}

impl MutationPermit {
    /// Caller must save the provider outcome before acknowledging settlement.
    pub fn checkpointed(mut self) -> Result<(), String> {
        self.settled = true;
        self.release(false)
    }

    fn release(&self, abandoned: bool) -> Result<(), String> {
        let mut inner = self
            .control
            .0
            .inner
            .lock()
            .map_err(|_| "Lifecycle lock unavailable")?;
        inner.active = inner.active.saturating_sub(1);
        if abandoned {
            self.control.transition(
                &mut inner,
                JobState::RecoveryRequired,
                Some("Mutation task ended before its outcome checkpoint"),
            )?;
        } else if inner.active == 0 && inner.update.state == JobState::Pausing {
            self.control
                .transition(&mut inner, JobState::Paused, None)?;
        }
        Ok(())
    }
}

impl Drop for MutationPermit {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.release(true);
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
