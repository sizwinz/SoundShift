use crate::models::{Playlist, SourceTrack};
use crate::providers::contracts::{
    MutationOutcome, OccurrenceOutcome, OccurrenceStatus, PlaylistOccurrence, PlaylistRead,
};
use crate::providers::error::{ProviderError, ProviderErrorKind, SendCertainty};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, COOKIE, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct SpotifySessionCache {
    inner: Arc<RwLock<Option<CachedSpotifySession>>>,
}

#[derive(Clone)]
struct CachedSpotifySession {
    sp_dc: String,
    access_token: String,
    expires_at_ms: u64,
    me_user_id: Option<String>,
}

impl SpotifySessionCache {
    /// Clears any cached Spotify bearer token and profile identity.
    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }

    async fn access_token_for(&self, sp_dc: &str, now_ms: u64) -> Option<String> {
        let guard = self.inner.read().await;
        let cached = guard.as_ref()?;
        if cached.sp_dc == sp_dc && now_ms.saturating_add(60_000) < cached.expires_at_ms {
            Some(cached.access_token.clone())
        } else {
            None
        }
    }

    async fn set_access_token(&self, sp_dc: String, access_token: String, expires_at_ms: u64) {
        let mut guard = self.inner.write().await;
        let me_user_id = guard
            .as_ref()
            .filter(|existing| existing.sp_dc == sp_dc)
            .and_then(|existing| existing.me_user_id.clone());
        *guard = Some(CachedSpotifySession {
            sp_dc,
            access_token,
            expires_at_ms,
            me_user_id,
        });
    }

    async fn me_user_id_for(&self, sp_dc: &str) -> Option<String> {
        let guard = self.inner.read().await;
        let cached = guard.as_ref()?;
        if cached.sp_dc == sp_dc {
            cached.me_user_id.clone()
        } else {
            None
        }
    }

    async fn set_me_user_id(&self, sp_dc: &str, user_id: String) {
        let mut guard = self.inner.write().await;
        if let Some(cached) = guard.as_mut() {
            if cached.sp_dc == sp_dc {
                cached.me_user_id = Some(user_id);
            }
        }
    }

    async fn invalidate(&self, sp_dc: &str) {
        let mut guard = self.inner.write().await;
        if guard.as_ref().is_some_and(|c| c.sp_dc == sp_dc) {
            *guard = None;
        }
    }
}

#[derive(Clone)]
pub struct SpotifyProvider {
    client: reqwest::Client,
    sp_dc: String,
    access_token: Arc<RwLock<Option<String>>>,
    me_user_id: Arc<RwLock<Option<String>>>,
    session_cache: SpotifySessionCache,
    #[cfg(test)]
    mutation_url: Option<String>,
}

impl SpotifyProvider {
    fn playlist_items_url(&self, playlist_id: &str) -> String {
        let base = "https://api.spotify.com";
        #[cfg(test)]
        let base = self.mutation_url.as_deref().unwrap_or(base);
        format!("{base}/v1/playlists/{playlist_id}/items")
    }
    pub fn new(sp_dc: String) -> Self {
        Self::with_session_cache(sp_dc, SpotifySessionCache::default())
    }

