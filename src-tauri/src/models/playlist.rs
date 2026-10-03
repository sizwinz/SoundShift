use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub service: String,
    pub title: String,
    pub description: Option<String>,
    pub track_count: u32,
    pub is_public: bool,
    pub cover_url: Option<String>,
}
