//! Evidence carried across provider and durable journal boundaries.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct SyncCapabilities {
    pub duplicate_insertion: bool,
    pub exact_removal: bool,
    pub occurrence_movement: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceStatus {
    Accepted,
    Rejected,
    Skipped,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OccurrenceOutcome {
    pub index: usize,
    pub track_id: String,
    pub status: OccurrenceStatus,
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationOutcome {
    pub occurrences: Vec<OccurrenceOutcome>,
    pub revision: Option<String>,
}

impl MutationOutcome {
    pub fn matches_payload(&self, ids: &[String]) -> bool {
        let mut identities = std::collections::HashSet::new();
        self.occurrences.len() == ids.len()
            && self.occurrences.iter().enumerate().all(|(index, o)| {
                o.index == index
                    && o.track_id == ids[index]
                    && o.remote_id
                        .as_ref()
                        .is_none_or(|id| !id.is_empty() && identities.insert(id))
            })
    }
    pub fn all_accepted(&self) -> bool {
        self.occurrences
            .iter()
            .all(|o| o.status == OccurrenceStatus::Accepted)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistOccurrence {
    pub index: usize,
    pub track: crate::models::SourceTrack,
    pub remote_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistRead {
    pub occurrences: Vec<PlaylistOccurrence>,
    pub revision: Option<String>,
    pub complete: bool,
}
impl PlaylistRead {
    pub fn ids(&self) -> Vec<String> {
        self.occurrences
            .iter()
            .map(|o| o.track.id.clone())
            .collect()
    }
}

#[derive(Default)]
pub struct PageWindow {
    total: Option<usize>,
    offset: usize,
}
impl PageWindow {
    pub fn accept(&mut self, offset: usize, count: usize, total: usize) -> Result<bool, String> {
        if offset != self.offset
            || self.total.is_some_and(|n| n != total)
            || offset.saturating_add(count) > total
            || (count == 0 && offset < total)
        {
            return Err(
                "Playlist pagination drift or unexpected empty page; incomplete read".into(),
            );
        }
        self.total = Some(total);
        self.offset += count;
        Ok(self.offset == total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pagination_never_accepts_drift_or_premature_end_as_complete() {
        let mut pages = PageWindow::default();
        assert!(!pages.accept(0, 1, 2).unwrap());
        assert!(pages.accept(1, 0, 2).is_err());
        let mut changed = PageWindow::default();
        assert!(!changed.accept(0, 1, 2).unwrap());
        assert!(changed.accept(1, 1, 3).is_err());
    }
}
