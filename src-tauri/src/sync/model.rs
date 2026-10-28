use crate::models::SourceTrack;
use crate::providers::contracts::PlaylistRead;
use serde::{Deserialize, Serialize};

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{:032x}", rand::random::<u128>())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Spotify,
    Ytmusic,
}
impl Side {
    pub const ALL: [Side; 2] = [Side::Spotify, Side::Ytmusic];
    pub fn service(self) -> &'static str {
        match self {
            Self::Spotify => "spotify",
            Self::Ytmusic => "ytmusic",
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Spotify => Self::Ytmusic,
            Self::Ytmusic => Self::Spotify,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedOccurrence {
    pub logical_id: String,
    pub spotify: Option<SourceTrack>,
    pub ytmusic: Option<SourceTrack>,
    pub spotify_remote_id: Option<String>,
    pub ytmusic_remote_id: Option<String>,
}
impl LinkedOccurrence {
    pub fn track(&self, side: Side) -> Option<&SourceTrack> {
        match side {
            Side::Spotify => self.spotify.as_ref(),
            Side::Ytmusic => self.ytmusic.as_ref(),
        }
    }
    pub fn remote(&self, side: Side) -> Option<&str> {
        match side {
            Side::Spotify => self.spotify_remote_id.as_deref(),
            Side::Ytmusic => self.ytmusic_remote_id.as_deref(),
        }
    }
    pub fn set_track(&mut self, side: Side, track: SourceTrack) {
        match side {
            Side::Spotify => self.spotify = Some(track),
            Side::Ytmusic => self.ytmusic = Some(track),
        }
    }
    pub fn set_remote(&mut self, side: Side, id: Option<String>) {
        match side {
            Side::Spotify => self.spotify_remote_id = id,
            Side::Ytmusic => self.ytmusic_remote_id = id,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub code: String,
    pub logical_id: Option<String>,
    pub detail: String,
    pub candidates: Vec<SourceTrack>,
}
impl SyncConflict {
    pub fn new(code: &str, detail: &str) -> Self {
        Self {
            code: code.into(),
            logical_id: None,
            detail: detail.into(),
            candidates: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlan {
    pub plan_id: String,
    pub pair_id: String,
    pub baseline_version: u64,
    pub pair_stamp: String,
    pub spotify_before: PlaylistRead,
    pub ytmusic_before: PlaylistRead,
    pub desired: Vec<LinkedOccurrence>,
    pub conflicts: Vec<SyncConflict>,
    pub initial: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBaseline {
    pub version: u64,
    pub occurrences: Vec<LinkedOccurrence>,
    pub spotify: PlaylistRead,
    pub ytmusic: PlaylistRead,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEndpoint {
    pub playlist_id: String,
    pub title: String,
    pub generation: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPair {
    pub pair_id: String,
    pub spotify: SyncEndpoint,
    pub ytmusic: SyncEndpoint,
    pub baseline: Option<SyncBaseline>,
    #[serde(default)]
    pub approved_mappings: Vec<LinkedOccurrence>,
    #[serde(default)]
    pub tombstones: Vec<LinkedOccurrence>,
    pub enabled: bool,
    pub state: String,
    pub next_due_at: Option<i64>,
    pub last_run_id: Option<String>,
    pub last_error: Option<String>,
}
impl SyncPair {
    pub fn endpoint(&self, side: Side) -> &SyncEndpoint {
        match side {
            Side::Spotify => &self.spotify,
            Side::Ytmusic => &self.ytmusic,
        }
    }
}
impl Side {
    pub fn index(self) -> usize {
        match self {
            Self::Spotify => 0,
            Self::Ytmusic => 1,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncCounts {
    pub added: usize,
    pub removed: usize,
    pub moved: usize,
    pub failed: usize,
    pub uncertain: usize,
    pub unsent: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRunSummary {
    pub pair_id: String,
    pub job_id: String,
    pub state: String,
    pub event_seq: u64,
    pub counts: SyncCounts,
    pub verified: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRunData {
    pub plan: SyncPlan,
    pub expected: [PlaylistRead; 2],
    pub orders: [Vec<String>; 2],
    pub all: Vec<LinkedOccurrence>,
    pub counts: SyncCounts,
    pub verified: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditKind {
    Add,
    Remove,
    Move,
}
impl EditKind {
    pub fn operation_type(self) -> &'static str {
        match self {
            Self::Add => "sync_add",
            Self::Remove => "sync_remove",
            Self::Move => "sync_move",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEdit {
    pub side: Side,
    pub kind: EditKind,
    pub logical_id: String,
    pub at: usize,
    pub before: Option<usize>,
    pub track: SourceTrack,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchChoice {
    pub logical_id: String,
    pub side: Side,
    pub track: SourceTrack,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResolution {
    pub plan_id: String,
    pub order: Vec<String>,
    pub choices: Vec<MatchChoice>,
    pub strategy: Option<Side>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncUpdate {
    pub pair_id: String,
    pub job_id: String,
    pub state: String,
    pub event_seq: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOperationRecord {
    pub operation_id: String,
    pub side: Side,
    pub kind: EditKind,
    pub status: String,
    pub attempt_count: u32,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncInspection {
    pub summary: SyncRunSummary,
    pub operations: Vec<SyncOperationRecord>,
    pub can_resume: bool,
    pub next_action: String,
}
impl SyncPlan {
    pub fn before(&self, side: Side) -> &PlaylistRead {
        match side {
            Side::Spotify => &self.spotify_before,
            Side::Ytmusic => &self.ytmusic_before,
        }
    }
}
