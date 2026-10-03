use crate::engine::normalizer::{calculate_similarity, canonicalize_string, normalize_title};
use crate::engine::rate_limiter::SearchRateLimiter;
use crate::models::SourceTrack;
use crate::providers::traits::MusicProvider;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MatchClassification {
    Exact,     // Green
    Ambiguous, // Amber
    NotFound,  // Red
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchCandidate {
    pub track: SourceTrack,
    pub similarity: f64,
    pub duration_delta_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchResult {
    pub source_track: SourceTrack,
    pub status: MatchClassification,
    pub matched_track: Option<SourceTrack>,
    pub candidates: Vec<MatchCandidate>,
    pub confidence: f64,
    pub match_method: Option<String>,
}

/// Evaluates a candidate track against the source track per MTCH-03 through MTCH-06, D-05, D-06, D-07.
pub fn score_candidate(
    source: &SourceTrack,
    candidate: &SourceTrack,
    source_clean_title: &str,
    feat_artists: &[String],
) -> Option<MatchCandidate> {
    let delta_ms = if source.duration_ms > 0 && candidate.duration_ms > 0 {
        (candidate.duration_ms as i64 - source.duration_ms as i64).unsigned_abs()
    } else {
        0
    };

    // MTCH-06: Strict rejection gate for duration delta > 15 seconds
    if source.duration_ms > 0 && candidate.duration_ms > 0 && delta_ms > 15_000 {
        return None;
    }

    let (cand_clean_title, _) = normalize_title(&candidate.title);

    // Evaluate direct similarity as well as candidate title with artist prefix/suffix stripped
    let mut cand_titles_to_test = vec![cand_clean_title.clone()];
    for artist in &source.artists {
        let clean_artist = canonicalize_string(artist);
        if !clean_artist.is_empty() {
            if let Some(stripped) = cand_clean_title.strip_prefix(&format!("{} - ", clean_artist)) {
                cand_titles_to_test.push(stripped.trim().to_string());
            } else if let Some(stripped) = cand_clean_title.strip_prefix(&clean_artist) {
                cand_titles_to_test.push(stripped.trim().trim_start_matches('-').trim().to_string());
            }
            if let Some(stripped) = cand_clean_title.strip_suffix(&format!(" - {}", clean_artist)) {
                cand_titles_to_test.push(stripped.trim().to_string());
            } else if let Some(stripped) = cand_clean_title.strip_suffix(&clean_artist) {
                cand_titles_to_test.push(stripped.trim().trim_end_matches('-').trim().to_string());
            }
        }
    }

    let mut title_sim = 0.0f64;
    for ct in &cand_titles_to_test {
        let s = calculate_similarity(source_clean_title, ct);
        if s > title_sim {
            title_sim = s;
        }
    }

    let mut max_artist_sim = 0.0f64;
    for sa in &source.artists {
        for ca in &candidate.artists {
            let s = calculate_similarity(sa, ca);
            if s > max_artist_sim {
                max_artist_sim = s;
            }
        }
    }
    for fa in feat_artists {
        for ca in &candidate.artists {
            let s = calculate_similarity(fa, ca);
            if s > max_artist_sim {
                max_artist_sim = s;
            }
        }
    }
    if source.artists.is_empty() || candidate.artists.is_empty() {
        max_artist_sim = 0.85;
    }

    let overall_sim = title_sim * 0.70 + max_artist_sim * 0.30;

    // Explicit status alignment per D-06
    let final_sim = if source.is_explicit != candidate.is_explicit {
        overall_sim * 0.95
    } else {
        overall_sim
    };

    Some(MatchCandidate {
        track: candidate.clone(),
        similarity: final_sim,
        duration_delta_ms: delta_ms,
    })
}

/// Executes the 4-stage matching pipeline for a single track.
pub async fn match_track(
    source: &SourceTrack,
    source_service: &str,
    target_service: &str,
    provider: &dyn MusicProvider,
    db: Option<&Mutex<rusqlite::Connection>>,
    rate_limiter: &SearchRateLimiter,
) -> MatchResult {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Pass 1: SQLite Cache Lookup (MTCH-01)
    if let Some(db_mutex) = db {
        if let Ok(conn) = db_mutex.lock() {
            let cached_query = conn.prepare(
                "SELECT target_track_id, match_method, confidence 
                 FROM track_match_cache 
                 WHERE source_track_id = ?1 AND target_service = ?2",
            );

            if let Ok(mut stmt) = cached_query {
                let cached = stmt.query_row(params![source.id, target_service], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                });

                if let Ok((target_id, method, confidence)) = cached {
                    let mut cached_track = source.clone();
                    cached_track.id = target_id;
                    return MatchResult {
                        source_track: source.clone(),
                        status: MatchClassification::Exact,
                        matched_track: Some(cached_track),
                        candidates: Vec::new(),
                        confidence,
                        match_method: Some(format!("cache:{}", method)),
                    };
                }
            }
        }
    }

    // Pass 2: Direct ISRC Query (MTCH-02)
    if let Some(ref isrc) = source.isrc {
        if !isrc.trim().is_empty() {
            let query = format!("isrc:{}", isrc.trim());
            if let Ok(_permit) = rate_limiter.acquire().await {
                if let Ok(isrc_results) = provider.search_track(&query).await {
                    for candidate in isrc_results {
                        let delta_ms = if source.duration_ms > 0 && candidate.duration_ms > 0 {
                            (candidate.duration_ms as i64 - source.duration_ms as i64).unsigned_abs()
                        } else {
                            0
                        };

                        // ISRC match verified with duration check (delta <= 15s)
                        if delta_ms <= 15_000 {
                            if let Some(db_mutex) = db {
                                if let Ok(conn) = db_mutex.lock() {
                                    let _ = conn.execute(
                                        "INSERT OR REPLACE INTO track_match_cache 
                                         (source_service, source_track_id, target_service, target_track_id, match_method, confidence, created_at) 
                                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                        params![source_service, source.id, target_service, candidate.id, "isrc", 1.0, now],
                                    );
                                }
                            }

                            return MatchResult {
                                source_track: source.clone(),
                                status: MatchClassification::Exact,
                                matched_track: Some(candidate),
                                candidates: Vec::new(),
                                confidence: 1.0,
                                match_method: Some("isrc".to_string()),
                            };
                        }
                    }
                }
            }
        }
    }

    // Pass 3: Duration-Anchored Fuzzy Search (MTCH-03 through MTCH-06, D-05, D-06, D-07)
    let (clean_title, feat_artists) = normalize_title(&source.title);
    let primary_artist = source.artists.first().map(|s| s.as_str()).unwrap_or("");
    let search_query = format!("{} {}", clean_title, primary_artist).trim().to_string();

    let mut raw_candidates = Vec::new();
    if let Ok(_permit) = rate_limiter.acquire().await {
        if let Ok(results) = provider.search_track(&search_query).await {
            raw_candidates = results;
        }
    }

    // If initial query returned no results and we have featured artists, retry with featuring artist
    if raw_candidates.is_empty() && !feat_artists.is_empty() {
        let fallback_query = format!("{} {}", clean_title, feat_artists.join(" "));
        if let Ok(_permit) = rate_limiter.acquire().await {
            if let Ok(results) = provider.search_track(&fallback_query).await {
                raw_candidates = results;
            }
        }
    }

    // Score and filter candidates per MTCH-04, MTCH-05, MTCH-06
    let mut scored_candidates = Vec::new();
    for cand in raw_candidates {
        if let Some(scored) = score_candidate(source, &cand, &clean_title, &feat_artists) {
            scored_candidates.push(scored);
        }
    }

    // Sort: highest similarity first, then lowest duration delta
    scored_candidates.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.duration_delta_ms.cmp(&b.duration_delta_ms))
    });

    if let Some(best) = scored_candidates.first() {
        let best_track = best.track.clone();
        let best_sim = best.similarity;
        let best_delta = best.duration_delta_ms;
        let top_candidates: Vec<MatchCandidate> = scored_candidates.iter().take(3).cloned().collect();

        // Exact Match (Green - MTCH-04): delta <= 4s and similarity >= 0.85
        if best_delta <= 4_000 && best_sim >= 0.85 {
            if let Some(db_mutex) = db {
                if let Ok(conn) = db_mutex.lock() {
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO track_match_cache 
                         (source_service, source_track_id, target_service, target_track_id, match_method, confidence, created_at) 
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![source_service, source.id, target_service, best_track.id, "fuzzy", best_sim, now],
                    );
                }
            }

            return MatchResult {
                source_track: source.clone(),
                status: MatchClassification::Exact,
                matched_track: Some(best_track),
                candidates: top_candidates,
                confidence: best_sim,
                match_method: Some("fuzzy".to_string()),
            };
        }

        // Ambiguous Match (Amber - MTCH-05): 4s < delta <= 15s or 0.65 <= similarity < 0.85
        if best_delta <= 15_000 && best_sim >= 0.65 {
            return MatchResult {
                source_track: source.clone(),
                status: MatchClassification::Ambiguous,
                matched_track: Some(best_track),
                candidates: top_candidates,
                confidence: best_sim,
                match_method: Some("fuzzy".to_string()),
            };
        }
    }

    // Pass 4: Not Found (Red - MTCH-07)
    let top_candidates: Vec<MatchCandidate> = scored_candidates.into_iter().take(3).collect();
    MatchResult {
        source_track: source.clone(),
        status: MatchClassification::NotFound,
        matched_track: None,
        candidates: top_candidates,
        confidence: 0.0,
        match_method: None,
    }
}

