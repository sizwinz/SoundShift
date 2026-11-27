use crate::engine::normalizer::{calculate_similarity, canonicalize_string, normalize_title};
use crate::engine::rate_limiter::SearchRateLimiter;
use crate::models::SourceTrack;
use crate::providers::error::{ProviderError, ProviderErrorKind, SendCertainty};
use crate::providers::traits::MusicProvider;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

#[derive(Debug, Clone)]
pub struct CachedMatchEntry {
    pub target_track_id: String,
    pub match_method: String,
    pub confidence: f64,
    pub target_track: Option<SourceTrack>,
}

#[derive(Debug, Clone)]
pub struct PendingCacheWrite {
    pub source_service: String,
    pub source_track_id: String,
    pub target_service: String,
    pub target_track_id: String,
    pub match_method: String,
    pub confidence: f64,
    pub target_track_json: Option<String>,
    pub created_at: i64,
}

/// Pre-loads cached matches for a `(source_service, target_service)` pair in a single SQLite pass.
pub fn load_match_cache_map(
    conn: &Connection,
    source_service: &str,
    target_service: &str,
) -> HashMap<String, CachedMatchEntry> {
    let mut map = HashMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT source_track_id, target_track_id, match_method, confidence, target_track_json, source_service
         FROM track_match_cache
         WHERE target_service = ?1 AND (source_service = ?2 OR source_service = '')",
    ) else {
        return map;
    };

    if let Ok(rows) = stmt.query_map(params![target_service, source_service], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    }) {
        for row in rows.flatten() {
            let (src_id, tgt_id, method, confidence, json_opt, row_src_svc) = row;
            let parsed_track = json_opt
                .as_deref()
                .and_then(|s| serde_json::from_str::<SourceTrack>(s).ok());
            let entry = CachedMatchEntry {
                target_track_id: tgt_id,
                match_method: method,
                confidence,
                target_track: parsed_track,
            };
            if row_src_svc == source_service || !map.contains_key(&src_id) {
                map.insert(src_id, entry);
            }
        }
    }
    map
}

