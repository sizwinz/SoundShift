use crate::models::{Playlist, SourceTrack};
use crate::providers::error::ReconciliationResult;
use async_trait::async_trait;
use std::collections::HashMap;

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

    /// Reads the destination after an ambiguous mutation. Matching is a
    /// multiset comparison so duplicate playlist occurrences are preserved.
    async fn reconcile_added_tracks(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<ReconciliationResult, String> {
        let tracks = self.get_playlist_tracks(playlist_id, None).await?;
        let mut available = HashMap::new();
        for track in tracks {
            *available.entry(track.id).or_insert(0usize) += 1;
        }

        for id in track_ids {
            let count = available.entry(id.clone()).or_insert(0);
            if *count == 0 {
                return Ok(ReconciliationResult::NotApplied);
            }
            *count -= 1;
        }

        Ok(ReconciliationResult::Confirmed)
    }
}
