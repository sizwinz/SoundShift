use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceTrack {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub isrc: Option<String>,
    pub is_explicit: bool,
    pub is_playable: bool,
    pub preview_url: Option<String>,
    pub thumbnail_url: Option<String>,
}
