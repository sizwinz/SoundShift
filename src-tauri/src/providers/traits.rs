use crate::models::{Playlist, SourceTrack};
use async_trait::async_trait;

#[async_trait]
pub trait MusicProvider: Send + Sync {
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String>;
    async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String>;
    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String>;
    async fn create_playlist(&self, title: &str, description: Option<&str>) -> Result<String, String>;
    async fn add_tracks_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<(), String>;
}
