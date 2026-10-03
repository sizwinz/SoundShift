use crate::models::{Playlist, SourceTrack};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, COOKIE, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SpotifyProvider {
    client: reqwest::Client,
    sp_dc: String,
    access_token: Arc<RwLock<Option<String>>>,
}

impl SpotifyProvider {
    pub fn new(sp_dc: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            sp_dc,
            access_token: Arc::new(RwLock::new(None)),
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

        // Synchronize with Spotify server time for exact clock alignment
        let mut server_time_secs = now_ms / 1000;
        if let Ok(time_res) = self.client.get("https://open.spotify.com/api/server-time").send().await {
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
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Spotify token exchange error ({}): {}", status, body));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Spotify token JSON: {}", e))?;

        let token = json["accessToken"]
            .as_str()
            .ok_or_else(|| "Missing accessToken in Spotify response".to_string())?
            .to_string();

        let mut write_guard = self.access_token.write().await;
        *write_guard = Some(token.clone());
        Ok(token)
    }

    /// Executes a GraphQL query against Spotify's internal Pathfinder Partner API.
    pub async fn pathfinder_query(
        &self,
        operation: &str,
        hash: &str,
        variables: Value,
    ) -> Result<Value, String> {
        let token = self.get_access_token().await?;
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
            HeaderValue::from_str(&format!("Bearer {}", token))
                .map_err(|e| format!("Invalid auth header: {}", e))?,
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

        let res = self
            .client
            .post("https://api-partner.spotify.com/pathfinder/v1/query")
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Spotify Pathfinder request ({}) failed: {}", operation, e))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!(
                "Spotify Pathfinder error ({}) status {}: {}",
                operation, status, body
            ));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Spotify Pathfinder JSON ({}): {}", operation, e))?;

        if let Some(errors) = json.get("errors") {
            if !errors.is_null() {
                if let Some(err_arr) = errors.as_array() {
                    if !err_arr.is_empty() {
                        let msg = err_arr[0]["message"].as_str().unwrap_or("GraphQL error");
                        return Err(format!("Spotify Pathfinder error ({}): {}", operation, msg));
                    }
                }
            }
        }

        Ok(json)
    }

    /// Normalizes track entities returned by Pathfinder queries (fetchPlaylist, fetchLibraryTracks, searchDesktop).
    pub fn parse_pathfinder_track(data: &Value, fallback_uri: Option<&str>) -> Option<SourceTrack> {
        let uri = data["uri"]
            .as_str()
            .or_else(|| fallback_uri)
            .unwrap_or("");

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
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        let mut playlists = Vec::new();
        let mut offset = 0;
        let limit = 50;

        loop {
            let variables = serde_json::json!({
                "filters": ["Playlists"],
                "order": serde_json::Value::Null,
                "textFilter": "",
                "features": ["LIKED_SONGS", "YOUR_EPISODES"],
                "limit": limit,
                "offset": offset
            });

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

                // Skip folders and non-playlist containers
                if typename.contains("Folder") {
                    continue;
                }

                if typename.contains("PseudoPlaylist") {
                    let data = &item_wrapper["data"];
                    let name = data["name"].as_str().unwrap_or("Liked Songs").to_string();
                    if name.to_lowercase().contains("episodes") {
                        continue;
                    }
                    let cover_url = data["images"]["items"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|img| img["sources"].as_array())
                        .and_then(|srcs| srcs.first())
                        .and_then(|src| src["url"].as_str())
                        .map(|s| s.to_string());

                    playlists.push(Playlist {
                        id: "collection:tracks".to_string(),
                        service: "spotify".to_string(),
                        title: name,
                        description: Some("Your Spotify Liked Songs".to_string()),
                        track_count: 0,
                        is_public: false,
                        cover_url,
                    });
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

                    if id.is_empty() {
                        continue;
                    }

                    let title = data["name"].as_str().unwrap_or("Untitled").to_string();
                    let description = data["description"].as_str().map(|s| s.to_string());
                    let track_count = data["trackCount"].as_u64().unwrap_or(0) as u32;

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
                        track_count,
                        is_public: false,
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

        Ok(playlists)
    }

    async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<Vec<SourceTrack>, String> {
        let mut all_tracks = Vec::new();
        let mut offset = 0;

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
                    .pathfinder_query(
                        "fetchLibraryTracks",
                        "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240",
                        variables,
                    )
                    .await?;

                let tracks_node = &json["data"]["me"]["library"]["tracks"];
                let items = match tracks_node["items"].as_array() {
                    Some(arr) => arr,
                    None => break,
                };

                if items.is_empty() {
                    break;
                }

                let mut chunk = Vec::new();
                for item in items {
                    let track_data = &item["track"]["data"];
                    let fallback_uri = item["track"]["_uri"].as_str();
                    if let Some(track) = Self::parse_pathfinder_track(track_data, fallback_uri) {
                        chunk.push(track);
                    }
                }

                if let Some(ref sender) = tx {
                    let _ = sender.send(chunk.clone()).await;
                }

                all_tracks.extend(chunk);

                let total = tracks_node["totalCount"].as_u64().unwrap_or(0) as usize;
                offset += items.len();
                if offset >= total || items.len() < limit {
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
                    .pathfinder_query(
                        "fetchPlaylist",
                        "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77",
                        variables,
                    )
                    .await?;

                let content = &json["data"]["playlistV2"]["content"];
                let items = match content["items"].as_array() {
                    Some(arr) => arr,
                    None => break,
                };

                if items.is_empty() {
                    break;
                }

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
                        if let Some(track) =
                            Self::parse_pathfinder_track(item_v2_data, fallback_uri)
                        {
                            chunk.push(track);
                        }
                    }
                }

                if let Some(ref sender) = tx {
                    let _ = sender.send(chunk.clone()).await;
                }

                all_tracks.extend(chunk);

                let total = content["totalCount"].as_u64().unwrap_or(0) as usize;
                offset += items.len();
                if offset >= total || items.len() < limit {
                    break;
                }
            }
        }

        Ok(all_tracks)
    }

    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String> {
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

        let json = self
            .pathfinder_query(
                "searchDesktop",
                "d9f785900f0710b31c07818d617f4f7600c1e21217e80f5b043d1e78d74e6026",
                variables,
            )
            .await?;

        let mut results = Vec::new();
        if let Some(items) = json["data"]["searchV2"]["tracksV2"]["items"].as_array() {
            for item in items {
                let track_data = &item["item"]["data"];
                let fallback_uri = track_data["uri"].as_str();
                if let Some(track) = Self::parse_pathfinder_track(track_data, fallback_uri) {
                    results.push(track);
                }
            }
        }

        Ok(results)
    }

    async fn create_playlist(&self, title: &str, description: Option<&str>) -> Result<String, String> {
        let token = self.get_access_token().await?;

        // Retrieve current user profile ID
        let me_res = self
            .client
            .get("https://api.spotify.com/v1/me")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Spotify profile: {}", e))?;

        let me_json: Value = me_res
            .json()
            .await
            .map_err(|e| format!("Failed to parse profile: {}", e))?;

        let user_id = me_json["id"]
            .as_str()
            .ok_or_else(|| "Failed to extract Spotify user id".to_string())?;

        let create_url = format!("https://api.spotify.com/v1/users/{}/playlists", user_id);
        let body = serde_json::json!({
            "name": title,
            "description": description.unwrap_or(""),
            "public": false
        });

        let res = self
            .client
            .post(&create_url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to create Spotify playlist: {}", e))?;

        let playlist_json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse created playlist: {}", e))?;

        let id = playlist_json["id"]
            .as_str()
            .ok_or_else(|| "Failed to retrieve created playlist ID".to_string())?;

        Ok(id.to_string())
    }

    async fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let token = self.get_access_token().await?;
        let url = format!("https://api.spotify.com/v1/playlists/{}/tracks", playlist_id);

        for chunk in track_ids.chunks(100) {
            let uris: Vec<String> = chunk
                .iter()
                .map(|id| {
                    if id.starts_with("spotify:track:") {
                        id.clone()
                    } else {
                        format!("spotify:track:{}", id)
                    }
                })
                .collect();

            let body = serde_json::json!({ "uris": uris });

            let res = self
                .client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Failed to add tracks to Spotify playlist: {}", e))?;

            if !res.status().is_success() {
                return Err(format!("Add tracks failed with status: {}", res.status()));
            }
        }

        Ok(())
    }

    async fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let token = self.get_access_token().await?;
        let url = format!("https://api.spotify.com/v1/playlists/{}/tracks", playlist_id);

        for chunk in track_ids.chunks(100) {
            let tracks: Vec<Value> = chunk
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

            let body = serde_json::json!({ "tracks": tracks });

            let res = self
                .client
                .delete(&url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Spotify remove_tracks failed: {}", e))?;

            if !res.status().is_success() {
                let status = res.status();
                let text = res.text().await.unwrap_or_default();
                return Err(format!("Spotify remove_tracks failed ({}): {}", status, text));
            }
        }

        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://api.spotify.com/v1/playlists/{}/followers",
            playlist_id
        );

        let res = self
            .client
            .delete(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Spotify delete_playlist failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(format!("Spotify delete_playlist failed ({}): {}", status, text));
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
    inner.update(&k_ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha1::new();
    outer.update(&k_opad);
    outer.update(&inner_hash);
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
        assert_eq!(track.thumbnail_url, Some("https://image-cdn.spotifycdn.com/cover.jpg".to_string()));
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

        let track = SpotifyProvider::parse_pathfinder_track(&json, Some("spotify:track:1AvLUHxSunGMWWRfEFmWSC"))
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
    #[ignore]
    async fn test_live_spotify_diag() {
        let app_data = std::env::var("APPDATA").unwrap_or_default();
        let db_path = format!("{}\\com.soundshift.app\\soundshift.db", app_data);
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            if let Ok(Some(sp_dc)) = crate::auth::keyring_store::retrieve_credential("spotify", &conn) {
                let provider = SpotifyProvider::new(sp_dc);
                let playlists = provider.list_playlists().await.expect("Live playlists should fetch successfully");
                assert!(!playlists.is_empty(), "Should find playlists in live user account");
                if let Some(first) = playlists.first() {
                    let tracks = provider.get_playlist_tracks(&first.id, None).await.expect("Tracks should fetch successfully");
                    assert!(!tracks.is_empty(), "First playlist should have tracks");
                }
            }
        }
    }
}