/// Matches an entire playlist of tracks against target service.
pub async fn match_playlist(
    tracks: &[SourceTrack],
    source_service: &str,
    target_service: &str,
    provider: &dyn MusicProvider,
    db: Option<&Mutex<rusqlite::Connection>>,
    rate_limiter: &SearchRateLimiter,
) -> Vec<MatchResult> {
    let mut results = Vec::with_capacity(tracks.len());
    for track in tracks {
        let result = match_track(
            track,
            source_service,
            target_service,
            provider,
            db,
            rate_limiter,
        )
        .await;
        results.push(result);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_candidate_exact_green() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Hotel California - 2013 Remaster".into(),
            artists: vec!["Eagles".into()],
            album: Some("Hotel California".into()),
            duration_ms: 390_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let candidate = SourceTrack {
            id: "target1".into(),
            title: "Hotel California".into(),
            artists: vec!["Eagles".into()],
            album: Some("Hotel California".into()),
            duration_ms: 392_000, // Delta: 2s (<= 4s)
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored = score_candidate(&source, &candidate, &clean_title, &feat).expect("should score");

        assert!(scored.duration_delta_ms <= 4_000);
        assert!(scored.similarity >= 0.85);
    }

    #[test]
    fn test_score_candidate_reject_delta_over_15s() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Comfortably Numb".into(),
            artists: vec!["Pink Floyd".into()],
            album: None,
            duration_ms: 382_000, // 6:22
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let live_candidate = SourceTrack {
            id: "cand_live".into(),
            title: "Comfortably Numb".into(),
            artists: vec!["Pink Floyd".into()],
            album: None,
            duration_ms: 450_000, // 7:30 -> Delta: 68s (> 15s)
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored = score_candidate(&source, &live_candidate, &clean_title, &feat);

        // MTCH-06: Strict rejection must return None
        assert!(scored.is_none());
    }

    #[test]
    fn test_score_candidate_ambiguous_amber() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Starboy".into(),
            artists: vec!["The Weeknd".into()],
            album: None,
            duration_ms: 230_000,
            isrc: None,
            is_explicit: true,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let candidate = SourceTrack {
            id: "cand1".into(),
            title: "Starboy".into(),
            artists: vec!["The Weeknd".into()],
            album: None,
            duration_ms: 238_000, // Delta: 8s (4s < delta <= 15s)
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored = score_candidate(&source, &candidate, &clean_title, &feat).expect("should score");

        assert!(scored.duration_delta_ms > 4_000);
        assert!(scored.duration_delta_ms <= 15_000);
    }
}
