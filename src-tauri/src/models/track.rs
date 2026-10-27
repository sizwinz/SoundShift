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

impl SourceTrack {
    pub fn unavailable(position: usize) -> Self {
        Self {
            id: format!("unavailable_{position}"),
            title: "Unavailable Track".into(),
            artists: vec!["Unknown".into()],
            album: None,
            duration_ms: 0,
            isrc: None,
            is_explicit: false,
            is_playable: false,
            preview_url: None,
            thumbnail_url: None,
        }
    }
}