    /// Builds a Spotify provider backed by a shared session auth cache.
    pub fn with_session_cache(sp_dc: String, session_cache: SpotifySessionCache) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(8))
                .timeout(std::time::Duration::from_secs(15))
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .tcp_keepalive(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            sp_dc,
            access_token: Arc::new(RwLock::new(None)),
            me_user_id: Arc::new(RwLock::new(None)),
            session_cache,
            #[cfg(test)]
            mutation_url: None,
        }
    }

    fn rest_headers(&self, token: &str) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|_| {
                ProviderError::new(
                    ProviderErrorKind::Authentication,
                    SendCertainty::NotSent,
                    "Invalid Spotify session header; re-authenticate",
                )
            })?,
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            HeaderValue::from_static("https://open.spotify.com"),
        );
        headers.insert(
            reqwest::header::REFERER,
            HeaderValue::from_static("https://open.spotify.com/"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
            ),
        );
        Ok(headers)
    }

    fn pathfinder_action(operation: &str) -> &'static str {
        match operation {
            "fetchLibraryTracks" => "checking session",
            "fetchPlaylist" => "reading playlist",
            "libraryV3" => "loading playlists",
            "searchDesktop" => "searching tracks",
            _ => "querying catalog",
        }
    }

    /// Obtains an active bearer access token, exchanging sp_dc session cookie via Spotify Web Player TOTP.
    pub async fn get_access_token(&self) -> Result<String, String> {
        {
            let read_guard = self.access_token.read().await;
            if let Some(token) = read_guard.as_ref() {
                return Ok(token.clone());
            }
        }

        // Direct token case if sp_dc is already formatted as bearer token
        if self.sp_dc.starts_with("BQ") && self.sp_dc.len() > 50 {
            let mut write_guard = self.access_token.write().await;
            *write_guard = Some(self.sp_dc.clone());
            return Ok(self.sp_dc.clone());
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if let Some(cached) = self
            .session_cache
            .access_token_for(&self.sp_dc, now_ms)
            .await
        {
            let mut write_guard = self.access_token.write().await;
            *write_guard = Some(cached.clone());
            return Ok(cached);
        }

        // Synchronize with Spotify server time for exact clock alignment
        let mut server_time_secs = now_ms / 1000;
        if let Ok(time_res) = self
            .client
            .get("https://open.spotify.com/api/server-time")
            .send()
            .await
        {
            if let Ok(time_json) = time_res.json::<Value>().await {
                if let Some(st) = time_json["serverTime"].as_u64() {
                    server_time_secs = st;
                }
            }
        }

        let totp = generate_spotify_totp(now_ms);
        let server_totp = generate_spotify_totp(server_time_secs * 1000);

        let mut headers = HeaderMap::new();
        let cookie_str = format!("sp_dc={}", self.sp_dc);
        if let Ok(val) = HeaderValue::from_str(&cookie_str) {
            headers.insert(COOKIE, val);
        }
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
            ),
        );
        headers.insert(
            reqwest::header::REFERER,
            HeaderValue::from_static("https://open.spotify.com/"),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            HeaderValue::from_static("https://open.spotify.com"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );

        let url = format!(
            "https://open.spotify.com/api/token?reason=init&productType=web_player&totp={}&totpServer={}&totpVer=61",
            totp, server_totp
        );

        let res = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("Spotify token exchange request failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                self.invalidate_access_token().await;
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::read_http_for(
                    "Spotify",
                    "refreshing session token",
                    status.as_u16(),
                    res.headers(),
                )
                .to_string());
            }

            return Err(format!("Spotify token exchange error ({})", status));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Spotify token JSON: {}", e))?;

        let token = json["accessToken"]
            .as_str()
            .ok_or_else(|| "Missing accessToken in Spotify response".to_string())?
            .to_string();

        let expires_at_ms = json["accessTokenExpirationTimestampMs"]
            .as_u64()
            .unwrap_or_else(|| now_ms.saturating_add(15 * 60_000));

        self.session_cache
            .set_access_token(self.sp_dc.clone(), token.clone(), expires_at_ms)
            .await;

        let mut write_guard = self.access_token.write().await;
        *write_guard = Some(token.clone());
        Ok(token)
    }

    async fn fetch_tracks(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, ProviderError> {
        let mut all_tracks = Vec::new();
        let mut offset = 0;
        let mut pages = crate::providers::contracts::PageWindow::default();

        if playlist_id == "collection:tracks"
            || playlist_id == "me:liked"
            || playlist_id.contains("collection")
        {
            // Liked Songs via fetchLibraryTracks
            let limit = 50;
            loop {
                let variables = serde_json::json!({
                    "offset": offset,
                    "limit": limit
                });

                let json = self
                    .pathfinder_typed(
                        "fetchLibraryTracks",
                        "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240",
                        variables,
                    )
                    .await?;

                let tracks_node = &json["data"]["me"]["library"]["tracks"];
                let items = match tracks_node["items"].as_array() {
                    Some(arr) => arr,
                    None => {
                        return Err(ProviderError::new(
                            ProviderErrorKind::MalformedResponse,
                            SendCertainty::NotSent,
                            "Spotify playlist page schema mismatch; incomplete read",
                        ))
                    }
                };

                let total = tracks_node["totalCount"].as_u64().ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Playlist total missing; incomplete read",
                    )
                })? as usize;
                let complete = pages.accept(offset, items.len(), total).map_err(|e| {
                    ProviderError::new(ProviderErrorKind::Pagination, SendCertainty::NotSent, &e)
                })?;
                let mut chunk = Vec::new();
                for item in items {
                    let track_data = &item["track"]["data"];
                    let fallback_uri = item["track"]["_uri"].as_str();
                    chunk.push(
                        Self::parse_pathfinder_track(track_data, fallback_uri)
                            .unwrap_or_else(|| SourceTrack::unavailable(offset + chunk.len())),
                    );
                }

                if let Some(ref sender) = tx {
                    let _ = sender.send(chunk.clone()).await;
                }

                all_tracks.extend(chunk);

                offset += items.len();
                if complete {
                    break;
                }
            }
        } else {
            // Standard playlist via fetchPlaylist
            let limit = 100;
            let uri = if playlist_id.starts_with("spotify:playlist:") {
                playlist_id.to_string()
            } else {
                format!("spotify:playlist:{}", playlist_id)
            };

            loop {
                let variables = serde_json::json!({
                    "uri": uri,
                    "offset": offset,
                    "limit": limit,
                    "enableWatchFeedEntrypoint": false
                });

                let json = self
                    .pathfinder_typed(
                        "fetchPlaylist",
                        "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77",
                        variables,
                    )
                    .await?;

                let content = &json["data"]["playlistV2"]["content"];
                let items = match content["items"].as_array() {
                    Some(arr) => arr,
                    None => {
                        return Err(ProviderError::new(
                            ProviderErrorKind::MalformedResponse,
                            SendCertainty::NotSent,
                            "Spotify playlist page schema mismatch; incomplete read",
                        ))
                    }
                };

                let total = content["totalCount"].as_u64().ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Playlist total missing; incomplete read",
                    )
                })? as usize;
                let complete = pages.accept(offset, items.len(), total).map_err(|e| {
                    ProviderError::new(ProviderErrorKind::Pagination, SendCertainty::NotSent, &e)
                })?;
                let mut chunk = Vec::new();
                for item in items {
                    let item_v2_data = &item["itemV2"]["data"];
                    if item_v2_data.is_null() {
                        // Retain unavailable/region-restricted source tracks per D-03
                        chunk.push(SourceTrack {
                            id: format!("unavailable_{}", offset + chunk.len()),
                            title: "Unavailable Track".to_string(),
                            artists: vec!["Unknown".to_string()],
                            album: None,
                            duration_ms: 0,
                            isrc: None,
                            is_explicit: false,
                            is_playable: false,
                            preview_url: None,
                            thumbnail_url: None,
                        });
                    } else {
                        let fallback_uri = item_v2_data["uri"]
                            .as_str()
                            .or_else(|| item["itemV3"]["data"]["uri"].as_str());
                        chunk.push(
                            Self::parse_pathfinder_track(item_v2_data, fallback_uri)
                                .unwrap_or_else(|| SourceTrack::unavailable(offset + chunk.len())),
                        );
                    }
                }

                if let Some(ref sender) = tx {
                    let _ = sender.send(chunk.clone()).await;
                }

                all_tracks.extend(chunk);

                offset += items.len();
                if complete {
                    break;
                }
            }
        }

        Ok(all_tracks)
    }

    pub async fn invalidate_access_token(&self) {
        let mut write_guard = self.access_token.write().await;
        *write_guard = None;
        let mut id_guard = self.me_user_id.write().await;
        *id_guard = None;
        self.session_cache.invalidate(&self.sp_dc).await;
    }

    async fn send_authed_get_json(&self, url: &str, action: &str) -> Result<Value, ProviderError> {
        let mut attempt = 0u32;
        loop {
            let token = self
                .get_access_token()
                .await
                .map_err(ProviderError::before_send)?;
            let headers = self.rest_headers(&token)?;
            let response = self
                .client
                .get(url)
                .headers(headers)
                .send()
                .await
                .map_err(ProviderError::read_transport)?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                self.invalidate_access_token().await;
            }
            if response.status().is_success() {
                return response.json::<Value>().await.map_err(|_| {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Spotify read response JSON malformed",
                    )
                });
            }
            let status = response.status().as_u16();
            let err = ProviderError::read_http_for("Spotify", action, status, response.headers());
            if status == 429 && attempt < 2 {
                let wait_ms = err.retry_after_ms.unwrap_or(1_200);
                if wait_ms <= 4_000 {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                    continue;
                }
            }
            return Err(err);
        }
    }

    /// Executes a GraphQL query against Spotify's internal Pathfinder Partner API.
    pub async fn pathfinder_query(
        &self,
        operation: &str,
        hash: &str,
        variables: Value,
    ) -> Result<Value, String> {
        self.pathfinder_typed(operation, hash, variables)
            .await
            .map_err(|e| e.to_string())
    }

    async fn pathfinder_typed(
        &self,
        operation: &str,
        hash: &str,
        variables: Value,
    ) -> Result<Value, ProviderError> {
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let action = Self::pathfinder_action(operation);
        let payload = serde_json::json!({
            "variables": variables,
            "operationName": operation,
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": hash
                }
            }
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|_| {
                ProviderError::new(
                    ProviderErrorKind::Authentication,
                    SendCertainty::NotSent,
                    "Invalid session header; re-authenticate",
                )
            })?,
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("app-platform"),
            HeaderValue::from_static("WebPlayer"),
        );
        headers.insert(
            HeaderName::from_static("spotify-app-version"),
            HeaderValue::from_static("1.2.87.311.g2db0c2c4"),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            HeaderValue::from_static("https://open.spotify.com"),
        );
        headers.insert(
            reqwest::header::REFERER,
            HeaderValue::from_static("https://open.spotify.com/"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
            ),
        );

        let endpoint = "https://api-partner.spotify.com/pathfinder/v1/query";
        #[cfg(test)]
        let endpoint = self.mutation_url.as_deref().unwrap_or(endpoint);
        let mut attempt = 0u32;
        let res = loop {
            let res = self
                .client
                .post(endpoint)
                .headers(headers.clone())
                .json(&payload)
                .send()
                .await
                .map_err(ProviderError::read_transport)?;
            if res.status().is_success() {
                break res;
            }
            let status = res.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                self.invalidate_access_token().await;
            }
            let err =
                ProviderError::read_http_for("Spotify", action, status.as_u16(), res.headers());
            if status.as_u16() == 429 && attempt < 2 {
                let wait_ms = err.retry_after_ms.unwrap_or(1_200);
                if wait_ms <= 4_000 {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                    continue;
                }
            }
            return Err(err);
        };
        let json: Value = res.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Spotify playlist response schema invalid",
            )
        })?;

        if let Some(errors) = json.get("errors") {
            if !errors.is_null() {
                if let Some(err_arr) = errors.as_array() {
                    if !err_arr.is_empty() {
                        return Err(ProviderError::new(
                            ProviderErrorKind::MalformedResponse,
                            SendCertainty::NotSent,
                            "Spotify GraphQL response rejected; no mutation sent",
                        ));
                    }
                }
            }
        }

        Ok(json)
    }

    /// Fetches the total track count for a given Spotify playlist using fetchPlaylist.
    pub async fn fetch_playlist_track_count(&self, playlist_id: &str) -> Option<u32> {
        let uri = if playlist_id.starts_with("spotify:playlist:") {
            playlist_id.to_string()
        } else {
            format!("spotify:playlist:{}", playlist_id)
        };

        let variables = serde_json::json!({
            "uri": uri,
            "offset": 0,
            "limit": 1,
            "enableWatchFeedEntrypoint": false
        });

        let json = self
            .pathfinder_query(
                "fetchPlaylist",
                "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77",
                variables,
            )
            .await
            .ok()?;

        json["data"]["playlistV2"]["content"]["totalCount"]
            .as_u64()
            .map(|c| c as u32)
    }

    /// Normalizes track entities returned by Pathfinder queries (fetchPlaylist, fetchLibraryTracks, searchDesktop).
    pub fn parse_pathfinder_track(data: &Value, fallback_uri: Option<&str>) -> Option<SourceTrack> {
        let uri = data["uri"].as_str().or(fallback_uri).unwrap_or("");

        let id = if let Some(stripped) = uri.strip_prefix("spotify:track:") {
            stripped.to_string()
        } else if let Some(raw_id) = data["id"].as_str() {
            raw_id.to_string()
        } else if !uri.is_empty() {
            uri.to_string()
        } else {
            return None;
        };

        let title = data["name"].as_str().unwrap_or("Unknown").to_string();

        let artists = data["artists"]["items"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a["profile"]["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let album = data["albumOfTrack"]["name"].as_str().map(|s| s.to_string());

        let duration_ms = data["trackDuration"]["totalMilliseconds"]
            .as_u64()
            .or_else(|| data["duration"]["totalMilliseconds"].as_u64())
            .unwrap_or(0);

        let isrc = data["externalIds"]["isrc"]
            .as_str()
            .or_else(|| data["isrc"].as_str())
            .map(|s| s.to_string());

        let is_explicit = data["contentRating"]["label"]
            .as_str()
            .map(|l| l.eq_ignore_ascii_case("EXPLICIT"))
            .unwrap_or(false);

        let is_playable = data["playability"]["playable"].as_bool().unwrap_or(true);

        let preview_url = data["preview_url"]
            .as_str()
            .or_else(|| {
                data["associationsV3"]["audioAssociations"]["items"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item["url"].as_str())
            })
            .map(|s| s.to_string());

        let thumbnail_url = data["albumOfTrack"]["coverArt"]["sources"]
            .as_array()
            .and_then(|imgs| imgs.first())
            .and_then(|img| img["url"].as_str())
            .map(|s| s.to_string());

        Some(SourceTrack {
            id,
            title,
            artists,
            album,
            duration_ms,
            isrc,
            is_explicit,
            is_playable,
            preview_url,
            thumbnail_url,
        })
    }

    pub fn parse_spotify_track(track_obj: &Value) -> Option<SourceTrack> {
        let id = track_obj["id"].as_str()?.to_string();
        let title = track_obj["name"].as_str().unwrap_or("Unknown").to_string();

        let artists = track_obj["artists"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let album = track_obj["album"]["name"].as_str().map(|s| s.to_string());
        let duration_ms = track_obj["duration_ms"].as_u64().unwrap_or(0);
        let isrc = track_obj["external_ids"]["isrc"]
            .as_str()
            .map(|s| s.to_string());
        let is_explicit = track_obj["explicit"].as_bool().unwrap_or(false);
        let is_playable = track_obj["is_playable"].as_bool().unwrap_or(true);
        let preview_url = track_obj["preview_url"].as_str().map(|s| s.to_string());

        let thumbnail_url = track_obj["album"]["images"]
            .as_array()
            .and_then(|imgs| imgs.first())
            .and_then(|img| img["url"].as_str())
            .map(|s| s.to_string());

        Some(SourceTrack {
            id,
            title,
            artists,
            album,
            duration_ms,
            isrc,
            is_explicit,
            is_playable,
            preview_url,
            thumbnail_url,
        })
    }

    async fn check_playlist_editable_via_pathfinder(
        &self,
        playlist_id: &str,
    ) -> Result<Option<()>, ProviderError> {
        let uri = if playlist_id.starts_with("spotify:playlist:") {
            playlist_id.to_string()
        } else {
            format!("spotify:playlist:{playlist_id}")
        };
        let variables = serde_json::json!({
            "uri": uri,
            "offset": 0,
            "limit": 1,
            "enableWatchFeedEntrypoint": false
        });
        let json = self
            .pathfinder_typed(
                "fetchPlaylist",
                "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77",
                variables,
            )
            .await?;
        match json
            .pointer("/data/playlistV2/currentUserCapabilities/canEditItems")
            .and_then(|v| v.as_bool())
        {
            Some(true) => Ok(Some(())),
            Some(false) => Err(ProviderError::new(
                ProviderErrorKind::Permission,
                SendCertainty::NotSent,
                "Spotify playlist is read-only or not owned by the connected account",
            )),
            None => Ok(None),
        }
    }

    async fn current_user_id_rest(&self) -> Result<String, ProviderError> {
        if let Some(cached) = self.me_user_id.read().await.clone() {
            return Ok(cached);
        }
        if let Some(cached) = self.session_cache.me_user_id_for(&self.sp_dc).await {
            *self.me_user_id.write().await = Some(cached.clone());
            return Ok(cached);
        }
        let base = "https://api.spotify.com";
        #[cfg(test)]
        let base = self.mutation_url.as_deref().unwrap_or(base);
        let me_json = self
            .send_authed_get_json(&format!("{base}/v1/me"), "verifying account ownership")
            .await
            .map_err(|err| {
                if err.kind == ProviderErrorKind::MalformedResponse {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Playlist ownership response is malformed",
                    )
                } else {
                    err
                }
            })?;
        let id = me_json["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::Permission,
                    SendCertainty::NotSent,
                    "Spotify playlist is read-only or not owned by the connected account",
                )
            })?;
        *self.me_user_id.write().await = Some(id.clone());
        self.session_cache
            .set_me_user_id(&self.sp_dc, id.clone())
            .await;
        Ok(id)
    }

    async fn check_playlist_editable_rest(&self, playlist_id: &str) -> Result<(), ProviderError> {
        let base = "https://api.spotify.com";
        #[cfg(test)]
        let base = self.mutation_url.as_deref().unwrap_or(base);
        let current_user_id = self.current_user_id_rest().await?;

        let playlist_json = self
            .send_authed_get_json(
                &format!("{base}/v1/playlists/{playlist_id}?fields=owner(id)"),
                "checking playlist editability",
            )
            .await
            .map_err(|err| {
                if err.kind == ProviderErrorKind::MalformedResponse {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Playlist ownership response is malformed",
                    )
                } else {
                    err
                }
            })?;
        if playlist_json["owner"]["id"].as_str() != Some(current_user_id.as_str()) {
            return Err(ProviderError::new(
                ProviderErrorKind::Permission,
                SendCertainty::NotSent,
                "Spotify playlist is read-only or not owned by the connected account",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    async fn check_playlist_editable(&self, playlist_id: &str) -> Result<(), ProviderError> {
        if !playlist_id.chars().all(|c| c.is_ascii_alphanumeric()) || playlist_id.is_empty() {
            return Err(ProviderError::new(
                ProviderErrorKind::Permission,
                SendCertainty::NotSent,
                "Choose an owned editable Spotify playlist",
            ));
        }
        match self
            .check_playlist_editable_via_pathfinder(playlist_id)
            .await
        {
            Ok(Some(())) => Ok(()),
            Ok(None) => self.check_playlist_editable_rest(playlist_id).await,
            Err(err)
                if matches!(
                    err.kind,
                    ProviderErrorKind::Authentication | ProviderErrorKind::Permission
                ) =>
            {
                Err(err)
            }
            Err(pathfinder_err) => match self.check_playlist_editable_rest(playlist_id).await {
                Ok(()) => Ok(()),
                Err(rest_err) => Err(
                    if pathfinder_err.kind == ProviderErrorKind::RateLimited
                        && rest_err.kind != ProviderErrorKind::RateLimited
                    {
                        pathfinder_err
                    } else {
                        rest_err
                    },
                ),
            },
        }
    }
    fn sync_capabilities(&self) -> crate::providers::contracts::SyncCapabilities {
        crate::providers::contracts::SyncCapabilities {
            duplicate_insertion: true,
            exact_removal: false,
            occurrence_movement: true,
        }
    }
    async fn sync_remove_occurrence(
        &self,
        playlist_id: &str,
        occurrence: &PlaylistOccurrence,
        baseline: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        if !baseline.complete
            || baseline.revision.as_deref().is_none_or(str::is_empty)
            || baseline
                .occurrences
                .get(occurrence.index)
                .is_none_or(|o| o.track.id != occurrence.track.id)
            || baseline
                .occurrences
                .iter()
                .filter(|o| o.track.id == occurrence.track.id)
                .count()
                != 1
        {
            return Err(ProviderError::new(
                ProviderErrorKind::Conflict,
                SendCertainty::NotSent,
                "Spotify duplicate or unversioned removal is unsupported; manual review required",
            ));
        }
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let headers = self.rest_headers(&token)?;
        let uri = if occurrence.track.id.starts_with("spotify:track:") {
            occurrence.track.id.clone()
        } else {
            format!("spotify:track:{}", occurrence.track.id)
        };
        let response = self
            .client
            .delete(self.playlist_items_url(playlist_id))
            .headers(headers)
            .json(&serde_json::json!({"items":[{"uri":uri}],"snapshot_id":baseline.revision}))
            .send()
            .await
            .map_err(ProviderError::transport)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access_token().await;
        }
        if !response.status().is_success() {
            return Err(ProviderError::http(
                response.status().as_u16(),
                response.headers(),
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Invalid Spotify removal response; outcome uncertain",
            )
        })?;
        let revision = value["snapshot_id"]
            .as_str()
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        Ok(MutationOutcome {
            occurrences: vec![OccurrenceOutcome {
                index: 0,
                track_id: occurrence.track.id.clone(),
                remote_id: None,
                status: if revision.is_some() {
                    OccurrenceStatus::Accepted
                } else {
                    OccurrenceStatus::Uncertain
                },
            }],
            revision,
        })
    }
    async fn move_occurrence(
        &self,
        playlist_id: &str,
        occurrence: &PlaylistOccurrence,
        before: Option<&PlaylistOccurrence>,
        baseline: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        let valid = |o: &PlaylistOccurrence| {
            baseline
                .occurrences
                .get(o.index)
                .is_some_and(|b| b.track.id == o.track.id)
        };
        let revision = baseline
            .revision
            .as_deref()
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::Conflict,
                    SendCertainty::NotSent,
                    "Spotify revision missing; no move sent",
                )
            })?;
        if !baseline.complete || !valid(occurrence) || before.is_some_and(|o| !valid(o)) {
            return Err(ProviderError::new(
                ProviderErrorKind::Conflict,
                SendCertainty::NotSent,
                "Occurrence differs from complete baseline",
            ));
        }
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let headers = self.rest_headers(&token)?;
        let response = self.client.put(self.playlist_items_url(playlist_id)).headers(headers)
            .json(&serde_json::json!({"range_start":occurrence.index,"range_length":1,"insert_before":before.map_or(baseline.occurrences.len(),|o|o.index),"snapshot_id":revision}))
            .send().await.map_err(ProviderError::transport)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access_token().await;
        }
        if !response.status().is_success() {
            return Err(ProviderError::http(
                response.status().as_u16(),
                response.headers(),
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Invalid Spotify movement response; outcome uncertain",
            )
        })?;
        let revision = value["snapshot_id"]
            .as_str()
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        Ok(MutationOutcome {
            occurrences: vec![OccurrenceOutcome {
                index: 0,
                track_id: occurrence.track.id.clone(),
                remote_id: None,
                status: if revision.is_some() {
                    OccurrenceStatus::Accepted
                } else {
                    OccurrenceStatus::Uncertain
                },
            }],
            revision,
        })
    }
    fn supports_isrc_search(&self) -> bool {
        true
    }

    async fn playlist_revision(&self, playlist_id: &str) -> Result<Option<String>, ProviderError> {
        if playlist_id.contains("collection") || playlist_id == "me:liked" {
            return Ok(None);
        }
        let url = format!(
            "https://api.spotify.com/v1/playlists/{}?fields=snapshot_id",
            playlist_id.trim_start_matches("spotify:playlist:")
        );
        #[cfg(test)]
        let url = self.mutation_url.clone().unwrap_or(url);
        let value = self
            .send_authed_get_json(&url, "reading playlist revision")
            .await
            .map_err(|err| {
                if err.kind == ProviderErrorKind::MalformedResponse {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Invalid playlist revision response; no mutation sent",
                    )
                } else {
                    err
                }
            })?;
        value["snapshot_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| Some(id.to_string()))
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::MalformedResponse,
                    SendCertainty::NotSent,
                    "Playlist revision missing; no mutation sent",
                )
            })
    }

    async fn canary(&self) -> Result<(), String> {
        self.check_session().await.map_err(|e| e.to_string())
    }
    async fn check_session(&self) -> Result<(), ProviderError> {
        let value = self
            .pathfinder_typed(
                "fetchLibraryTracks",
                "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240",
                serde_json::json!({"offset":0,"limit":1}),
            )
            .await?;
        if value
            .pointer("/data/me/library/tracks/items")
            .and_then(|v| v.as_array())
            .is_none()
        {
            return Err(ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Session canary schema mismatch; reconnect or review provider availability",
            ));
        }
        Ok(())
    }

    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        let mut playlists = Vec::new();
        let mut seen_playlist_ids = std::collections::HashSet::new();
        let mut folder_queue: std::collections::VecDeque<Option<String>> =
            std::collections::VecDeque::new();
        folder_queue.push_back(None); // None represents root library
        let mut visited_folders = std::collections::HashSet::new();

        while let Some(current_folder) = folder_queue.pop_front() {
            if let Some(ref f_uri) = current_folder {
                if !visited_folders.insert(f_uri.clone()) {
                    continue;
                }
            }

            let mut offset = 0;
            let limit = 50;

            loop {
                let mut variables = serde_json::json!({
                    "filters": if current_folder.is_none() { serde_json::json!(["Playlists"]) } else { serde_json::json!([]) },
                    "order": serde_json::Value::Null,
                    "textFilter": "",
                    "features": ["LIKED_SONGS", "YOUR_EPISODES"],
                    "limit": limit,
                    "offset": offset
                });

                if let Some(ref f_uri) = current_folder {
                    variables["folderUri"] = serde_json::Value::String(f_uri.clone());
                }

                let json = self
                    .pathfinder_query(
                        "libraryV3",
                        "973e511ca44261fda7eebac8b653155e7caee3675abb4fb110cc1b8c78b091c3",
                        variables,
                    )
                    .await?;

                let library = &json["data"]["me"]["libraryV3"];
                let items = match library["items"].as_array() {
                    Some(arr) => arr,
                    None => break,
                };

                if items.is_empty() {
                    break;
                }

                for item in items {
                    let item_wrapper = &item["item"];
                    let typename = item_wrapper["__typename"].as_str().unwrap_or("");

                    if typename.contains("Folder") {
                        let folder_uri = item_wrapper["_uri"]
                            .as_str()
                            .or_else(|| item_wrapper["data"]["uri"].as_str());
                        if let Some(uri) = folder_uri {
                            if !uri.is_empty() && !visited_folders.contains(uri) {
                                folder_queue.push_back(Some(uri.to_string()));
                            }
                        }
                        continue;
                    }

                    if typename.contains("PseudoPlaylist") {
                        if current_folder.is_none() {
                            let data = &item_wrapper["data"];
                            let name = data["name"].as_str().unwrap_or("Liked Songs").to_string();
                            if name.to_lowercase().contains("episodes") {
                                continue;
                            }
                            let count = data["count"].as_u64().unwrap_or(0) as u32;
                            let cover_url = data["images"]["items"]
                                .as_array()
                                .and_then(|arr| arr.first())
                                .and_then(|img| img["sources"].as_array())
                                .and_then(|srcs| srcs.first())
                                .and_then(|src| src["url"].as_str())
                                .map(|s| s.to_string());

                            if seen_playlist_ids.insert("collection:tracks".to_string()) {
                                playlists.push(Playlist {
                                    id: "collection:tracks".to_string(),
                                    service: "spotify".to_string(),
                                    title: name,
                                    description: Some("Your Spotify Liked Songs".to_string()),
                                    track_count: count,
                                    is_public: false,
                                    cover_url,
                                });
                            }
                        }
                        continue;
                    }

                    if typename.contains("Playlist") {
                        let data = &item_wrapper["data"];
                        let uri = data["uri"]
                            .as_str()
                            .or_else(|| item_wrapper["_uri"].as_str())
                            .unwrap_or("");
                        let id = uri
                            .strip_prefix("spotify:playlist:")
                            .unwrap_or(uri)
                            .to_string();

                        if id.is_empty() || !seen_playlist_ids.insert(id.clone()) {
                            continue;
                        }

                        let title = data["name"].as_str().unwrap_or("Untitled").to_string();
                        let description = data["description"].as_str().map(|s| s.to_string());
                        let can_edit = data["currentUserCapabilities"]["canEditItems"]
                            .as_bool()
                            .unwrap_or(true);
                        let is_public = !can_edit;
                        let initial_count = data["count"]
                            .as_u64()
                            .or_else(|| data["trackCount"].as_u64())
                            .unwrap_or(0) as u32;

                        let cover_url = data["images"]["items"]
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|img| img["sources"].as_array())
                            .and_then(|srcs| srcs.first())
                            .and_then(|src| src["url"].as_str())
                            .map(|s| s.to_string());

                        playlists.push(Playlist {
                            id,
                            service: "spotify".to_string(),
                            title,
                            description,
                            track_count: initial_count,
                            is_public,
                            cover_url,
                        });
                    }
                }

                let total = library["totalCount"].as_u64().unwrap_or(0) as usize;
                offset += items.len();
                if offset >= total || items.len() < limit {
                    break;
                }
            }
        }

        // Concurrently resolve exact track counts for playlists with moderate backoff/concurrency
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        let mut join_set = tokio::task::JoinSet::new();

        for (idx, pl) in playlists.iter().enumerate() {
            if pl.id == "collection:tracks" || pl.track_count > 0 {
                continue;
            }
            let sem_clone = sem.clone();
            let provider = self.clone();
            let pl_id = pl.id.clone();
            join_set.spawn(async move {
                let _permit = sem_clone.acquire().await.ok();
                tokio::time::sleep(std::time::Duration::from_millis((idx % 3) as u64 * 45)).await;
                let count = provider.fetch_playlist_track_count(&pl_id).await;
                (idx, count)
            });
        }

        while let Some(res) = join_set.join_next().await {
            if let Ok((idx, Some(count))) = res {
                if idx < playlists.len() {
                    playlists[idx].track_count = count;
                }
            }
        }

        Ok(playlists)
    }

    async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        self.fetch_tracks(playlist_id, tx)
            .await
            .map_err(|e| e.to_string())
    }
    async fn read_playlist(&self, playlist_id: &str) -> Result<PlaylistRead, ProviderError> {
        let revision_before = self.playlist_revision(playlist_id).await?;
        let before = self.fetch_tracks(playlist_id, None).await?;
        let after = self.fetch_tracks(playlist_id, None).await?;
        let revision = self.playlist_revision(playlist_id).await?;
        if revision_before != revision
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
                "Unresolved playlist occurrences; complete baseline unavailable",
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

    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String> {
        self.search_track_resilient(query)
            .await
            .map_err(|e| e.to_string())
    }

    async fn search_track_resilient(&self, query: &str) -> Result<Vec<SourceTrack>, ProviderError> {
        let variables = serde_json::json!({
            "searchTerm": query,
            "offset": 0,
            "limit": 5,
            "numberOfTopResults": 5,
            "includeAudiobooks": false,
            "includeArtistHasConcertsField": false,
            "includePreReleases": false,
            "includeLocalConcertsField": false
        });

        match self
            .pathfinder_typed(
                "searchDesktop",
                "d9f785900f0710b31c07818d617f4f7600c1e21217e80f5b043d1e78d74e6026",
                variables,
            )
            .await
        {
            Ok(json) => {
                let mut results = Vec::new();
                if let Some(items) = json["data"]["searchV2"]["tracksV2"]["items"].as_array() {
                    for item in items {
                        let track_data = &item["item"]["data"];
                        let fallback_uri = track_data["uri"].as_str();
                        if let Some(track) = Self::parse_pathfinder_track(track_data, fallback_uri)
                        {
                            results.push(track);
                        }
                    }
                }
                Ok(results)
            }
            Err(err)
                if matches!(
                    err.kind,
                    ProviderErrorKind::Authentication | ProviderErrorKind::RateLimited
                ) =>
            {
                Err(err)
            }
            Err(_) => {
                let token = self
                    .get_access_token()
                    .await
                    .map_err(ProviderError::before_send)?;
                let headers = self.rest_headers(&token)?;
                let base = "https://api.spotify.com";
                #[cfg(test)]
                let base = self.mutation_url.as_deref().unwrap_or(base);
                let res = self
                    .client
                    .get(format!("{base}/v1/search"))
                    .query(&[("q", query), ("type", "track"), ("limit", "5")])
                    .headers(headers)
                    .send()
                    .await
                    .map_err(ProviderError::read_transport)?;
                if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                    self.invalidate_access_token().await;
                }
                if !res.status().is_success() {
                    return Err(ProviderError::read_http_for(
                        "Spotify",
                        "searching tracks",
                        res.status().as_u16(),
                        res.headers(),
                    ));
                }
                let json: Value = res.json().await.map_err(|_| {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Invalid Spotify search response schema",
                    )
                })?;
                let mut results = Vec::new();
                if let Some(items) = json["tracks"]["items"].as_array() {
                    for item in items {
                        if let Some(track) = Self::parse_spotify_track(item) {
                            results.push(track);
                        }
                    }
                }
                Ok(results)
            }
        }
    }

    async fn fetch_track_by_id(
        &self,
        track_id: &str,
    ) -> Result<Option<SourceTrack>, ProviderError> {
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let headers = self.rest_headers(&token)?;
        let base = "https://api.spotify.com";
        #[cfg(test)]
        let base = self.mutation_url.as_deref().unwrap_or(base);
        let res = self
            .client
            .get(format!("{base}/v1/tracks/{track_id}"))
            .headers(headers)
            .send()
            .await
            .map_err(ProviderError::read_transport)?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access_token().await;
        }
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(ProviderError::read_http_for(
                "Spotify",
                "loading track metadata",
                res.status().as_u16(),
                res.headers(),
            ));
        }
        let json: Value = match res.json().await {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        Ok(Self::parse_spotify_track(&json))
    }

    async fn create_playlist_checked(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, ProviderError> {
        let user_id = self.current_user_id_rest().await?;
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let headers = self.rest_headers(&token)?;

        let create_url = format!("https://api.spotify.com/v1/users/{}/playlists", user_id);
        let body = serde_json::json!({
            "name": title,
            "description": description.unwrap_or(""),
            "public": false
        });

        let res = self
            .client
            .post(&create_url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(ProviderError::transport)?;

        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access_token().await;
        }
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let headers = res.headers().clone();
            let body = res.text().await.ok();
            return Err(ProviderError::http_with_detail(
                status,
                &headers,
                body.as_deref(),
            ));
        }

        let playlist_json: Value = res.json().await.map_err(|e| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                &format!("Failed to parse created playlist response: {}", e),
            )
        })?;

        let id = playlist_json["id"].as_str().ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Failed to retrieve created Spotify playlist ID",
            )
        })?;

        Ok(id.to_string())
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String> {
        self.create_playlist_checked(title, description)
            .await
            .map_err(|e| e.to_string())
    }

    async fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let outcome = self
            .add_occurrences(playlist_id, track_ids)
            .await
            .map_err(|e| e.to_string())?;
        if !outcome.all_accepted() {
            return Err(
                "Spotify mutation lacks snapshot evidence; remote outcome uncertain".into(),
            );
        }
        Ok(())
    }

    async fn add_occurrences(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<MutationOutcome, ProviderError> {
        if track_ids.is_empty() || track_ids.len() > 100 {
            return Err(ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Invalid mutation batch size; no send",
            ));
        }
        let token = self
            .get_access_token()
            .await
            .map_err(ProviderError::before_send)?;
        let headers = self.rest_headers(&token)?;
        let url = self.playlist_items_url(playlist_id);
        let uris: Vec<String> = track_ids
            .iter()
            .map(|id| {
                if id.starts_with("spotify:track:") {
                    id.clone()
                } else {
                    format!("spotify:track:{}", id)
                }
            })
            .collect();
        let res = self
            .client
            .post(&url)
            .headers(headers)
            .json(&serde_json::json!({"uris":uris}))
            .send()
            .await
            .map_err(ProviderError::transport)?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_access_token().await;
        }
        if !res.status().is_success() {
            return Err(ProviderError::http(res.status().as_u16(), res.headers()));
        }
        let json: Value = res.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Invalid Spotify mutation response; remote outcome uncertain",
            )
        })?;
        let revision = json["snapshot_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Ok(MutationOutcome {
            occurrences: track_ids
                .iter()
                .enumerate()
                .map(|(index, id)| OccurrenceOutcome {
                    index,
                    track_id: id.clone(),
                    status: if revision.is_some() {
                        OccurrenceStatus::Accepted
                    } else {
                        OccurrenceStatus::Uncertain
                    },
                    remote_id: None,
                })
                .collect(),
            revision,
        })
    }

    async fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let token = self.get_access_token().await?;
        let headers = self.rest_headers(&token).map_err(|e| e.to_string())?;
        let url = self.playlist_items_url(playlist_id);

        for chunk in track_ids.chunks(100) {
            let items: Vec<Value> = chunk
                .iter()
                .map(|id| {
                    let uri = if id.starts_with("spotify:track:") {
                        id.clone()
                    } else {
                        format!("spotify:track:{}", id)
                    };
                    serde_json::json!({ "uri": uri })
                })
                .collect();

            let body = serde_json::json!({ "items": items });

            let res = self
                .client
                .delete(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Spotify remove_tracks failed: {}", e))?;

            if !res.status().is_success() {
                let status = res.status();

                return Err(format!("Spotify remove_tracks failed ({})", status));
            }
        }

        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
        let token = self.get_access_token().await?;
        let headers = self.rest_headers(&token).map_err(|e| e.to_string())?;
        let url = format!(
            "https://api.spotify.com/v1/playlists/{}/followers",
            playlist_id
        );

        let res = self
            .client
            .delete(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("Spotify delete_playlist failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();

            return Err(format!("Spotify delete_playlist failed ({})", status));
        }

        Ok(())
    }
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let mut hasher = Sha1::new();
        hasher.update(key);
        let hashed = hasher.finalize();
        k_block[..20].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut k_ipad = [0u8; 64];
    let mut k_opad = [0u8; 64];
    for i in 0..64 {
        k_ipad[i] = k_block[i] ^ 0x36;
        k_opad[i] = k_block[i] ^ 0x5c;
    }

    let mut inner = Sha1::new();
    inner.update(k_ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha1::new();
    outer.update(k_opad);
    outer.update(inner_hash);
    let outer_hash = outer.finalize();

    let mut result = [0u8; 20];
    result.copy_from_slice(&outer_hash);
    result
}

fn derive_spotify_totp_key() -> Vec<u8> {
    const RAW_SECRET: &str = ",7/*F(\"rLJ2oxaKL^f+E1xvP@N";
    let mut num_str = String::new();
    for (idx, ch) in RAW_SECRET.chars().enumerate() {
        let val = (ch as u32) ^ ((idx as u32 % 33) + 9);
        num_str.push_str(&val.to_string());
    }
    num_str.into_bytes()
}

pub fn generate_spotify_totp(timestamp_ms: u64) -> String {
    let key = derive_spotify_totp_key();
    let counter = timestamp_ms / 1000 / 30;
    let data = counter.to_be_bytes();
    let digest = hmac_sha1(&key, &data);

    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);

    let otp = binary % 1_000_000;
    format!("{:06}", otp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sync_editability_requires_current_user_ownership() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for body in [r#"{}"#, r#"{"id":"owner"}"#, r#"{"owner":{"id":"owner"}}"#] {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0; 4096];
                socket.read(&mut bytes).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        provider.check_playlist_editable("fixture").await.unwrap();
        thread.join().unwrap();
    }
    #[tokio::test]
    async fn check_playlist_editable_retries_short_429_and_caches_user_id() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut paths = Vec::new();
            for _ in 0..6 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0; 4096];
                let size = socket.read(&mut bytes).unwrap();
                let req = String::from_utf8_lossy(&bytes[..size]).to_string();
                let line = req.lines().next().unwrap_or("").to_string();
                paths.push(line.clone());
                if line.starts_with("POST ") {
                    let body = r#"{}"#;
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                } else if line.contains("/v1/me")
                    && paths.iter().filter(|p| p.contains("/v1/me")).count() == 1
                {
                    write!(
                        socket,
                        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else if line.contains("/v1/me") {
                    let body = r#"{"id":"owner"}"#;
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                } else {
                    let body = r#"{"owner":{"id":"owner"}}"#;
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                }
            }
            paths
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        provider.check_playlist_editable("pl1").await.unwrap();
        provider.check_playlist_editable("pl2").await.unwrap();
        let paths = thread.join().unwrap();
        assert_eq!(paths.len(), 6);
        assert!(paths[5].contains("/v1/playlists/pl2"));
    }
    #[tokio::test]
    async fn sync_unique_removal_uses_items_and_snapshot() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut bytes = [0; 8192];
            let size = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).to_string();
            let body = r#"{"snapshot_id":"after"}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            request
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let occurrence = PlaylistOccurrence {
            index: 0,
            track: SourceTrack {
                id: "a".into(),
                ..SourceTrack::unavailable(0)
            },
            remote_id: None,
        };
        let baseline = PlaylistRead {
            occurrences: vec![occurrence.clone()],
            revision: Some("before".into()),
            complete: true,
        };
        assert!(provider
            .sync_remove_occurrence("fixture", &occurrence, &baseline)
            .await
            .unwrap()
            .all_accepted());
        let request = thread.join().unwrap();
        assert!(request.starts_with("DELETE /v1/playlists/fixture/items "));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"items":[{"uri":"spotify:track:a"}],"snapshot_id":"before"})
        );
    }
    #[tokio::test]
    async fn sync_spotify_move_targets_versioned_position() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut bytes = [0; 8192];
            let size = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).to_string();
            let body = r#"{"snapshot_id":"after"}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            request
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let a = PlaylistOccurrence {
            index: 0,
            track: SourceTrack {
                id: "a".into(),
                ..SourceTrack::unavailable(0)
            },
            remote_id: None,
        };
        let b = PlaylistOccurrence {
            index: 1,
            track: SourceTrack {
                id: "b".into(),
                ..SourceTrack::unavailable(1)
            },
            remote_id: None,
        };
        let baseline = PlaylistRead {
            occurrences: vec![a.clone(), b.clone()],
            revision: Some("before".into()),
            complete: true,
        };
        let outcome = provider
            .move_occurrence("fixture", &b, Some(&a), &baseline)
            .await
            .unwrap();
        assert_eq!(outcome.revision.as_deref(), Some("after"));
        let request = thread.join().unwrap();
        assert!(request.starts_with("PUT /v1/playlists/fixture/items "));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"range_start":1,"range_length":1,"insert_before":0,"snapshot_id":"before"})
        );
    }
    #[tokio::test]
    async fn playlist_add_uses_current_items_endpoint() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            let size = socket.read(&mut request).unwrap();
            let first = String::from_utf8_lossy(&request[..size])
                .lines()
                .next()
                .unwrap()
                .to_string();
            let body = r#"{"snapshot_id":"after"}"#;
            write!(socket,"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            first
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        assert!(provider
            .add_occurrences("fixture", &["a".into()])
            .await
            .unwrap()
            .all_accepted());
        assert_eq!(
            thread.join().unwrap(),
            "POST /v1/playlists/fixture/items HTTP/1.1"
        );
    }
    #[tokio::test]
    async fn spotify_structured_library_read_preserves_retry_headers() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let error = provider
            .read_playlist("collection:tracks")
            .await
            .unwrap_err();
        thread.join().unwrap();
        assert_eq!(error.kind, ProviderErrorKind::RateLimited);
        assert_eq!(error.retry_after_ms, Some(60_000));
        assert_eq!(error.certainty, SendCertainty::NotSent);
    }

    #[tokio::test]
    async fn structured_playlist_reads_retain_spotify_revision() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        server.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                let Ok((mut socket, _)) = server.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                let mut request = [0u8; 8192];
                socket.read(&mut request).unwrap();
                let body = r#"{"snapshot_id":"revision-1"}"#;
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                return;
            }
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let revision = provider.playlist_revision("fixture").await.unwrap();
        thread.join().unwrap();
        assert_eq!(revision.as_deref(), Some("revision-1"));
    }

    #[tokio::test]
    async fn spotify_mutation_without_snapshot_is_uncertain() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let result = provider
            .add_tracks_to_playlist("fixture", &["a".into()])
            .await;
        thread.join().unwrap();
        assert!(
            result.is_err(),
            "successful HTTP alone is insufficient mutation evidence"
        );
    }

    #[tokio::test]
    async fn spotify_missing_playlist_page_is_not_empty_success() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        let result = provider.get_playlist_tracks("fixture", None).await;
        thread.join().unwrap();
        assert!(
            result.is_err(),
            "missing schema cannot become an empty baseline"
        );
    }

    #[tokio::test]
    async fn unauthorized_mutation_invalidates_cached_access_token() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        assert!(provider
            .add_tracks_to_playlist("fixture", &["a".into()])
            .await
            .is_err());
        thread.join().unwrap();
        assert!(provider.access_token.read().await.is_none());
    }

    #[test]
    fn test_spotify_totp_vector() {
        let totp = generate_spotify_totp(1791023811000);
        assert_eq!(totp, "237400");
    }

    #[test]
    fn test_parse_pathfinder_track_fetch_playlist() {
        let json = serde_json::json!({
            "__typename": "Track",
            "uri": "spotify:track:2ckIzhVR3hqnf7sJ2uvypz",
            "name": "No Batidao - Phonk (Marimba Ringtone Cover)",
            "trackDuration": {
                "totalMilliseconds": 31384
            },
            "albumOfTrack": {
                "name": "No Batidao",
                "coverArt": {
                    "sources": [
                        { "url": "https://image-cdn.spotifycdn.com/cover.jpg" }
                    ]
                }
            },
            "artists": {
                "items": [
                    { "profile": { "name": "Anime Ringtones" } },
                    { "profile": { "name": "Anytunz" } }
                ]
            },
            "contentRating": {
                "label": "NONE"
            },
            "playability": {
                "playable": true
            }
        });

        let track = SpotifyProvider::parse_pathfinder_track(&json, None).expect("Should parse");
        assert_eq!(track.id, "2ckIzhVR3hqnf7sJ2uvypz");
        assert_eq!(track.title, "No Batidao - Phonk (Marimba Ringtone Cover)");
        assert_eq!(track.artists, vec!["Anime Ringtones", "Anytunz"]);
        assert_eq!(track.album, Some("No Batidao".to_string()));
        assert_eq!(track.duration_ms, 31384);
        assert!(!track.is_explicit);
        assert!(track.is_playable);
        assert_eq!(
            track.thumbnail_url,
            Some("https://image-cdn.spotifycdn.com/cover.jpg".to_string())
        );
    }

    #[test]
    fn test_parse_pathfinder_track_fetch_library() {
        let json = serde_json::json!({
            "__typename": "Track",
            "name": "A Song of Ice and Fire",
            "duration": {
                "totalMilliseconds": 131888
            },
            "albumOfTrack": {
                "name": "Game Of Thrones: Season 8",
                "coverArt": {
                    "sources": [
                        { "url": "https://image-cdn.spotifycdn.com/got.jpg" }
                    ]
                }
            },
            "artists": {
                "items": [
                    { "profile": { "name": "Ramin Djawadi" } }
                ]
            },
            "contentRating": {
                "label": "EXPLICIT"
            },
            "playability": {
                "playable": true
            }
        });

        let track = SpotifyProvider::parse_pathfinder_track(
            &json,
            Some("spotify:track:1AvLUHxSunGMWWRfEFmWSC"),
        )
        .expect("Should parse with fallback URI");
        assert_eq!(track.id, "1AvLUHxSunGMWWRfEFmWSC");
        assert_eq!(track.title, "A Song of Ice and Fire");
        assert_eq!(track.artists, vec!["Ramin Djawadi"]);
        assert_eq!(track.album, Some("Game Of Thrones: Season 8".to_string()));
        assert_eq!(track.duration_ms, 131888);
        assert!(track.is_explicit);
        assert!(track.is_playable);
    }

    #[test]
    fn test_parse_pathfinder_track_search_desktop() {
        let json = serde_json::json!({
            "__typename": "Track",
            "id": "37JOhPdeecNxkqpfcj1XcX",
            "uri": "spotify:track:37JOhPdeecNxkqpfcj1XcX",
            "name": "Darmiyaan (From \"Musafir Cafe\")",
            "duration": {
                "totalMilliseconds": 270125
            },
            "albumOfTrack": {
                "name": "Musafir Cafe",
                "coverArt": {
                    "sources": [
                        { "url": "https://image-cdn.spotifycdn.com/musafir.jpg" }
                    ]
                }
            },
            "artists": {
                "items": [
                    { "profile": { "name": "Rekha Bhardwaj" } },
                    { "profile": { "name": "Raghav Kaushik" } }
                ]
            },
            "contentRating": {
                "label": "NONE"
            },
            "playability": {
                "playable": true
            }
        });

        let track = SpotifyProvider::parse_pathfinder_track(&json, None).expect("Should parse");
        assert_eq!(track.id, "37JOhPdeecNxkqpfcj1XcX");
        assert_eq!(track.title, "Darmiyaan (From \"Musafir Cafe\")");
        assert_eq!(track.artists, vec!["Rekha Bhardwaj", "Raghav Kaushik"]);
        assert_eq!(track.duration_ms, 270125);
    }

    #[tokio::test]
    async fn check_playlist_editable_prefers_pathfinder_without_rest_calls() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut methods = Vec::new();
            for _ in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                let size = socket.read(&mut bytes).unwrap();
                let req = String::from_utf8_lossy(&bytes[..size]).to_string();
                let line = req.lines().next().unwrap_or("").to_string();
                methods.push(line.clone());
                if line.starts_with("POST ") {
                    let can_edit = !req.contains("readonlyplaylist");
                    let body = serde_json::json!({
                        "data": {
                            "playlistV2": {
                                "currentUserCapabilities": {
                                    "canEditItems": can_edit
                                }
                            }
                        }
                    })
                    .to_string();
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                } else {
                    write!(
                        socket,
                        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 59\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
            }
            methods
        });
        let mut provider = SpotifyProvider::new("fixture".into());
        provider.mutation_url = Some(format!("http://{address}"));
        *provider.access_token.write().await = Some("cached".into());
        provider
            .check_playlist_editable("editableplaylist")
            .await
            .unwrap();
        let readonly_err = provider
            .check_playlist_editable("readonlyplaylist")
            .await
            .unwrap_err();
        assert_eq!(readonly_err.kind, ProviderErrorKind::Permission);
        let methods = thread.join().unwrap();
        assert_eq!(methods.len(), 2);
        assert!(methods.iter().all(|m| m.starts_with("POST ")));
    }

    #[tokio::test]
    async fn shared_session_cache_reuses_token_and_me_user_id_across_provider_instances() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let cache = SpotifySessionCache::default();
        cache
            .set_access_token(
                "sp_dc_user".into(),
                "shared_access_token".into(),
                now_ms + 300_000,
            )
            .await;
        cache
            .set_me_user_id("sp_dc_user", "cached_owner".into())
            .await;

        let first = SpotifyProvider::with_session_cache("sp_dc_user".into(), cache.clone());
        assert_eq!(
            first.get_access_token().await.unwrap(),
            "shared_access_token"
        );
        assert_eq!(first.current_user_id_rest().await.unwrap(), "cached_owner");

        let second = SpotifyProvider::with_session_cache("sp_dc_user".into(), cache.clone());
        assert_eq!(
            second.get_access_token().await.unwrap(),
            "shared_access_token"
        );
        assert_eq!(second.current_user_id_rest().await.unwrap(), "cached_owner");

        second.invalidate_access_token().await;
        assert!(cache.access_token_for("sp_dc_user", now_ms).await.is_none());
        assert!(cache.me_user_id_for("sp_dc_user").await.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_spotify_diag() {
        let app_data = std::env::var("APPDATA").unwrap_or_default();
        let db_path = format!("{}\\com.soundshift.app\\soundshift.db", app_data);
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            if let Ok(Some(sp_dc)) =
                crate::auth::keyring_store::retrieve_credential("spotify", &conn)
            {
                let provider = SpotifyProvider::new(sp_dc);
                crate::diagnostics::write_error(&format!("Calling provider.list_playlists()..."));
                let start = std::time::Instant::now();
                match provider.list_playlists().await {
                    Ok(playlists) => {
                        crate::diagnostics::write_error(&format!(
                            "LIST_PLAYLISTS SUCCESS! Found {} playlists in {:?}",
                            playlists.len(),
                            start.elapsed()
                        ));
                        for (i, p) in playlists.iter().enumerate() {
                            crate::diagnostics::write_error(&format!(
                                "  #{:02}: title='{}', tracks={}, public={}, id={}",
                                i + 1,
                                p.title,
                                p.track_count,
                                p.is_public,
                                p.id
                            ));
                        }
                    }
                    Err(e) => {
                        crate::diagnostics::write_error(&format!("LIST_PLAYLISTS FAILED: {}", e))
                    }
                }
            }
        }
    }
}
