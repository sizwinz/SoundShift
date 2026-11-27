use crate::models::{Playlist, SourceTrack};
use crate::providers::contracts::{MutationOutcome, PlaylistOccurrence, PlaylistRead};
use crate::providers::error::{
    ProviderError, ProviderErrorKind, ReconciliationResult, SendCertainty,
};
use async_trait::async_trait;

#[async_trait]
pub trait MusicProvider: Send + Sync {
    async fn check_playlist_editable(&self, _playlist_id: &str) -> Result<(), ProviderError> {
        Err(ProviderError::new(
            ProviderErrorKind::Permission,
            SendCertainty::NotSent,
            "Playlist editability is unverified; no sync writes permitted",
        ))
    }
    fn sync_capabilities(&self) -> crate::providers::contracts::SyncCapabilities {
        crate::providers::contracts::SyncCapabilities::default()
    }
    async fn move_occurrence(
        &self,
        _playlist_id: &str,
        _occurrence: &PlaylistOccurrence,
        _before: Option<&PlaylistOccurrence>,
        _baseline: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        Err(ProviderError::new(
            ProviderErrorKind::Unknown,
            SendCertainty::NotSent,
            "Occurrence movement unsupported; no mutation sent",
        ))
    }
    async fn sync_remove_occurrence(
        &self,
        playlist_id: &str,
        occurrence: &PlaylistOccurrence,
        baseline: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        if !baseline.complete || !self.sync_capabilities().exact_removal {
            return Err(ProviderError::new(
                ProviderErrorKind::Unknown,
                SendCertainty::NotSent,
                "Exact removal unsupported; no mutation sent",
            ));
        }
        self.remove_occurrences(
            playlist_id,
            std::slice::from_ref(occurrence),
            baseline.revision.as_deref(),
        )
        .await?;
        Ok(MutationOutcome {
            occurrences: vec![crate::providers::contracts::OccurrenceOutcome {
                index: 0,
                track_id: occurrence.track.id.clone(),
                status: crate::providers::contracts::OccurrenceStatus::Accepted,
                remote_id: occurrence.remote_id.clone(),
            }],
            revision: None,
        })
    }
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String>;
    async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String>;
    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String>;
    async fn search_track_resilient(&self, query: &str) -> Result<Vec<SourceTrack>, ProviderError> {
        self.search_track(query)
            .await
            .map_err(ProviderError::before_send)
    }
    async fn fetch_track_by_id(
        &self,
        _track_id: &str,
    ) -> Result<Option<SourceTrack>, ProviderError> {
        Ok(None)
    }
    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String>;
    async fn create_playlist_checked(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, ProviderError> {
        self.create_playlist(title, description)
            .await
            .map_err(|err| {
                let kind = crate::providers::error::classify_error(&err);
                let lower = err.to_ascii_lowercase();
                let certainty = if matches!(
                    kind,
                    ProviderErrorKind::Authentication
                        | ProviderErrorKind::Permission
                        | ProviderErrorKind::RateLimited
                ) || lower.contains("400")
                    || lower.contains("404")
                    || lower.contains("422")
                    || lower.contains("bad request")
                {
                    SendCertainty::Rejected
                } else {
                    SendCertainty::MayHaveApplied
                };
                ProviderError::new(kind, certainty, &err)
            })
    }
    async fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String>;
    async fn add_occurrences_with_options(
        &self,
        playlist_id: &str,
        ids: &[String],
        _skip_duplicates: bool,
    ) -> Result<MutationOutcome, ProviderError> {
        self.add_occurrences(playlist_id, ids).await
    }
    async fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String>;

    async fn playlist_revision(&self, _playlist_id: &str) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    /// Two complete reads detect observable drift; absent revision/occurrence evidence
    /// never grants permission to attribute an ambiguous mutation or remove duplicates.
    async fn read_playlist(&self, playlist_id: &str) -> Result<PlaylistRead, ProviderError> {
        let before_revision = self.playlist_revision(playlist_id).await?;
        let before = self
            .get_playlist_tracks(playlist_id, None)
            .await
            .map_err(ProviderError::before_send)?;
        let after = self
            .get_playlist_tracks(playlist_id, None)
            .await
            .map_err(ProviderError::before_send)?;
        let revision = self.playlist_revision(playlist_id).await?;
        if before_revision != revision
            || before.len() != after.len()
            || before.iter().zip(&after).any(|(a, b)| a.id != b.id)
        {
            return Err(ProviderError::new(
                ProviderErrorKind::Conflict,
                SendCertainty::NotSent,
                "Playlist changed during read; no mutation sent",
            ));
        }
        if after
            .iter()
            .any(|t| t.id.is_empty() || t.id.starts_with("unavailable_"))
        {
            return Err(ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Playlist contains unresolved occurrences; complete baseline unavailable",
            ));
        }
        Ok(PlaylistRead {
            occurrences: after
                .into_iter()
                .enumerate()
                .map(|(index, track)| PlaylistOccurrence {
                    index,
                    track,
                    remote_id: None,
                })
                .collect(),
            revision,
            complete: true,
        })
    }

    /// Defaults refuse unsupported writes; legacy success is not occurrence evidence.
    async fn add_occurrences(
        &self,
        _playlist_id: &str,
        _ids: &[String],
    ) -> Result<MutationOutcome, ProviderError> {
        Err(ProviderError::new(
            ProviderErrorKind::Unknown,
            SendCertainty::NotSent,
            "Provider occurrence outcomes are unsupported; no mutation sent",
        ))
    }

    fn supports_occurrence_removal(&self) -> bool {
        false
    }
    async fn remove_occurrences(
        &self,
        _playlist_id: &str,
        _occurrences: &[PlaylistOccurrence],
        _revision: Option<&str>,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::new(
            ProviderErrorKind::Unknown,
            SendCertainty::NotSent,
            "Exact occurrence removal unavailable; manual review required",
        ))
    }
    fn supports_verified_deletion(&self) -> bool {
        false
    }
    async fn verify_playlist_removed(&self, _playlist_id: &str) -> Result<bool, ProviderError> {
        Ok(false)
    }

    async fn check_session(&self) -> Result<(), ProviderError> {
        self.canary().await.map_err(ProviderError::before_send)
    }

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
