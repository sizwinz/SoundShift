use crate::models::{Playlist, SourceTrack};
use crate::providers::error::ReconciliationResult;
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
    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String>;
    async fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String>;
    async fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String>;

    /// Performs a low-cost live check. Implementations may override this with a
    /// provider-specific endpoint; the default deliberately exercises auth.
    async fn canary(&self) -> Result<(), String> {
        self.list_playlists().await.map(|_| ())
    }

    /// Indicates whether the provider natively supports `isrc:XXXXX` field queries.
    /// Spotify supports ISRC queries; YouTube Music does not.
    fn supports_isrc_search(&self) -> bool {
        false
    }

    /// Without a durable before-state and provider operation identity, playlist
    /// membership cannot prove that this write created an occurrence.
    async fn reconcile_added_tracks(
        &self,
        _playlist_id: &str,
        _track_ids: &[String],
    ) -> Result<ReconciliationResult, String> {
        Ok(ReconciliationResult::Uncertain)
    }
}