/// Persists newly resolved matches inside one SQLite transaction.
pub fn flush_match_cache_writes(
    conn: &mut Connection,
    writes: &[PendingCacheWrite],
) -> Result<(), String> {
    if writes.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for w in writes {
        tx.execute(
            "INSERT OR REPLACE INTO track_match_cache
             (source_service, source_track_id, target_service, target_track_id, match_method, confidence, created_at, target_track_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                w.source_service,
                w.source_track_id,
                w.target_service,
                w.target_track_id,
                w.match_method,
                w.confidence,
                w.created_at,
                w.target_track_json
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
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

    let mut cand_titles_to_test = vec![cand_clean_title.clone()];
    for artist in &source.artists {
        let clean_artist = canonicalize_string(artist);
        if !clean_artist.is_empty() {
            if let Some(stripped) = cand_clean_title.strip_prefix(&format!("{} - ", clean_artist)) {
                cand_titles_to_test.push(stripped.trim().to_string());
            } else if let Some(stripped) = cand_clean_title.strip_prefix(&clean_artist) {
                cand_titles_to_test
                    .push(stripped.trim().trim_start_matches('-').trim().to_string());
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

async fn search_with_retry(
    provider: &dyn MusicProvider,
    query: &str,
    rate_limiter: &SearchRateLimiter,
) -> Result<Vec<SourceTrack>, ProviderError> {
    let mut attempt: u32 = 0;
    loop {
        let _permit = rate_limiter
            .acquire()
            .await
            .map_err(ProviderError::before_send)?;
        match provider.search_track_resilient(query).await {
            Ok(results) => return Ok(results),
            Err(err)
                if attempt < 2
                    && matches!(
                        err.kind,
                        ProviderErrorKind::RateLimited
                            | ProviderErrorKind::Timeout
                            | ProviderErrorKind::Transient
                    ) =>
            {
                drop(_permit);
                if let Some(ms) = err.retry_after_ms {
                    tokio::time::sleep(std::time::Duration::from_millis(ms.min(16_000))).await;
                } else {
                    rate_limiter.backoff(attempt).await;
                }
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

pub async fn match_track_with_cache(
    source: &SourceTrack,
    source_service: &str,
    target_service: &str,
    provider: &dyn MusicProvider,
    cached_entry: Option<&CachedMatchEntry>,
    rate_limiter: &SearchRateLimiter,
) -> Result<(MatchResult, Option<PendingCacheWrite>), ProviderError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Some(cached) = cached_entry {
        let hydrated_track = cached.target_track.clone().unwrap_or_else(|| {
            let mut t = source.clone();
            t.id = cached.target_track_id.clone();
            t
        });
        return Ok((
            MatchResult {
                source_track: source.clone(),
                status: MatchClassification::Exact,
                matched_track: Some(hydrated_track),
                candidates: Vec::new(),
                confidence: cached.confidence,
                match_method: Some(format!("cache:{}", cached.match_method)),
            },
            None,
        ));
    }

    // Pass 2: Direct ISRC Query
    if provider.supports_isrc_search() {
        if let Some(ref isrc) = source.isrc {
            if !isrc.trim().is_empty() {
                let query = format!("isrc:{}", isrc.trim());
                match search_with_retry(provider, &query, rate_limiter).await {
                    Ok(isrc_results) => {
                        for candidate in isrc_results {
                            let delta_ms = if source.duration_ms > 0 && candidate.duration_ms > 0 {
                                (candidate.duration_ms as i64 - source.duration_ms as i64)
                                    .unsigned_abs()
                            } else {
                                0
                            };

                            if delta_ms <= 15_000 {
                                let write = PendingCacheWrite {
                                    source_service: source_service.to_string(),
                                    source_track_id: source.id.clone(),
                                    target_service: target_service.to_string(),
                                    target_track_id: candidate.id.clone(),
                                    match_method: "isrc".to_string(),
                                    confidence: 1.0,
                                    target_track_json: serde_json::to_string(&candidate).ok(),
                                    created_at: now,
                                };
                                return Ok((
                                    MatchResult {
                                        source_track: source.clone(),
                                        status: MatchClassification::Exact,
                                        matched_track: Some(candidate),
                                        candidates: Vec::new(),
                                        confidence: 1.0,
                                        match_method: Some("isrc".to_string()),
                                    },
                                    Some(write),
                                ));
                            }
                        }
                    }
                    Err(err) if err.kind == ProviderErrorKind::Authentication => return Err(err),
                    Err(_) => {}
                }
            }
        }
    }

    // Pass 3: Duration-Anchored Fuzzy Search
    let (clean_title, feat_artists) = normalize_title(&source.title);
    let primary_artist = source.artists.first().map(|s| s.as_str()).unwrap_or("");
    let search_query = format!("{} {}", clean_title, primary_artist)
        .trim()
        .to_string();

    let mut raw_candidates = match search_with_retry(provider, &search_query, rate_limiter).await {
        Ok(results) => results,
        Err(err) if err.kind == ProviderErrorKind::Authentication => return Err(err),
        Err(_) => Vec::new(),
    };

    if raw_candidates.is_empty() && !feat_artists.is_empty() {
        let fallback_query = format!("{} {}", clean_title, feat_artists.join(" "));
        match search_with_retry(provider, &fallback_query, rate_limiter).await {
            Ok(results) => raw_candidates = results,
            Err(err) if err.kind == ProviderErrorKind::Authentication => return Err(err),
            Err(_) => {}
        }
    }

    let mut scored_candidates = Vec::new();
    for cand in raw_candidates {
        if let Some(scored) = score_candidate(source, &cand, &clean_title, &feat_artists) {
            scored_candidates.push(scored);
        }
    }

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
        let top_candidates: Vec<MatchCandidate> =
            scored_candidates.iter().take(3).cloned().collect();

        if best_delta <= 4_000 && best_sim >= 0.85 {
            let write = PendingCacheWrite {
                source_service: source_service.to_string(),
                source_track_id: source.id.clone(),
                target_service: target_service.to_string(),
                target_track_id: best_track.id.clone(),
                match_method: "fuzzy".to_string(),
                confidence: best_sim,
                target_track_json: serde_json::to_string(&best_track).ok(),
                created_at: now,
            };

            return Ok((
                MatchResult {
                    source_track: source.clone(),
                    status: MatchClassification::Exact,
                    matched_track: Some(best_track),
                    candidates: top_candidates,
                    confidence: best_sim,
                    match_method: Some("fuzzy".to_string()),
                },
                Some(write),
            ));
        }

        if best_delta <= 15_000 && best_sim >= 0.65 {
            return Ok((
                MatchResult {
                    source_track: source.clone(),
                    status: MatchClassification::Ambiguous,
                    matched_track: Some(best_track),
                    candidates: top_candidates,
                    confidence: best_sim,
                    match_method: Some("fuzzy".to_string()),
                },
                None,
            ));
        }
    }

    let top_candidates: Vec<MatchCandidate> = scored_candidates.into_iter().take(3).collect();
    Ok((
        MatchResult {
            source_track: source.clone(),
            status: MatchClassification::NotFound,
            matched_track: None,
            candidates: top_candidates,
            confidence: 0.0,
            match_method: None,
        },
        None,
    ))
}

/// Executes the 4-stage matching pipeline for a single track.
pub async fn match_track(
    source: &SourceTrack,
    source_service: &str,
    target_service: &str,
    provider: &dyn MusicProvider,
    db: Option<&Mutex<Connection>>,
    rate_limiter: &SearchRateLimiter,
) -> MatchResult {
    let cached = db.and_then(|db_mutex| {
        db_mutex.lock().ok().and_then(|conn| {
            let map = load_match_cache_map(&conn, source_service, target_service);
            map.get(&source.id).cloned()
        })
    });

    match match_track_with_cache(
        source,
        source_service,
        target_service,
        provider,
        cached.as_ref(),
        rate_limiter,
    )
    .await
    {
        Ok((result, maybe_write)) => {
            if let (Some(write), Some(db_mutex)) = (maybe_write, db) {
                if let Ok(mut conn) = db_mutex.lock() {
                    let _ = flush_match_cache_writes(&mut conn, &[write]);
                }
            }
            result
        }
        Err(_) => MatchResult {
            source_track: source.clone(),
            status: MatchClassification::NotFound,
            matched_track: None,
            candidates: Vec::new(),
            confidence: 0.0,
            match_method: None,
        },
    }
}

/// Matches an entire playlist of tracks against target service.
pub async fn match_playlist(
    tracks: &[SourceTrack],
    source_service: &str,
    target_service: &str,
    provider: &dyn MusicProvider,
    db: Option<&Mutex<Connection>>,
    rate_limiter: &SearchRateLimiter,
) -> Vec<MatchResult> {
    let cache_map = db
        .and_then(|db_mutex| {
            db_mutex
                .lock()
                .ok()
                .map(|conn| load_match_cache_map(&conn, source_service, target_service))
        })
        .unwrap_or_default();

    let mut results = Vec::with_capacity(tracks.len());
    let mut pending_writes = Vec::new();
    for track in tracks {
        let cached = cache_map.get(&track.id);
        match match_track_with_cache(
            track,
            source_service,
            target_service,
            provider,
            cached,
            rate_limiter,
        )
        .await
        {
            Ok((res, write_opt)) => {
                if let Some(w) = write_opt {
                    pending_writes.push(w);
                }
                results.push(res);
            }
            Err(_) => results.push(MatchResult {
                source_track: track.clone(),
                status: MatchClassification::NotFound,
                matched_track: None,
                candidates: Vec::new(),
                confidence: 0.0,
                match_method: None,
            }),
        }
    }
    if let (false, Some(db_mutex)) = (pending_writes.is_empty(), db) {
        if let Ok(mut conn) = db_mutex.lock() {
            let _ = flush_match_cache_writes(&mut conn, &pending_writes);
        }
    }
    results
}

pub fn format_auth_abort_error(target_service: &str) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Authentication,
        SendCertainty::NotSent,
        &format!(
            "Target service '{}' session expired during matching; reconnect account before continuing",
            target_service
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AuthFailMock;
    #[async_trait]
    impl MusicProvider for AuthFailMock {
        async fn list_playlists(&self) -> Result<Vec<crate::models::Playlist>, String> {
            Ok(vec![])
        }
        async fn get_playlist_tracks(
            &self,
            _: &str,
            _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
        ) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn search_track(&self, _: &str) -> Result<Vec<SourceTrack>, String> {
            Err("Unauthorized".into())
        }
        async fn search_track_resilient(&self, _: &str) -> Result<Vec<SourceTrack>, ProviderError> {
            Err(ProviderError::new(
                ProviderErrorKind::Authentication,
                SendCertainty::NotSent,
                "401 Unauthorized",
            ))
        }
        async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
            Ok("pl".into())
        }
        async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn delete_playlist(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct RetryRateLimitMock {
        attempts: AtomicUsize,
        target: SourceTrack,
    }
    #[async_trait]
    impl MusicProvider for RetryRateLimitMock {
        async fn list_playlists(&self) -> Result<Vec<crate::models::Playlist>, String> {
            Ok(vec![])
        }
        async fn get_playlist_tracks(
            &self,
            _: &str,
            _: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
        ) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![])
        }
        async fn search_track(&self, _: &str) -> Result<Vec<SourceTrack>, String> {
            Ok(vec![self.target.clone()])
        }
        async fn search_track_resilient(&self, _: &str) -> Result<Vec<SourceTrack>, ProviderError> {
            let n = self.attempts.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                let mut err = ProviderError::new(
                    ProviderErrorKind::RateLimited,
                    SendCertainty::Rejected,
                    "429",
                );
                err.retry_after_ms = Some(10);
                return Err(err);
            }
            Ok(vec![self.target.clone()])
        }
        async fn create_playlist(&self, _: &str, _: Option<&str>) -> Result<String, String> {
            Ok("pl".into())
        }
        async fn add_tracks_to_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn remove_tracks_from_playlist(&self, _: &str, _: &[String]) -> Result<(), String> {
            Ok(())
        }
        async fn delete_playlist(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

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
            duration_ms: 392_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored =
            score_candidate(&source, &candidate, &clean_title, &feat).expect("should score");

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
            duration_ms: 382_000,
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
            duration_ms: 450_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored = score_candidate(&source, &live_candidate, &clean_title, &feat);

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
            duration_ms: 238_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };

        let (clean_title, feat) = normalize_title(&source.title);
        let scored =
            score_candidate(&source, &candidate, &clean_title, &feat).expect("should score");

        assert!(scored.duration_delta_ms > 4_000);
        assert!(scored.duration_delta_ms <= 15_000);
    }

    #[tokio::test]
    async fn authentication_error_aborts_matching_instead_of_returning_not_found() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Song".into(),
            artists: vec!["Artist".into()],
            album: None,
            duration_ms: 180_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };
        let limiter = SearchRateLimiter::new(2, 0);
        let res =
            match_track_with_cache(&source, "spotify", "ytmusic", &AuthFailMock, None, &limiter)
                .await;
        assert_eq!(res.unwrap_err().kind, ProviderErrorKind::Authentication);
    }

    #[tokio::test]
    async fn rate_limited_search_retries_after_backoff_and_resolves() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Starboy".into(),
            artists: vec!["The Weeknd".into()],
            album: None,
            duration_ms: 230_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };
        let target = SourceTrack {
            id: "yt1".into(),
            title: "Starboy".into(),
            artists: vec!["The Weeknd".into()],
            album: None,
            duration_ms: 231_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: Some("https://preview.local/starboy".into()),
            thumbnail_url: Some("https://img.local/starboy.jpg".into()),
        };
        let mock = RetryRateLimitMock {
            attempts: AtomicUsize::new(0),
            target: target.clone(),
        };
        let limiter = SearchRateLimiter::new(2, 0);
        let (res, write) =
            match_track_with_cache(&source, "spotify", "ytmusic", &mock, None, &limiter)
                .await
                .unwrap();
        assert_eq!(mock.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(res.status, MatchClassification::Exact);
        assert_eq!(res.matched_track.unwrap().id, "yt1");
        assert!(write.unwrap().target_track_json.is_some());
    }

    #[tokio::test]
    async fn cached_match_hydrates_full_target_track_json() {
        let source = SourceTrack {
            id: "src1".into(),
            title: "Original Source".into(),
            artists: vec!["Source Artist".into()],
            album: None,
            duration_ms: 180_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: None,
        };
        let cached_target = SourceTrack {
            id: "yt-99".into(),
            title: "Resolved Target Version".into(),
            artists: vec!["Target Artist".into()],
            album: Some("Target Album".into()),
            duration_ms: 181_000,
            isrc: None,
            is_explicit: false,
            is_playable: true,
            preview_url: None,
            thumbnail_url: Some("https://img.local/yt99.jpg".into()),
        };
        let entry = CachedMatchEntry {
            target_track_id: "yt-99".into(),
            match_method: "manual_disambiguation".into(),
            confidence: 1.0,
            target_track: Some(cached_target.clone()),
        };
        let limiter = SearchRateLimiter::new(1, 0);
        let (res, write) = match_track_with_cache(
            &source,
            "spotify",
            "ytmusic",
            &AuthFailMock,
            Some(&entry),
            &limiter,
        )
        .await
        .unwrap();
        assert!(write.is_none());
        let matched = res.matched_track.unwrap();
        assert_eq!(matched.title, "Resolved Target Version");
        assert_eq!(
            matched.thumbnail_url.as_deref(),
            Some("https://img.local/yt99.jpg")
        );
    }
}
