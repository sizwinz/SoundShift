use crate::models::{Playlist, SourceTrack};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, COOKIE, USER_AGENT};
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

    fn parse_spotify_track(track_obj: &Value) -> Option<SourceTrack> {
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
        let token = self.get_access_token().await?;
        let mut playlists = Vec::new();
        let mut offset = 0;
        let limit = 50;

        loop {
            let url = format!(
                "https://api.spotify.com/v1/me/playlists?limit={}&offset={}",
                limit, offset
            );

            let res = self
                .client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
                .map_err(|e| format!("Spotify list_playlists request failed: {}", e))?;

            if !res.status().is_success() {
                let status = res.status();
                let text = res.text().await.unwrap_or_default();
                return Err(format!("Spotify API error ({}): {}", status, text));
            }

            let json: Value = res
                .json()
                .await
                .map_err(|e| format!("Failed to parse Spotify playlists: {}", e))?;

            let items = match json["items"].as_array() {
                Some(arr) => arr,
                None => break,
            };

            if items.is_empty() {
                break;
            }

            for item in items {
                let id = match item["id"].as_str() {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let title = item["name"].as_str().unwrap_or("Untitled").to_string();
                let description = item["description"].as_str().map(|s| s.to_string());
                let track_count = item["tracks"]["total"].as_u64().unwrap_or(0) as u32;
                let is_public = item["public"].as_bool().unwrap_or(true);
                let cover_url = item["images"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|img| img["url"].as_str())
                    .map(|s| s.to_string());

                playlists.push(Playlist {
                    id,
                    service: "spotify".to_string(),
                    title,
                    description,
                    track_count,
                    is_public,
                    cover_url,
                });
            }

            let total = json["total"].as_u64().unwrap_or(0) as usize;
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
        let token = self.get_access_token().await?;
        let mut all_tracks = Vec::new();
        let mut offset = 0;
        let limit = 100;

        loop {
            let url = format!(
                "https://api.spotify.com/v1/playlists/{}/tracks?limit={}&offset={}",
                playlist_id, limit, offset
            );

            let res = self
                .client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
                .map_err(|e| format!("Spotify get_playlist_tracks request failed: {}", e))?;

            if !res.status().is_success() {
                let status = res.status();
                let text = res.text().await.unwrap_or_default();
                return Err(format!("Spotify tracks error ({}): {}", status, text));
            }

            let json: Value = res
                .json()
                .await
                .map_err(|e| format!("Failed to parse Spotify tracks: {}", e))?;

            let items = match json["items"].as_array() {
                Some(arr) => arr,
                None => break,
            };

            if items.is_empty() {
                break;
            }

            let mut chunk = Vec::new();

            for item in items {
                let track_obj = &item["track"];
                if track_obj.is_null() {
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
                } else if let Some(track) = Self::parse_spotify_track(track_obj) {
                    chunk.push(track);
                }
            }

            if let Some(ref sender) = tx {
                let _ = sender.send(chunk.clone()).await;
            }

            all_tracks.extend(chunk);

            let total = json["total"].as_u64().unwrap_or(0) as usize;
            offset += items.len();
            if offset >= total || items.len() < limit {
                break;
            }
        }

        Ok(all_tracks)
    }

    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://api.spotify.com/v1/search?type=track&limit=5&q={}",
            urlencoding::encode(query)
        );

        let res = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Spotify search request failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Spotify search failed with status: {}", res.status()));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Spotify search results: {}", e))?;

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

mod urlencoding {
    pub fn encode(data: &str) -> String {
        url::form_urlencoded::byte_serialize(data.as_bytes()).collect()
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
}
